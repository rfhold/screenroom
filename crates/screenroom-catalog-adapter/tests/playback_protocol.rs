//! Isolated integration through the private adapter's exported API; no provider services.
use screenroom_catalog_adapter::jellyfin::{ApiKey, ConnectionConfig, LiveJellyfin, playback::*};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Default)]
struct IsolatedTestGuard(std::sync::Mutex<Vec<DispatchPurpose>>);
impl DispatchGuard for IsolatedTestGuard {
    fn check<'a>(&'a self, context: DispatchContext<'a>) -> DispatchCheck<'a> {
        Box::pin(async move {
            self.0.lock().unwrap().push(context.purpose);
            Ok(())
        })
    }
}

#[tokio::test]
async fn exported_private_api_negotiates_streams_reports_and_recovers_exact_encoding() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let replies = [
            (
                "200 OK",
                "application/json",
                r#"{"Id":"server","ServerName":"test","Version":"10.10.7"}"#,
            ),
            (
                "200 OK",
                "application/json",
                r#"{"Id":"personal","Policy":{"IsDisabled":false}}"#,
            ),
            (
                "200 OK",
                "application/json",
                r#"{"Id":"item","Type":"Episode","MediaSources":[{"Id":"source","Protocol":"File"}]}"#,
            ),
            (
                "200 OK",
                "application/json",
                r#"{"PlaySessionId":"play","MediaSources":[{"Id":"source","Protocol":"File","Container":"mp4","SupportsDirectPlay":true,"MediaStreams":[{"Type":"Video","Index":0,"Codec":"h264"},{"Type":"Audio","Index":1,"Codec":"aac"}],"DefaultAudioStreamIndex":1}]}"#,
            ),
            ("206 Partial Content", "video/mp4", "abc"),
            ("204 No Content", "application/json", ""),
            (
                "200 OK",
                "application/json",
                r#"{"Id":"server","ServerName":"test","Version":"10.10.7"}"#,
            ),
            ("204 No Content", "application/json", ""),
            ("204 No Content", "application/json", ""),
        ];
        let mut requests = Vec::new();
        for (status, mime, body) in replies {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut bytes = [0; 4096];
                let n = socket.read(&mut bytes).await.unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&bytes[..n]);
                if let Some(end) = request.windows(4).position(|s| s == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                    let length = headers
                        .lines()
                        .find_map(|s| s.strip_prefix("content-length: "))
                        .and_then(|s| s.parse::<usize>().ok())
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            requests.push(String::from_utf8(request).unwrap());
            let range = if status.starts_with("206") {
                "Content-Range: bytes 0-2/100\r\n"
            } else {
                ""
            };
            socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\n{range}Connection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        }
        requests
    });
    let device = GatewayDeviceId::generate().unwrap();
    let intent = AuthorizationIntent::new(
        device.clone(),
        "server".into(),
        "personal".into(),
        "indexing",
    )
    .unwrap();
    let guard = std::sync::Arc::new(IsolatedTestGuard::default());
    let adapter = LiveJellyfin::from_api_key(
        ConnectionConfig {
            source_key: "isolated".into(),
            base_url: format!("http://{address}/deployment/"),
            timeout_ms: 1000,
            max_response_bytes: 1024 * 1024,
        },
        ApiKey::new("integration-admin-secret".into()).unwrap(),
    )
    .unwrap()
    .playback_adapter_fenced(intent.clone(), PlaybackTimeouts::default(), guard.clone())
    .unwrap();
    let auth = adapter
        .restore_session(
            intent,
            SessionToken::from_private_store("integration-user-secret".into()).unwrap(),
        )
        .await
        .unwrap();
    let playback = adapter
        .negotiate(
            &auth,
            NegotiationRequest {
                item_id: "item",
                capabilities: ClientCapabilities {
                    mp4: true,
                    h264: true,
                    aac: true,
                    ..Default::default()
                },
                position: Milliseconds::ZERO,
                selection: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(playback.method, PlaybackMethod::DirectPlay);
    let mut stream = adapter
        .stream(
            &auth,
            &playback.resource,
            MediaRequest {
                range: Some(ByteRange::parse("bytes=0-2").unwrap()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(stream.status, 206);
    assert_eq!(stream.next_chunk().await.unwrap().unwrap(), b"abc");
    drop(stream);
    adapter
        .report(
            &auth,
            &playback,
            PlaybackReport {
                kind: ReportKind::Start,
                position: Milliseconds::ZERO,
                paused: false,
                muted: false,
                volume: 1.0,
            },
        )
        .await
        .unwrap();
    let encoding =
        EncodingIdentity::from_recorded(device.clone(), playback.private_play_session_id().into())
            .unwrap();
    adapter
        .stop_recorded_encoding(&auth, &encoding)
        .await
        .unwrap();
    adapter.logout(&auth).await.unwrap();
    let requests = task.await.unwrap();
    assert_eq!(
        *guard.0.lock().unwrap(),
        [
            DispatchPurpose::Active,
            DispatchPurpose::Active,
            DispatchPurpose::Active,
            DispatchPurpose::Active,
            DispatchPurpose::Active,
            DispatchPurpose::Active,
            DispatchPurpose::Cleanup,
            DispatchPurpose::Cleanup,
            DispatchPurpose::Cleanup,
        ]
    );
    assert_eq!(requests.len(), guard.0.lock().unwrap().len());
    assert!(requests[4].starts_with("GET /deployment/Videos/item/stream.mp4?"));
    assert!(requests[4].contains("integration-user-secret"));
    assert!(!requests[4].contains("integration-admin-secret"));
    assert!(requests[5].starts_with("POST /deployment/Sessions/Playing "));
    assert!(requests[7].starts_with(&format!(
        "DELETE /deployment/Videos/ActiveEncodings?deviceId={}&playSessionId=play ",
        device.as_private_str()
    )));
}
