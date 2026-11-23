use super::*;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const SERVER: &str = r#"{"Id":"server","ServerName":"isolated","Version":"10.10.7"}"#;
const PROFILES: &str =
    r#"[{"Id":"personal","Name":"Test","Policy":{"IsDisabled":false,"IsHidden":false}}]"#;
const ME: &str = r#"{"Id":"personal","Policy":{"IsDisabled":false}}"#;
const ITEM: &str = r#"{"Id":"item","Type":"Movie","MediaSources":[{"Id":"source","Protocol":"File"}],"UserData":{"PlaybackPositionTicks":1234567890}}"#;
const PENDING: &str = r#"{"Secret":"test-owned-qc-secret","Code":"123456"}"#;
const AUTH: &str =
    r#"{"AccessToken":"test-owned-user-token","ServerId":"server","User":{"Id":"personal"}}"#;

struct AllowTestDispatch;
impl DispatchGuard for AllowTestDispatch {
    fn check<'a>(&'a self, _: DispatchContext<'a>) -> DispatchCheck<'a> {
        Box::pin(async { Ok(()) })
    }
}
fn test_adapter(admin: &LiveJellyfin, timeout: u64) -> PlaybackAdapter {
    let timeout = Duration::from_millis(timeout);
    admin
        .playback_adapter_fenced(
            intent(),
            PlaybackTimeouts {
                control: timeout,
                headers: timeout,
                idle: timeout,
            },
            Arc::new(AllowTestDispatch),
        )
        .unwrap()
}

struct SwitchGuard {
    calls: std::sync::atomic::AtomicUsize,
    revoked: std::sync::atomic::AtomicBool,
    deny_cleanup: std::sync::atomic::AtomicBool,
    pause_at: usize,
    arrived: tokio::sync::Notify,
    resume: tokio::sync::Notify,
    records: Mutex<Vec<(DispatchPurpose, bool, Option<String>)>>,
}
impl SwitchGuard {
    fn new(pause_at: usize) -> Arc<Self> {
        Arc::new(Self {
            calls: 0.into(),
            revoked: false.into(),
            deny_cleanup: false.into(),
            pause_at,
            arrived: tokio::sync::Notify::new(),
            resume: tokio::sync::Notify::new(),
            records: Mutex::new(Vec::new()),
        })
    }
    fn revoke(&self) {
        self.revoked
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}
impl DispatchGuard for SwitchGuard {
    fn check<'a>(&'a self, context: DispatchContext<'a>) -> DispatchCheck<'a> {
        Box::pin(async move {
            use std::sync::atomic::Ordering::SeqCst;
            assert_eq!(context.intent, &intent());
            self.records.lock().unwrap().push((
                context.purpose,
                context.mutation,
                context.play_session_id.map(str::to_owned),
            ));
            if self.calls.fetch_add(1, SeqCst) + 1 == self.pause_at {
                self.arrived.notify_one();
                self.resume.notified().await;
            }
            let denied = match context.purpose {
                DispatchPurpose::Active => self.revoked.load(SeqCst),
                DispatchPurpose::Cleanup => self.deny_cleanup.load(SeqCst),
            };
            if denied {
                Err(DispatchDenial::Revoked)
            } else {
                Ok(())
            }
        })
    }
}

#[tokio::test]
async fn fence_rechecks_after_discovery_before_mutation_without_followup_io() {
    let (mut p, requests, server) = provider(vec![reply(ITEM), reply("{}")], 1000).await;
    let guard = SwitchGuard::new(2);
    p.guard = guard.clone();
    let operation = tokio::spawn(async move {
        p.negotiate(
            &auth(),
            NegotiationRequest {
                item_id: "item",
                capabilities: caps(),
                position: Milliseconds::ZERO,
                selection: None,
            },
        )
        .await
    });
    guard.arrived.notified().await;
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert!(requests.lock().unwrap()[0].starts_with("GET "));
    guard.revoke();
    guard.resume.notify_one();
    let error = operation.await.unwrap().unwrap_err();
    assert_eq!(error.failure, Failure::FenceDenied);
    assert_eq!(error.effect, Effect::NotDispatched);
    assert_eq!(error.prior_effect, None);
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert_eq!(
        guard
            .records
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.1)
            .collect::<Vec<_>>(),
        [false, true]
    );
    server.abort();
}

#[tokio::test]
async fn every_quick_connect_read_and_mutation_is_fenced_individually() {
    for pause_at in 1..=4 {
        let (mut p, requests, server) = provider(
            vec![
                reply(SERVER),
                reply(PROFILES),
                reply("true"),
                reply(PENDING),
            ],
            1000,
        )
        .await;
        let guard = SwitchGuard::new(pause_at);
        p.guard = guard.clone();
        let operation = tokio::spawn(async move { p.initiate(&intent()).await });
        guard.arrived.notified().await;
        assert_eq!(requests.lock().unwrap().len(), pause_at - 1);
        guard.revoke();
        guard.resume.notify_one();
        let error = operation.await.unwrap().unwrap_err();
        assert_eq!(error.failure, Failure::FenceDenied);
        assert_eq!(error.effect, Effect::NotDispatched);
        assert_eq!(requests.lock().unwrap().len(), pause_at - 1);
        server.abort();
    }
    let (mut p, requests, server) =
        provider(vec![reply(SERVER), reply(PROFILES), reply("true")], 1000).await;
    let guard = SwitchGuard::new(3);
    p.guard = guard.clone();
    let operation = tokio::spawn(async move {
        p.authorize(PendingQuickConnect {
            intent: intent(),
            secret: SessionToken::from_private_store("pending-secret".into()).unwrap(),
            code: "123456".into(),
        })
        .await
    });
    guard.arrived.notified().await;
    guard.revoke();
    guard.resume.notify_one();
    assert_eq!(
        operation.await.unwrap().unwrap_err().effect,
        Effect::NotDispatched
    );
    assert_eq!(requests.lock().unwrap().len(), 2);
    server.abort();
}

#[tokio::test]
async fn exchange_followup_fence_denial_retains_prior_remote_authentication_effect() {
    let (mut p, requests, server) = provider(vec![reply(AUTH), reply(SERVER)], 1000).await;
    let guard = SwitchGuard::new(2);
    p.guard = guard.clone();
    let operation = tokio::spawn(async move {
        p.exchange(AuthorizedQuickConnect(PendingQuickConnect {
            intent: intent(),
            secret: SessionToken::from_private_store("pending-secret".into()).unwrap(),
            code: "123456".into(),
        }))
        .await
    });
    guard.arrived.notified().await;
    assert!(
        requests.lock().unwrap()[0]
            .starts_with("POST /jellyfin/Users/AuthenticateWithQuickConnect ")
    );
    guard.revoke();
    guard.resume.notify_one();
    let error = operation.await.unwrap().unwrap_err();
    assert_eq!(error.failure, Failure::FenceDenied);
    assert_eq!(error.effect, Effect::NotDispatched);
    assert_eq!(error.prior_effect, Some(Effect::Uncertain));
    assert_eq!(requests.lock().unwrap().len(), 1);
    server.abort();
}

#[tokio::test]
async fn disabled_profile_cleanup_uses_independent_exact_record_fence() {
    let (mut p, requests, server) = provider(
        vec![
            reply(SERVER),
            reply(r#"{"Id":"personal","Policy":{"IsDisabled":true}}"#),
            reply(SERVER),
            reply(""),
            reply(""),
            reply(SERVER),
            reply(""),
        ],
        1000,
    )
    .await;
    let guard = SwitchGuard::new(0);
    guard.revoke();
    p.guard = guard.clone();
    let auth = p
        .restore_session_for_cleanup(
            intent(),
            SessionToken::from_private_store("user-token".into()).unwrap(),
        )
        .await
        .unwrap();
    let encoding = EncodingIdentity::from_recorded(intent().device.clone(), "play".into()).unwrap();
    p.stop_recorded_encoding(&auth, &encoding).await.unwrap();
    p.logout(&auth).await.unwrap();
    p.recover_device(&intent()).await.unwrap();
    server.await.unwrap();
    assert_eq!(requests.lock().unwrap().len(), 7);
    {
        let records = guard.records.lock().unwrap();
        assert!(records.iter().all(|r| r.0 == DispatchPurpose::Cleanup));
        assert_eq!(records[3].2.as_deref(), Some("play"));
    }
    let (mut denied, requests, server) = provider(vec![reply(SERVER)], 1000).await;
    guard
        .deny_cleanup
        .store(true, std::sync::atomic::Ordering::SeqCst);
    denied.guard = guard;
    let error = denied.recover_device(&intent()).await.unwrap_err();
    assert_eq!(error.failure, Failure::FenceDenied);
    assert_eq!(error.effect, Effect::NotDispatched);
    assert!(requests.lock().unwrap().is_empty());
    server.abort();
}

#[tokio::test]
async fn denied_cleanup_after_server_check_never_deletes_device() {
    let (mut p, requests, server) = provider(vec![reply(SERVER), reply("")], 1000).await;
    let guard = SwitchGuard::new(2);
    p.guard = guard.clone();
    let operation = tokio::spawn(async move { p.recover_device(&intent()).await });
    guard.arrived.notified().await;
    guard
        .deny_cleanup
        .store(true, std::sync::atomic::Ordering::SeqCst);
    guard.resume.notify_one();
    let error = operation.await.unwrap().unwrap_err();
    assert_eq!(error.effect, Effect::NotDispatched);
    assert_eq!(error.prior_effect, None);
    assert_eq!(requests.lock().unwrap().len(), 1);
    server.abort();
}

#[tokio::test]
async fn revoked_stream_and_report_have_no_provider_dispatch_and_guards_are_session_local() {
    let (mut p, requests, server) = provider(vec![reply("")], 1000).await;
    let guard = SwitchGuard::new(0);
    guard.revoke();
    let unfenced_clone_for_test = p.clone();
    p.guard = guard;
    let error = p
        .stream(
            &auth(),
            &resource(&p, "Videos/item/stream.mp4"),
            MediaRequest::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.failure, Failure::FenceDenied);
    assert!(requests.lock().unwrap().is_empty());
    let error = p.resume_position(&auth(), "item").await.unwrap_err();
    assert_eq!(error.failure, Failure::FenceDenied);
    for kind in [ReportKind::Start, ReportKind::Progress, ReportKind::Stopped] {
        let error = p
            .report(
                &auth(),
                &transport(&p),
                PlaybackReport {
                    kind,
                    position: Milliseconds::ZERO,
                    paused: false,
                    muted: false,
                    volume: 1.0,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(error.failure, Failure::FenceDenied);
        assert_eq!(error.effect, Effect::NotDispatched);
    }
    let error = p.ping_encoding(&auth(), &transport(&p)).await.unwrap_err();
    assert_eq!(error.failure, Failure::FenceDenied);
    assert!(requests.lock().unwrap().is_empty());
    // Replacing one test instance's guard does not mutate another session's immutable adapter.
    unfenced_clone_for_test.logout(&auth()).await.unwrap();
    server.await.unwrap();
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn playback_deadlines_are_explicit_and_do_not_inherit_metadata_timeout() {
    let (p, _, server) = provider(Vec::new(), 3000).await;
    server.await.unwrap();
    let defaults = p
        .admin
        .playback_adapter_fenced(
            intent(),
            PlaybackTimeouts::default(),
            Arc::new(AllowTestDispatch),
        )
        .unwrap();
    assert_eq!(defaults.admin.config.timeout_ms, 3000);
    assert_eq!(defaults.control_timeout, Duration::from_secs(10));
    assert_eq!(defaults.header_timeout, Duration::from_secs(10));
    assert_eq!(defaults.idle_timeout, Duration::from_secs(10));
    let explicit = PlaybackTimeouts {
        control: Duration::from_millis(450),
        headers: Duration::from_millis(200),
        idle: Duration::from_millis(300),
    };
    let independent = p
        .admin
        .playback_adapter_fenced(intent(), explicit, Arc::new(AllowTestDispatch))
        .unwrap();
    assert_eq!(independent.control_timeout, explicit.control);
    assert_eq!(independent.header_timeout, explicit.headers);
    assert_eq!(independent.idle_timeout, explicit.idle);
    for invalid in [
        Duration::ZERO,
        Duration::from_nanos(1),
        Duration::from_millis(10001),
    ] {
        assert!(
            p.admin
                .playback_adapter_fenced(
                    intent(),
                    PlaybackTimeouts {
                        control: invalid,
                        ..explicit
                    },
                    Arc::new(AllowTestDispatch),
                )
                .is_err()
        );
    }
}
#[tokio::test]
async fn admin_checks_use_playback_client_deadline_not_metadata_whole_request_deadline() {
    let mut delayed = reply(SERVER);
    delayed.delay = 30;
    let (p, requests, server) = provider(
        vec![delayed, reply(PROFILES), reply("true"), reply(PENDING)],
        1,
    )
    .await;
    let playback = p
        .admin
        .playback_adapter_fenced(
            intent(),
            PlaybackTimeouts::default(),
            Arc::new(AllowTestDispatch),
        )
        .unwrap();
    playback.initiate(&intent()).await.unwrap();
    server.await.unwrap();
    assert_eq!(requests.lock().unwrap().len(), 4);
    assert_eq!(playback.admin.config.timeout_ms, 1);
}

#[tokio::test]
async fn guard_unavailability_and_deadline_are_sanitized_and_not_dispatched() {
    struct UnavailableGuard(bool);
    impl DispatchGuard for UnavailableGuard {
        fn check<'a>(&'a self, _: DispatchContext<'a>) -> DispatchCheck<'a> {
            Box::pin(async move {
                if self.0 {
                    std::future::pending::<()>().await;
                }
                Err(DispatchDenial::Unavailable)
            })
        }
    }
    let (mut p, requests, server) = provider(vec![reply(SERVER)], 10).await;
    for hang in [false, true] {
        p.guard = Arc::new(UnavailableGuard(hang));
        let error = p.initiate(&intent()).await.unwrap_err();
        assert_eq!(error.failure, Failure::FenceUnavailable);
        assert_eq!(error.effect, Effect::NotDispatched);
        assert_eq!(error.prior_effect, None);
        assert!(requests.lock().unwrap().is_empty());
    }
    server.abort();
}

#[tokio::test]
async fn streaming_header_deadline_is_independent_of_control_deadline() {
    let mut delayed = reply("abc");
    delayed.delay = 50;
    delayed.headers = "Content-Type: video/mp4\r\n".into();
    let (mut p, requests, server) = provider(vec![delayed], 1000).await;
    p.header_timeout = Duration::from_millis(10);
    let error = p
        .stream(
            &auth(),
            &resource(&p, "Videos/item/stream.mp4"),
            MediaRequest::default(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.failure, Failure::Deadline);
    assert_eq!(requests.lock().unwrap().len(), 1);
    server.await.unwrap();
}

struct Reply {
    status: &'static str,
    body: String,
    headers: String,
    delay: u64,
    raw: bool,
}
fn reply(body: impl Into<String>) -> Reply {
    Reply {
        status: "200 OK",
        body: body.into(),
        headers: String::new(),
        delay: 0,
        raw: false,
    }
}
fn chunked_reply(body: &str, mime: &str) -> Reply {
    let mut response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
    );
    for chunk in body.as_bytes().chunks(16 * 1024) {
        response.push_str(&format!("{:x}\r\n", chunk.len()));
        response.push_str(std::str::from_utf8(chunk).unwrap());
        response.push_str("\r\n");
    }
    response.push_str("0\r\n\r\n");
    Reply {
        raw: true,
        ..reply(response)
    }
}
async fn read_request(socket: &mut tokio::net::TcpStream) -> String {
    let mut bytes = Vec::new();
    loop {
        let mut buf = [0; 4096];
        let n = socket.read(&mut buf).await.unwrap();
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..n]);
        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
            let length = headers
                .lines()
                .find_map(|s| s.strip_prefix("content-length: "))
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(0);
            if bytes.len() >= end + 4 + length {
                break;
            }
        }
    }
    String::from_utf8(bytes).unwrap()
}
async fn provider(
    replies: Vec<Reply>,
    timeout: u64,
) -> (
    PlaybackAdapter,
    Arc<Mutex<Vec<String>>>,
    tokio::task::JoinHandle<()>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let records = requests.clone();
    let task = tokio::spawn(async move {
        for reply in replies {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            records.lock().unwrap().push(request);
            tokio::time::sleep(Duration::from_millis(reply.delay)).await;
            let response = if reply.raw {
                reply.body
            } else {
                format!(
                    "HTTP/1.1 {}\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n{}",
                    reply.status,
                    reply.body.len(),
                    reply.headers,
                    reply.body
                )
            };
            let _ = socket.write_all(response.as_bytes()).await;
        }
    });
    let admin = LiveJellyfin::from_api_key(
        super::super::ConnectionConfig {
            source_key: "test".into(),
            base_url: format!("http://{address}/jellyfin/"),
            timeout_ms: timeout,
            max_response_bytes: 1024 * 1024,
        },
        ApiKey::new("test-owned-admin-key".into()).unwrap(),
    )
    .unwrap();
    (test_adapter(&admin, timeout), requests, task)
}
fn intent() -> AuthorizationIntent {
    AuthorizationIntent::new(
        GatewayDeviceId::from_recorded(format!("screenroom-playback-{}", "a".repeat(64))).unwrap(),
        "server".into(),
        "personal".into(),
        "indexing",
    )
    .unwrap()
}
fn auth() -> SessionAuth {
    SessionAuth {
        intent: intent(),
        token: SessionToken::from_private_store("test-owned-user-token".into()).unwrap(),
    }
}
fn caps() -> ClientCapabilities {
    ClientCapabilities {
        mp4: true,
        h264: true,
        aac: true,
        mse_hls: true,
        webvtt: true,
        ..Default::default()
    }
}
fn scope() -> ResourceScope {
    let i = intent();
    ResourceScope {
        device: i.device,
        server: i.server,
        profile: i.personal_profile,
        item: "item".into(),
        source: "source".into(),
        play_session: "play".into(),
    }
}
fn resource(adapter: &PlaybackAdapter, path: &str) -> ResourceRef {
    adapter
        .resource(&scope(), &adapter.base_directory().unwrap(), path, 0)
        .unwrap()
}

#[tokio::test]
async fn private_resource_identity_covers_all_fields_and_equal_hashes() {
    use std::hash::{Hash, Hasher};
    let (adapter, _, task) = provider(vec![], 1000).await;
    let original = resource(&adapter, "Videos/item/main.m3u8");
    let hash = |value: &ResourceRef| {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        value.hash(&mut hasher);
        hasher.finish()
    };
    assert_eq!(original, original.clone());
    assert_eq!(hash(&original), hash(&original.clone()));
    let mut variants = Vec::new();
    let mut changed = original.clone();
    changed.url.set_path("/Videos/item/other.m3u8");
    variants.push(changed);
    let mut changed = original.clone();
    changed.depth += 1;
    variants.push(changed);
    let mut changed = original.clone();
    changed.kind = ResourceKind::Segment;
    variants.push(changed);
    let mut changed = original.clone();
    changed.scope.device = GatewayDeviceId::generate().unwrap();
    variants.push(changed);
    for field in ["server", "profile", "item", "source", "play_session"] {
        let mut changed = original.clone();
        match field {
            "server" => changed.scope.server.push('x'),
            "profile" => changed.scope.profile.push('x'),
            "item" => changed.scope.item.push('x'),
            "source" => changed.scope.source.push('x'),
            "play_session" => changed.scope.play_session.push('x'),
            _ => unreachable!(),
        }
        variants.push(changed);
    }
    let mut set = std::collections::HashSet::new();
    set.insert(original.clone());
    for changed in variants {
        assert_ne!(original, changed);
        assert!(set.insert(changed));
    }
    task.await.unwrap();
}

#[tokio::test]
async fn cloned_playlist_budget_stages_independently_even_after_mapper_failure() {
    let (adapter, _, task) = provider(vec![], 1000).await;
    let parent = resource(&adapter, "Videos/item/main.m3u8");
    let mut original = PlaylistBudget::default();
    let body = b"#EXTM3U\n#EXTINF:3,\nfirst.ts\n#EXTINF:3,\nsecond.ts\n#EXT-X-ENDLIST\n";
    let mut staged = original.clone();
    assert!(
        adapter
            .rewrite_playlist(&auth(), &parent, body, &mut staged, |_| {
                Err(fail(Failure::Bounds))
            })
            .is_err()
    );
    assert_eq!(original.resources(), 0);
    assert_eq!(staged.resources(), 1);
    let mut successful = original.clone();
    adapter
        .rewrite_playlist(&auth(), &parent, body, &mut successful, |_| {
            Ok("/playback/v1/media/opaque".into())
        })
        .unwrap();
    assert_eq!(successful.resources(), 2);
    assert_eq!(original.resources(), 0);
    adapter
        .rewrite_playlist(&auth(), &parent, body, &mut original, |_| {
            Ok("/playback/v1/media/opaque".into())
        })
        .unwrap();
    assert_eq!(original.resources(), 2);
    task.await.unwrap();
}
fn transport(adapter: &PlaybackAdapter) -> NegotiatedPlayback {
    NegotiatedPlayback {
        method: PlaybackMethod::Transcode,
        mime_type: "application/vnd.apple.mpegurl",
        duration: None,
        start_position: Milliseconds::ZERO,
        timeline_offset: Milliseconds::ZERO,
        resource: resource(adapter, "Videos/item/master.m3u8"),
        tracks: vec![],
        source_display_name: "Media".into(),
        scope: scope(),
        audio_index: Some(1),
        subtitle_index: None,
        default_audio_index: Some(1),
    }
}
fn negotiation(source: Value) -> String {
    json!({"PlaySessionId":"play","MediaSources":[source]}).to_string()
}
fn source() -> Value {
    json!({"Id":"source","Protocol":"File","Container":"mp4","RunTimeTicks":10000000000i64,
        "SupportsDirectPlay":true,"SupportsDirectStream":true,"SupportsTranscoding":true,
        "TranscodingUrl":"Videos/item/master.m3u8?MediaSourceId=source&PlaySessionId=play&api_key=discard",
        "DefaultAudioStreamIndex":1,
        "MediaStreams":[{"Type":"Video","Index":0,"Codec":"h264"},{"Type":"Audio","Index":1,"Codec":"aac"},
        {"Type":"Audio","Index":2,"Codec":"aac"},{"Type":"Subtitle","Index":3,"Codec":"srt","SupportsExternalStream":true},
        {"Type":"Subtitle","Index":4,"Codec":"ass"},{"Type":"Subtitle","Index":5,"Codec":"pgssub"}]})
}

#[tokio::test]
async fn quick_connect_phases_personal_attribution_and_redaction() {
    let (p, requests, task) = provider(
        vec![
            reply(SERVER),
            reply(PROFILES),
            reply("true"),
            reply(PENDING),
            reply(SERVER),
            reply(PROFILES),
            reply("true"),
            reply(AUTH),
            reply(SERVER),
            reply(ME),
        ],
        1000,
    )
    .await;
    let i = intent();
    let pending = p.initiate(&i).await.unwrap();
    assert_eq!(pending.secret_for_private_store(), "test-owned-qc-secret");
    assert!(!format!("{pending:?}").contains("123456"));
    let authorized = p.authorize(pending).await.unwrap();
    let session = p.exchange(authorized).await.unwrap();
    assert_eq!(
        session.token().expose_for_private_store(),
        "test-owned-user-token"
    );
    for debug in [
        format!("{session:?}"),
        format!("{p:?}"),
        format!("{:?}", session.token),
    ] {
        assert!(!debug.contains("test-owned"));
        assert!(!debug.contains("http://"));
    }
    task.await.unwrap();
    let requests = requests.lock().unwrap();
    assert!(requests[3].starts_with("POST /jellyfin/QuickConnect/Initiate "));
    assert!(requests[3].contains(i.device.as_private_str()));
    assert!(!requests[3].contains("test-owned-admin-key"));
    assert!(
        requests[6]
            .starts_with("POST /jellyfin/QuickConnect/Authorize?code=123456&userId=personal ")
    );
    assert!(requests[6].contains("test-owned-admin-key"));
    assert!(requests[7].contains("test-owned-qc-secret"));
    assert!(requests[9].contains("test-owned-user-token"));
    assert!(!requests[9].contains("test-owned-admin-key"));
}
#[tokio::test]
async fn disabled_wrong_identity_and_no_indexing_fallback() {
    let (p, requests, task) =
        provider(vec![reply(SERVER), reply(PROFILES), reply("false")], 1000).await;
    assert_eq!(
        p.initiate(&intent()).await.unwrap_err().failure,
        Failure::AuthorizationUnavailable
    );
    task.await.unwrap();
    assert_eq!(requests.lock().unwrap().len(), 3);
    for personal_profile in ["", "../personal", "invalid profile"] {
        assert!(
            AuthorizationIntent::new(
                intent().device,
                "server".into(),
                personal_profile.into(),
                "indexing"
            )
            .is_err()
        );
    }
    assert!(
        AuthorizationIntent::new(intent().device, "".into(), "personal".into(), "indexing")
            .is_err()
    );
    let (p, requests, task) = provider(vec![reply(SERVER), reply("[]")], 1000).await;
    assert_eq!(
        p.initiate(&intent()).await.unwrap_err().failure,
        Failure::Identity
    );
    task.await.unwrap();
    assert_eq!(requests.lock().unwrap().len(), 2);
    for (bad, phase) in [
        (AUTH.replace("personal", "other"), "user"),
        (AUTH.replace("server", "other"), "server"),
    ] {
        let (p, _, task) = provider(vec![reply(bad)], 1000).await;
        let authorized = AuthorizedQuickConnect(PendingQuickConnect {
            intent: intent(),
            secret: SessionToken::from_private_store("secret".into()).unwrap(),
            code: "123456".into(),
        });
        let error = p.exchange(authorized).await.unwrap_err();
        assert_eq!(error.failure, Failure::Identity, "{phase}");
        assert_eq!(error.effect, Effect::Uncertain);
        task.await.unwrap();
    }
    let (p, requests, task) = provider(vec![reply(SERVER.replace("server", "wrong"))], 1000).await;
    assert_eq!(
        p.initiate(&intent()).await.unwrap_err().failure,
        Failure::Identity
    );
    task.await.unwrap();
    assert_eq!(requests.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn explicitly_validated_personal_profile_may_match_indexing_profile() {
    let (p, requests, task) = provider(
        vec![
            reply(SERVER),
            reply(PROFILES),
            reply("true"),
            reply(PENDING),
            reply(SERVER),
            reply(PROFILES),
            reply("true"),
            reply(AUTH),
            reply(SERVER),
            reply(ME),
        ],
        1000,
    )
    .await;
    let intent = AuthorizationIntent::new(
        intent().device,
        "server".into(),
        "personal".into(),
        "personal",
    )
    .unwrap();
    let pending = p.initiate(&intent).await.unwrap();
    let authorized = p.authorize(pending).await.unwrap();
    let session = p.exchange(authorized).await.unwrap();
    assert_eq!(session.intent().profile_id(), "personal");
    task.await.unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 10);
    assert!(
        requests[6]
            .starts_with("POST /jellyfin/QuickConnect/Authorize?code=123456&userId=personal ")
    );
    assert!(requests[9].contains("test-owned-user-token"));
    assert!(!requests[9].contains("test-owned-admin-key"));
}
#[tokio::test]
async fn lost_authentication_or_authorization_response_is_uncertain_single_attempt() {
    for exchange in [true, false] {
        let lost = Reply {
            raw: true,
            body: "HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{".into(),
            ..reply("")
        };
        let replies = if exchange {
            vec![lost]
        } else {
            vec![reply(SERVER), reply(PROFILES), lost]
        };
        let (p, requests, task) = provider(replies, 1000).await;
        let pending = PendingQuickConnect {
            intent: intent(),
            secret: SessionToken::from_private_store("secret".into()).unwrap(),
            code: "123456".into(),
        };
        let error = if exchange {
            p.exchange(AuthorizedQuickConnect(pending))
                .await
                .unwrap_err()
        } else {
            p.authorize(pending).await.unwrap_err()
        };
        assert_eq!(error.effect, Effect::Uncertain);
        task.await.unwrap();
        assert_eq!(requests.lock().unwrap().len(), if exchange { 1 } else { 3 });
        assert!(!format!("{error:?}").contains("HTTP/1.1"));
    }
}
#[tokio::test]
async fn device_recovery_is_exact_and_server_verified() {
    let (p, requests, task) = provider(vec![reply(SERVER), reply("")], 1000).await;
    p.recover_device(&intent()).await.unwrap();
    task.await.unwrap();
    {
        let requests = requests.lock().unwrap();
        assert!(requests[1].starts_with(&format!(
            "DELETE /jellyfin/Devices?id={} ",
            intent().device.0
        )));
        assert!(!requests[1].contains("userId"));
    }
    let (p, requests, task) = provider(vec![reply(SERVER.replace("server", "wrong"))], 1000).await;
    assert!(p.recover_device(&intent()).await.is_err());
    task.await.unwrap();
    assert_eq!(requests.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn reports_timeout_ambiguity_rejection_and_exact_cleanup() {
    for (reply, expected) in [
        (
            Reply {
                delay: 100,
                ..reply("")
            },
            Effect::Uncertain,
        ),
        (
            Reply {
                status: "503 Unavailable",
                ..reply("private upstream error")
            },
            Effect::Uncertain,
        ),
        (
            Reply {
                status: "403 Forbidden",
                ..reply("private upstream error")
            },
            Effect::Rejected,
        ),
    ] {
        let (p, requests, task) = provider(vec![reply], 30).await;
        let error = p
            .report(
                &auth(),
                &transport(&p),
                PlaybackReport {
                    kind: ReportKind::Start,
                    position: Milliseconds::ZERO,
                    paused: false,
                    muted: false,
                    volume: 0.5,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(error.effect, expected);
        assert!(!format!("{error:?}").contains("private upstream"));
        task.await.unwrap();
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].contains("test-owned-user-token"));
        assert!(!requests[0].contains("test-owned-admin-key"));
        assert!(requests[0].contains("\"PlaySessionId\":\"play\""));
    }
    let (p, requests, task) =
        provider(vec![reply(""), reply(SERVER), reply(""), reply("")], 1000).await;
    let t = transport(&p);
    p.ping_encoding(&auth(), &t).await.unwrap();
    p.stop_encoding(&auth(), &t).await.unwrap();
    p.logout(&auth()).await.unwrap();
    task.await.unwrap();
    let requests = requests.lock().unwrap();
    assert!(requests[0].starts_with("POST /jellyfin/Sessions/Playing/Ping?playSessionId=play "));
    assert!(requests[2].starts_with(&format!(
        "DELETE /jellyfin/Videos/ActiveEncodings?deviceId={}&playSessionId=play ",
        intent().device.0
    )));
    assert!(requests[3].starts_with("POST /jellyfin/Sessions/Logout "));
}
#[test]
fn checked_wide_timeline_and_one_compatible_budget() {
    let max = Milliseconds::parse_decimal("922337203685477").unwrap();
    assert_eq!(max.ticks(), 9223372036854770000);
    for invalid in ["922337203685478", "-1", "1.2", "+1", " 1", ""] {
        assert!(Milliseconds::parse_decimal(invalid).is_err());
    }
    assert!(Milliseconds::from_ticks(-1).is_err());
    assert_eq!(Milliseconds::from_ticks(123456).unwrap().decimal(), "12");
    assert_eq!(
        Milliseconds::parse_decimal("40000")
            .unwrap()
            .provider_position(Milliseconds::parse_decimal("3000").unwrap())
            .unwrap()
            .decimal(),
        "43000"
    );
    assert!(
        max.provider_position(Milliseconds::parse_decimal("1").unwrap())
            .is_err()
    );
    let mut budget = NegotiationBudget::default();
    budget.reserve_compatible().unwrap();
    assert!(budget.compatible_used());
    assert!(budget.reserve_compatible().is_err());
    assert!(
        NegotiationBudget::from_recorded(true)
            .reserve_compatible()
            .is_err()
    );
}
#[tokio::test]
async fn negotiation_prefers_direct_tracks_and_live_resume() {
    let (p, requests, task) = provider(
        vec![reply(ITEM), reply(negotiation(source())), reply(ITEM)],
        1000,
    )
    .await;
    let result = p
        .negotiate(
            &auth(),
            NegotiationRequest {
                item_id: "item",
                capabilities: caps(),
                position: Milliseconds::parse_decimal("100").unwrap(),
                selection: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(result.method, PlaybackMethod::DirectPlay);
    assert_eq!(result.timeline_offset, Milliseconds::ZERO);
    assert_eq!(result.tracks[2].subtitle_mode, Some(SubtitleMode::WebVtt));
    assert!(result.tracks[2].resource.is_some());
    assert_eq!(result.tracks[3].subtitle_mode, Some(SubtitleMode::BurnIn));
    assert_eq!(
        p.resume_position(&auth(), "item").await.unwrap().decimal(),
        "123456"
    );
    let selection = result
        .selection(Some(&result.tracks[1]), Some(&result.tracks[3]))
        .unwrap();
    assert!(selection.burn_in);
    assert_eq!(selection.audio, Some(2));
    task.await.unwrap();
    let requests = requests.lock().unwrap();
    assert!(requests[1].contains("test-owned-user-token"));
    let body: Value = serde_json::from_str(requests[1].split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(
        body["DeviceProfile"]["DirectPlayProfiles"][0]["VideoCodec"],
        "h264"
    );
    assert_eq!(body["UserId"], "personal");
    assert_eq!(body["AutoOpenLiveStream"], false);
}

#[tokio::test]
async fn track_labels_are_safe_distinct_and_preserve_selection() {
    let mut s = source();
    s["TranscodingUrl"] = json!("Videos/item/master.m3u8?VideoCodec=h264");
    s["MediaStreams"] = json!([
        {"Type":"Video","Index":0,"Codec":"h264"},
        {"Type":"Audio","Index":1,"Codec":"aac","Language":"eng","Channels":2},
        {"Type":"Audio","Index":2,"Codec":"ac3","Language":"eng","Channels":6},
        {"Type":"Audio","Index":8,"Codec":"aac","Language":"eng","Channels":6},
        {"Type":"Audio","Index":9,"Codec":"aac","Language":"eng","Channels":2},
        {"Type":"Subtitle","Index":3,"Codec":"srt","Language":"pt-BR","IsForced":true,"SupportsExternalStream":true},
        {"Type":"Subtitle","Index":4,"Codec":"ass","Language":"zh-Hant-TW","IsHearingImpaired":true},
        {"Type":"Subtitle","Index":5,"Codec":"pgssub","Language":"en-x-secret","IsExternalUrl":true},
        {"Type":"Audio","Index":6,"Codec":"private/token","Title":"/private/test-owned-user-token","DisplayTitle":"secret","Channels":1000},
        {"Type":"Audio","Index":7,"Codec":"aac","Title":"Audio Commentary"}
    ]);
    let (p, _, task) = provider(vec![reply(ITEM), reply(negotiation(s))], 1000).await;
    let result = p
        .negotiate(
            &auth(),
            NegotiationRequest {
                item_id: "item",
                capabilities: caps(),
                position: Milliseconds::ZERO,
                selection: None,
            },
        )
        .await
        .unwrap();
    let labels: Vec<_> = result
        .tracks
        .iter()
        .map(|t| t.display_name.as_str())
        .collect();
    assert_eq!(
        labels,
        [
            "English · AAC · 2 channels · Track 1",
            "English · AC-3 · 6 channels",
            "English · AAC · 6 channels",
            "English · AAC · 2 channels · Track 2",
            "pt-br · SRT · Forced",
            "zh-hant-tw · ASS · SDH",
            "Unknown language · PGS",
            "Unknown language · Unknown codec",
            "Unknown language · AAC · Commentary",
        ]
    );
    assert_eq!(result.tracks[4].language.as_deref(), Some("pt-br"));
    assert_eq!(result.tracks[5].language.as_deref(), Some("zh-hant-tw"));
    assert!(result.tracks[6].language.is_none());
    assert!(!result.tracks[6].available);
    assert!(result.selection(None, Some(&result.tracks[6])).is_err());
    assert!(result.tracks[0].selected);
    let selection = result
        .selection(Some(&result.tracks[1]), Some(&result.tracks[5]))
        .unwrap();
    assert_eq!(selection.audio, Some(2));
    assert!(selection.burn_in);
    assert_eq!(result.tracks[4].subtitle_mode, Some(SubtitleMode::WebVtt));
    assert!(
        result.tracks[4]
            .resource
            .as_ref()
            .unwrap()
            .url
            .path()
            .ends_with("/Subtitles/3/Stream.vtt")
    );
    task.await.unwrap();
}

#[tokio::test]
async fn track_language_rejects_unbounded_private_or_malformed_tags() {
    let mut s = source();
    let invalid = [
        "",
        "english",
        "en_US",
        "en--US",
        "en-US-extra",
        "en-x-secret",
        "en-1234",
        "/private/token",
        "éñ",
        "und-private",
    ];
    let streams = s["MediaStreams"].as_array_mut().unwrap();
    streams.truncate(2);
    for (i, language) in invalid.iter().enumerate() {
        streams.push(json!({"Type":"Subtitle","Index":i + 2,"Codec":"srt","Language":language,"Title":"commentary /private/token"}));
    }
    let (p, _, task) = provider(vec![reply(ITEM), reply(negotiation(s))], 1000).await;
    let result = p
        .negotiate(
            &auth(),
            NegotiationRequest {
                item_id: "item",
                capabilities: caps(),
                position: Milliseconds::ZERO,
                selection: None,
            },
        )
        .await
        .unwrap();
    for track in &result.tracks {
        assert!(track.language.is_none());
        assert!(track.display_name.starts_with("Unknown language"));
        assert!(!track.display_name.contains("private"));
        assert!(!track.display_name.contains("Commentary"));
    }
    task.await.unwrap();
}
#[tokio::test]
async fn negotiation_remux_transcode_and_unsupported_are_truthful() {
    for (direct_stream, forced, expected) in [
        (true, false, PlaybackMethod::DirectStream),
        (false, false, PlaybackMethod::Transcode),
        (true, true, PlaybackMethod::Transcode),
    ] {
        let mut s = source();
        s["SupportsDirectPlay"] = json!(false);
        s["SupportsDirectStream"] = json!(direct_stream);
        s["TranscodingUrl"] = json!(format!(
            "/videos/item/master.m3u8?MediaSourceId=source&PlaySessionId=play&VideoCodec={}&api_key=discard",
            if direct_stream && !forced {
                "copy"
            } else {
                "h264"
            }
        ));
        let (p, requests, task) = provider(vec![reply(ITEM), reply(negotiation(s))], 1000).await;
        let request = NegotiationRequest {
            item_id: "item",
            capabilities: caps(),
            position: Milliseconds::parse_decimal("120000").unwrap(),
            selection: None,
        };
        let result = if forced {
            p.compatible_transcode(
                &auth(),
                request,
                NegotiationBudget::default().reserve_compatible().unwrap(),
            )
            .await
            .unwrap()
        } else {
            p.negotiate(&auth(), request).await.unwrap()
        };
        assert_eq!(result.method, expected);
        assert_eq!(result.timeline_offset.decimal(), "0");
        assert_eq!(result.start_position.decimal(), "120000");
        assert!(result.resource.url.path().starts_with("/jellyfin/videos/"));
        assert!(!result.resource.url.as_str().contains("api_key"));
        task.await.unwrap();
        if forced {
            let requests = requests.lock().unwrap();
            assert!(requests[1].contains("\"AllowVideoStreamCopy\":false"));
        }
    }
    for (field, value) in [
        ("Protocol", json!("Http")),
        ("IsEncrypted", json!(true)),
        ("RequiresOpening", json!(true)),
        ("IsInfiniteStream", json!(true)),
        ("LiveStreamId", json!("live")),
        ("VideoType", json!("Dvd")),
        ("IsRemote", json!(true)),
    ] {
        let mut s = source();
        s[field] = value;
        let (p, _, task) = provider(vec![reply(ITEM), reply(negotiation(s))], 1000).await;
        assert_eq!(
            p.negotiate(
                &auth(),
                NegotiationRequest {
                    item_id: "item",
                    capabilities: caps(),
                    position: Milliseconds::ZERO,
                    selection: None
                }
            )
            .await
            .unwrap_err()
            .failure,
            Failure::Unsupported
        );
        task.await.unwrap();
    }
    for bad in [
        ITEM.replace("Movie", "TvChannel"),
        ITEM.replace("\"Type\":\"Movie\"", "\"Type\":\"Movie\",\"PartCount\":2"),
    ] {
        let (p, requests, task) = provider(vec![reply(bad)], 1000).await;
        assert!(
            p.negotiate(
                &auth(),
                NegotiationRequest {
                    item_id: "item",
                    capabilities: caps(),
                    position: Milliseconds::ZERO,
                    selection: None
                }
            )
            .await
            .is_err()
        );
        task.await.unwrap();
        assert_eq!(requests.lock().unwrap().len(), 1);
    }
}
#[tokio::test]
async fn range_head_status_and_header_allowlist() {
    for (head, status, headers) in [
        (
            false,
            "206 Partial Content",
            "Content-Type: video/mp4\r\nContent-Range: bytes 0-2/100\r\nAccept-Ranges: bytes\r\nETag: \"test\"\r\n",
        ),
        (true, "200 OK", "Content-Type: video/mp4\r\n"),
        (
            false,
            "416 Range Not Satisfiable",
            "Content-Range: bytes */100\r\n",
        ),
    ] {
        let (p,requests,task)=provider(vec![Reply {status,headers:format!("{headers}Set-Cookie: private=secret\r\nLocation: http://private/\r\nServer: private\r\n"),..reply("abc")}],1000).await;
        let r = resource(&p, "Videos/item/stream.mp4");
        let mut stream = p
            .stream(
                &auth(),
                &r,
                MediaRequest {
                    head,
                    range: Some(ByteRange::parse("bytes=0-2").unwrap()),
                    if_range: Some("\"test\"".into()),
                },
            )
            .await
            .unwrap();
        assert_eq!(stream.status, status[..3].parse::<u16>().unwrap());
        for h in ["set-cookie", "location", "server"] {
            assert!(!stream.headers.contains_key(h));
        }
        if !head && status.starts_with("206") {
            assert_eq!(stream.next_chunk().await.unwrap().unwrap(), b"abc");
        } else {
            assert!(stream.next_chunk().await.unwrap().is_none());
        }
        task.await.unwrap();
        let requests = requests.lock().unwrap();
        assert!(requests[0].starts_with(if head { "HEAD " } else { "GET " }));
        assert!(requests[0].contains("range: bytes=0-2"));
        assert!(requests[0].contains("if-range: \"test\""));
        assert!(requests[0].contains("test-owned-user-token"));
        assert!(!requests[0].contains("test-owned-admin-key"));
    }
    for valid in ["bytes=0-", "bytes=-10", "bytes=0-0"] {
        ByteRange::parse(valid).unwrap();
    }
    for invalid in [
        "bytes=0-1,2-3",
        "bytes=-0",
        "bytes=2-1",
        "bytes=-",
        "items=0-1",
    ] {
        assert!(ByteRange::parse(invalid).is_err());
    }
}
#[tokio::test]
async fn large_content_length_and_chunked_media_preserve_all_bytes_in_bounded_outputs() {
    use sha2::Digest;
    let body = "0123456789abcdef".repeat(128 * 1024);
    for chunked in [false, true] {
        let response = if chunked {
            let raw = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n",
                body.len()
            );
            Reply {
                raw: true,
                ..reply(raw)
            }
        } else {
            Reply {
                headers: "Content-Type: video/mp4\r\n".into(),
                ..reply(body.clone())
            }
        };
        let (p, _, task) = provider(vec![response], 10000).await;
        let mut stream = p
            .stream(
                &auth(),
                &resource(&p, "Videos/item/stream.mp4"),
                MediaRequest::default(),
            )
            .await
            .unwrap();
        let mut digest = sha2::Sha256::new();
        let mut length = 0;
        while let Some(chunk) = stream.next_chunk().await.unwrap() {
            assert!(chunk.len() <= 256 * 1024);
            length += chunk.len();
            digest.update(&chunk);
        }
        assert_eq!(length, body.len());
        assert_eq!(digest.finalize(), sha2::Sha256::digest(body.as_bytes()));
        assert!(stream.next_chunk().await.unwrap().is_none());
        task.await.unwrap();
    }
}

#[tokio::test]
async fn media_stall_and_truncated_body_errors_close_stream_permanently() {
    for stall in [false, true] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let body = vec![b'x'; 1024 * 1024];
        let length = body.len();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            read_request(&mut socket).await;
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        length + 1
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            socket.write_all(&body).await.unwrap();
            if stall {
                let mut buffer = [0; 1];
                assert_eq!(
                    tokio::time::timeout(Duration::from_secs(3), socket.read(&mut buffer))
                        .await
                        .unwrap()
                        .unwrap(),
                    0
                );
            }
        });
        let admin = LiveJellyfin::from_api_key(
            super::super::ConnectionConfig {
                source_key: "test".into(),
                base_url: format!("http://{address}/jellyfin/"),
                timeout_ms: 1000,
                max_response_bytes: 1024,
            },
            ApiKey::new("test-owned-admin-key".into()).unwrap(),
        )
        .unwrap();
        let p = test_adapter(&admin, 1000);
        let mut stream = p
            .stream(
                &auth(),
                &resource(&p, "Videos/item/stream.mp4"),
                MediaRequest::default(),
            )
            .await
            .unwrap();
        let mut received = 0;
        let failure = loop {
            match stream.next_chunk().await {
                Ok(Some(chunk)) => {
                    assert!(chunk.len() <= 256 * 1024);
                    received += chunk.len();
                }
                Ok(None) => panic!("incomplete body cannot succeed"),
                Err(error) => break error.failure,
            }
        };
        assert_eq!(received, length);
        assert_eq!(
            failure,
            if stall {
                Failure::Deadline
            } else {
                Failure::Transport
            }
        );
        assert!(stream.next_chunk().await.unwrap().is_none());
        assert!(stream.next_chunk().await.unwrap().is_none());
        task.await.unwrap();
    }
}

#[tokio::test]
async fn streaming_does_not_buffer_film_idle_and_drop_cancel() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0u8; 4096];
        assert!(socket.read(&mut request).await.unwrap() > 0);
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 900000000\r\nContent-Type: video/mp4\r\n\r\nabc").await.unwrap();
        // No rest of film is sent. Returning headers and the first chunk must not wait for it.
        let n = tokio::time::timeout(Duration::from_secs(2), socket.read(&mut request))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(n, 0, "drop must close the unread response");
    });
    let admin = LiveJellyfin::from_api_key(
        super::super::ConnectionConfig {
            source_key: "test".into(),
            base_url: format!("http://{address}/jellyfin/"),
            timeout_ms: 60,
            max_response_bytes: 1024,
        },
        ApiKey::new("admin".into()).unwrap(),
    )
    .unwrap();
    let p = test_adapter(&admin, 60);
    let mut stream = p
        .stream(
            &auth(),
            &resource(&p, "Videos/item/stream.mp4"),
            MediaRequest::default(),
        )
        .await
        .unwrap();
    assert_eq!(stream.next_chunk().await.unwrap().unwrap(), b"abc");
    // Whole-request timeout is absent: keep the response beyond its header deadline.
    tokio::time::sleep(Duration::from_millis(80)).await;
    assert_eq!(
        stream.next_chunk().await.unwrap_err().failure,
        Failure::Deadline
    );
    assert!(stream.next_chunk().await.unwrap().is_none());
    drop(stream);
    task.await.unwrap();

    let (p, _, task) = provider(
        vec![Reply {
            raw: true,
            body:
                "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/\r\nContent-Length: 0\r\n\r\n"
                    .into(),
            ..reply("")
        }],
        1000,
    )
    .await;
    assert_eq!(
        p.stream(
            &auth(),
            &resource(&p, "Videos/item/stream.mp4"),
            MediaRequest::default()
        )
        .await
        .unwrap_err()
        .failure,
        Failure::Http(302)
    );
    task.await.unwrap();
}
#[tokio::test]
async fn resources_reject_external_traversal_basepath_scope_and_unknown_forms() {
    let (p, _, task) = provider(vec![], 1000).await;
    task.await.unwrap();
    for path in [
        "https://example.com/jellyfin/Videos/item/a.ts",
        "/Videos/item/a.ts",
        "../Videos/item/a.ts",
        "Videos/item/%2e%2e/a.ts",
        "Videos/item/a%2fts",
        "Videos/item/a.ts#fragment",
        "//example.com/a.ts",
        "Videos/other/a.ts",
        "Videos/item/a.js",
        "Videos/item/a.ts?MediaSourceId=other",
        "Videos/item/a.ts?DeviceId=other",
        "Videos/item/a.ts?Url=http://evil",
        "Videos/item/a.ts?foo=1&FOO=2",
        "Videos/item/other/Subtitles/3/Stream.vtt",
    ] {
        assert!(
            p.resource(&scope(), &p.base_directory().unwrap(), path, 0)
                .is_err(),
            "{path}"
        );
    }
    let r = resource(
        &p,
        "Videos/item/a.ts?api_key=secret&Access_Token=secret&MediaSourceId=source",
    );
    assert!(!r.url.as_str().contains("secret"));
    assert!(r.url.path().starts_with("/jellyfin/"));
    assert!(!format!("{r:?}").contains("http"));
    let mut wrong = auth();
    wrong.intent.device = GatewayDeviceId::generate().unwrap();
    assert_eq!(
        p.stream(&wrong, &r, MediaRequest::default())
            .await
            .unwrap_err()
            .failure,
        Failure::Identity
    );
}
#[tokio::test]
async fn playlist_rewrites_every_uri_attribute_and_strips_secrets() {
    let (p, _, task) = provider(vec![], 1000).await;
    task.await.unwrap();
    let parent = resource(&p, "Videos/item/master.m3u8");
    let body=b"#EXTM3U\n#EXT-X-VERSION:7\n#EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID=\"subs\",NAME=\"English\",URI=\"source/Subtitles/3/subtitles.m3u8?api_key=secret\"\n#EXT-X-I-FRAME-STREAM-INF:BANDWIDTH=100,URI=\"iframes.m3u8?api_key=secret\"\n#EXT-X-STREAM-INF:BANDWIDTH=100,CODECS=\"avc1.42E01E,mp4a.40.2\",SUBTITLES=\"subs\"\nmain.m3u8?api_key=secret\n";
    let mapped = Arc::new(Mutex::new(Vec::new()));
    let out = mapped.clone();
    let mut budget = PlaylistBudget::default();
    let mut mapper = move |r: ResourceRef| {
        assert!(!r.url.as_str().contains("secret"));
        let mut out = out.lock().unwrap();
        out.push(r);
        Ok(format!("/playback/v1/media/resource-{}", out.len()))
    };
    let mut output = p
        .rewrite_playlist(&auth(), &parent, body, &mut budget, &mut mapper)
        .unwrap();
    output.push_str(&p.rewrite_playlist(
        &auth(), &parent,
        b"#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4?api_key=secret\",BYTERANGE=\"10@0\"\n#EXTINF:6.0,private title with api_key=secret\na.ts?api_key=secret\n#EXT-X-ENDLIST\n",
        &mut budget, &mut mapper,
    ).unwrap());
    assert_eq!(budget.resources(), 5);
    assert_eq!(mapped.lock().unwrap().len(), 5);
    assert!(!output.contains("secret"));
    assert!(!output.contains("private title"));
    assert!(output.contains("URI=\"/playback/v1/media/resource-1\""));
    assert!(output.contains("\n/playback/v1/media/resource-3\n"));
    assert_eq!(mapped.lock().unwrap()[3].kind, ResourceKind::Initialization);
}
#[tokio::test]
async fn playlist_accepts_only_complete_vod_media_or_unmixed_master_before_mapping() {
    let (p, _, task) = provider(vec![], 1000).await;
    task.await.unwrap();
    let parent = resource(&p, "Videos/item/master.m3u8");
    let mut budget = PlaylistBudget::default();
    for body in [
        "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=100,VIDEO-RANGE=SDR\nmain.m3u8\n",
        "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:6,\n#EXT-X-BYTERANGE:10@0\na.ts\n#EXT-X-ENDLIST\n",
    ] {
        let mut calls = 0;
        let output = p
            .rewrite_playlist(&auth(), &parent, body.as_bytes(), &mut budget, |_| {
                calls += 1;
                Ok(format!("/playback/v1/media/valid-{calls}"))
            })
            .unwrap();
        assert!(calls > 0);
        assert!(output.contains("/playback/v1/media/valid-1"));
    }
    for body in [
        "#EXTM3U\n#EXTINF:6,\na.ts\n",
        "#EXTM3U\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXTINF:6,\na.ts\n",
        "#EXTM3U\n",
        "#EXTM3U\n#EXT-X-ENDLIST\n",
        "#EXTM3U\n#EXT-X-TARGETDURATION:6\n#EXT-X-ENDLIST\n",
        "#EXTM3U\n#EXT-X-MEDIA:TYPE=SUBTITLES,URI=\"subs.m3u8\"\n",
        "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=100\n",
        "#EXTM3U\n#EXTINF:6,\n#EXT-X-ENDLIST\n",
        "#EXTM3U\na.ts\n#EXT-X-ENDLIST\n",
        "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=100\nmain.m3u8\n#EXTINF:6,\na.ts\n#EXT-X-ENDLIST\n",
        "#EXTM3U\n#EXTINF:6,\na.ts\n#EXT-X-STREAM-INF:BANDWIDTH=100\nmain.m3u8\n#EXT-X-ENDLIST\n",
        "#EXTM3U\n#EXTINF:6,\na.ts\n#EXT-X-ENDLIST\n#EXT-X-ENDLIST\n",
        "#EXTM3U\n#EXTINF:6,\na.ts\n#EXT-X-ENDLIST\nb.ts\n",
        "#EXTM3U\n#EXTINF:6,\na.ts\n#EXT-X-UNKNOWN:URI=\"secret\"\n#EXT-X-ENDLIST\n",
        "#EXTM3U\n#EXTM3U\n#EXTINF:6,\na.ts\n#EXT-X-ENDLIST\n",
        "#EXTM3U\n#EXTINF:6,\nmain.m3u8\n#EXT-X-ENDLIST\n",
        "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=100\na.ts\n",
        "#EXTM3U\n#EXT-X-MAP:URI=\"init.mp4\",EVIL-URI=\"secret\"\n#EXTINF:6,\na.ts\n#EXT-X-ENDLIST\n",
    ] {
        let before = budget.resources();
        let mut calls = 0;
        assert!(
            p.rewrite_playlist(&auth(), &parent, body.as_bytes(), &mut budget, |_| {
                calls += 1;
                Ok("/playback/v1/media/unexpected".into())
            },)
                .is_err(),
            "{body}"
        );
        assert_eq!(calls, 0, "invalid manifests must not publish mappings");
        assert_eq!(budget.resources(), before);
    }
}

#[tokio::test]
async fn playlist_rejects_encryption_unknown_uri_forms_and_bounds() {
    let (p, _, task) = provider(vec![], 1000).await;
    task.await.unwrap();
    let parent = resource(&p, "Videos/item/master.m3u8");
    for line in [
        "#EXT-X-KEY:METHOD=AES-128,URI=\"key\"",
        "#EXT-X-SESSION-KEY:METHOD=NONE",
        "#EXT-X-UNKNOWN:URI=\"external\"",
        "#EXT-X-PART:URI=\"part.ts\"",
        "#EXT-X-MAP:URI=init.mp4",
        "#EXT-X-MAP:URI=\"init.mp4\",EVIL-URI=\"external\"",
        "#EXT-X-MEDIA:TYPE=SUBTITLES,URI=\"http://evil/a.m3u8\"",
        "#EXT-X-MAP:URI=\"init.mp4\",URI=\"other.mp4\"",
        "https://evil/a.ts",
        "../a.ts",
        "a.ts?api_key=secret\n#EXT-X-UNKNOWN:URL=\"secret\"",
    ] {
        let body = if line.starts_with("#EXT-X-MEDIA:") {
            format!("#EXTM3U\n{line}\n#EXT-X-STREAM-INF:BANDWIDTH=100\nmain.m3u8\n")
        } else if line.starts_with('#') {
            format!("#EXTM3U\n{line}\n#EXTINF:6,\na.ts\n#EXT-X-ENDLIST\n")
        } else {
            format!("#EXTM3U\n#EXTINF:6,\n{line}\n#EXT-X-ENDLIST\n")
        };
        assert!(
            p.rewrite_playlist(
                &auth(),
                &parent,
                body.as_bytes(),
                &mut PlaylistBudget::default(),
                |_| Ok("/playback/v1/media/x".into())
            )
            .is_err(),
            "{line}"
        );
    }
    let mut budget = PlaylistBudget::default();
    let mut calls = 0;
    assert_eq!(
        p.rewrite_playlist(
            &auth(),
            &parent,
            &vec![b'x'; 4 * 1024 * 1024 + 1],
            &mut budget,
            |_| {
                calls += 1;
                Ok("/playback/v1/media/x".into())
            }
        )
        .unwrap_err()
        .failure,
        Failure::Bounds
    );
    assert_eq!(calls, 0);
    assert_eq!(budget.resources(), 0);
    let body = format!(
        "#EXTM3U\n{}#EXT-X-ENDLIST\n",
        (0..8193)
            .map(|n| format!("#EXTINF:6,\na{n}.ts\n"))
            .collect::<String>()
    );
    assert_eq!(
        p.rewrite_playlist(&auth(), &parent, body.as_bytes(), &mut budget, |_| {
            calls += 1;
            Ok("/playback/v1/media/x".into())
        })
        .unwrap_err()
        .failure,
        Failure::Bounds
    );
    assert_eq!(calls, 0);
    assert_eq!(budget.resources(), 0);
    let mut deepest = parent;
    deepest.depth = 4;
    assert!(
        p.rewrite_playlist(
            &auth(),
            &deepest,
            b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=100\nchild.m3u8\n",
            &mut PlaylistBudget::default(),
            |_| Ok("/playback/v1/media/x".into())
        )
        .is_err()
    );
    assert!(
        p.rewrite_playlist(
            &auth(),
            &deepest,
            b"#EXTM3U\n#EXTINF:6,\na.ts\n#EXT-X-ENDLIST\n",
            &mut PlaylistBudget::default(),
            |_| Ok("/playback/v1/media/x".into())
        )
        .is_ok()
    );
}

#[tokio::test]
async fn complete_film_playlist_streams_and_rewrites_with_finite_graph_budget() {
    let device = intent().device().as_private_str().to_owned();
    let mut query = format!(
        "MediaSourceId=source&DeviceId={device}&PlaySessionId=play&VideoCodec=h264&AudioCodec=aac&SubtitleStreamIndex=-1&api_key=fake-playlist-token&Padding="
    );
    query.push_str(&"x".repeat(720 - query.len()));
    let mut body = String::from(
        "#EXTM3U\n#EXT-X-PLAYLIST-TYPE:VOD\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:4\n#EXT-X-MEDIA-SEQUENCE:0\n",
    );
    for index in 0..2859 {
        body.push_str(&format!(
            "#EXTINF:3.003000,\nhls1/main/{index}.ts?{query}\n"
        ));
    }
    body.push_str("#EXT-X-ENDLIST\n");
    assert!(body.len() > 2 * 1024 * 1024);
    assert!(body.len() < 4 * 1024 * 1024);
    let media = chunked_reply(&body, "application/vnd.apple.mpegurl");
    let (p, requests, task) = provider(vec![media], 10000).await;
    let parent = resource(&p, "Videos/item/main.m3u8");
    let mut budget = PlaylistBudget::default();
    let mut calls = 0;
    let output = p
        .playlist(&auth(), &parent, &mut budget, |resource| {
            assert_eq!(resource.kind(), ResourceKind::Segment);
            assert!(!resource.url.as_str().contains("fake-playlist-token"));
            calls += 1;
            Ok(format!("/playback/v1/media/{calls:064x}"))
        })
        .await
        .unwrap();
    task.await.unwrap();
    assert_eq!(requests.lock().unwrap().len(), 1);
    assert_eq!(calls, 2859);
    assert_eq!(budget.resources(), 2859);
    assert_eq!(
        output.lines().filter(|line| line.starts_with('/')).count(),
        2859
    );
    assert!(output.ends_with("#EXT-X-ENDLIST\n"));
    assert!(!output.contains("Padding"));
    assert!(!output.contains("fake-playlist-token"));
    assert!(output.len() < 2 * 1024 * 1024);
}

#[tokio::test]
async fn playlist_stream_cap_and_subtitle_cap_remain_separate() {
    for (kind, limit) in [
        (ResourceKind::Playlist, 4 * 1024 * 1024),
        (ResourceKind::Subtitle, 2 * 1024 * 1024),
    ] {
        let media = chunked_reply(
            &"x".repeat(limit + 1),
            if kind == ResourceKind::Playlist {
                "application/vnd.apple.mpegurl"
            } else {
                "text/vtt"
            },
        );
        let (p, _, task) = provider(vec![media], 10000).await;
        let mut budget = PlaylistBudget::default();
        let mut calls = 0;
        let error = if kind == ResourceKind::Playlist {
            let parent = resource(&p, "Videos/item/main.m3u8");
            p.playlist(&auth(), &parent, &mut budget, |_| {
                calls += 1;
                Ok("/playback/v1/media/unexpected".into())
            })
            .await
            .unwrap_err()
        } else {
            let subtitle = resource(&p, "Videos/item/source/Subtitles/3/Stream.vtt");
            p.subtitle(&auth(), &subtitle).await.unwrap_err()
        };
        assert_eq!(error.failure, Failure::Bounds);
        assert_eq!(calls, 0);
        assert_eq!(budget.resources(), 0);
        task.await.unwrap();
    }
    let mut media = reply("WEBVTT\n");
    media.headers = "Content-Type: text/vtt\r\n".into();
    let (p, _, task) = provider(vec![media], 1000).await;
    let subtitle = resource(&p, "Videos/item/source/Subtitles/3/Stream.vtt");
    let stream = p
        .stream(&auth(), &subtitle, MediaRequest::default())
        .await
        .unwrap();
    assert_eq!(
        stream
            .bounded_body(2 * 1024 * 1024 + 1)
            .await
            .unwrap_err()
            .failure,
        Failure::Bounds
    );
    task.await.unwrap();
}

#[tokio::test]
async fn alternate_audio_remains_remuxed_across_subtitle_only_and_repeated_updates() {
    let mut replies = vec![reply(ITEM), reply(negotiation(source()))];
    for subtitle in [None, Some(3), Some(3), None] {
        let mut selected = source();
        selected["DefaultAudioStreamIndex"] = json!(2);
        selected["DefaultSubtitleStreamIndex"] = json!(subtitle);
        selected["TranscodingUrl"] =
            json!("/videos/item/master.m3u8?VideoCodec=copy&AudioCodec=aac&AudioStreamIndex=2");
        replies.extend([reply(ITEM), reply(negotiation(selected)), reply("")]);
    }
    replies.extend([reply(ITEM), reply(negotiation(source()))]);
    let (p, requests, task) = provider(replies, 1000).await;
    let mut playback = p
        .negotiate(
            &auth(),
            NegotiationRequest {
                item_id: "item",
                capabilities: caps(),
                position: Milliseconds::ZERO,
                selection: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(playback.method, PlaybackMethod::DirectPlay);
    for (step, subtitle) in [None, Some(2), Some(2), None].into_iter().enumerate() {
        let selection = playback
            .selection(
                (step == 0).then_some(&playback.tracks[1]),
                subtitle.map(|index| &playback.tracks[index]),
            )
            .unwrap();
        playback = p
            .negotiate(
                &auth(),
                NegotiationRequest {
                    item_id: "item",
                    capabilities: caps(),
                    position: Milliseconds::ZERO,
                    selection: Some(&selection),
                },
            )
            .await
            .unwrap();
        assert_eq!(playback.method, PlaybackMethod::DirectStream);
        assert_eq!(playback.resource.kind, ResourceKind::Playlist);
        assert!(
            !playback
                .resource
                .url
                .query_pairs()
                .any(|(key, _)| key == "static")
        );
        assert!(playback.tracks[1].selected);
        assert!(!playback.tracks[0].selected);
        assert_eq!(playback.tracks[2].selected, subtitle.is_some());
        p.report(
            &auth(),
            &playback,
            PlaybackReport {
                kind: ReportKind::Progress,
                position: Milliseconds::ZERO,
                paused: false,
                muted: false,
                volume: 1.0,
            },
        )
        .await
        .unwrap();
    }
    let selection = playback.selection(Some(&playback.tracks[0]), None).unwrap();
    let restored = p
        .negotiate(
            &auth(),
            NegotiationRequest {
                item_id: "item",
                capabilities: caps(),
                position: Milliseconds::ZERO,
                selection: Some(&selection),
            },
        )
        .await
        .unwrap();
    assert_eq!(restored.method, PlaybackMethod::DirectPlay);
    task.await.unwrap();
    let requests = requests.lock().unwrap();
    for step in 0..4 {
        let negotiation: Value =
            serde_json::from_str(requests[3 + step * 3].split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(negotiation["AudioStreamIndex"], 2);
        assert_eq!(negotiation["EnableDirectPlay"], false);
        let report: Value =
            serde_json::from_str(requests[4 + step * 3].split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(report["AudioStreamIndex"], 2);
        assert_eq!(report["PlayMethod"], "DirectStream");
    }
}

#[tokio::test]
async fn track_changes_require_confirmed_provider_choices_and_full_vtt_timing() {
    let mut second = source();
    second["DefaultAudioStreamIndex"] = json!(2);
    second["DefaultSubtitleStreamIndex"] = json!(4);
    second["SupportsDirectPlay"] = json!(false);
    second["TranscodingUrl"] = json!(
        "/videos/item/master.m3u8?MediaSourceId=source&PlaySessionId=play&VideoCodec=h264&AudioCodec=aac&AudioStreamIndex=2&SubtitleStreamIndex=4&SubtitleMethod=Encode"
    );
    let (p, requests, task) = provider(
        vec![
            reply(ITEM),
            reply(negotiation(source())),
            reply(ITEM),
            reply(negotiation(second)),
            Reply {
                headers: "Content-Type: text/vtt\r\n".into(),
                ..reply("WEBVTT\n\n00:02:00.000 --> 00:02:01.000\nhello\n")
            },
        ],
        1000,
    )
    .await;
    let first = p
        .negotiate(
            &auth(),
            NegotiationRequest {
                item_id: "item",
                capabilities: caps(),
                position: Milliseconds::ZERO,
                selection: None,
            },
        )
        .await
        .unwrap();
    let selection = first
        .selection(Some(&first.tracks[1]), Some(&first.tracks[3]))
        .unwrap();
    let changed = p
        .negotiate(
            &auth(),
            NegotiationRequest {
                item_id: "item",
                capabilities: caps(),
                position: Milliseconds::parse_decimal("120000").unwrap(),
                selection: Some(&selection),
            },
        )
        .await
        .unwrap();
    assert_eq!(changed.method, PlaybackMethod::Transcode);
    assert_eq!(changed.timeline_offset, Milliseconds::ZERO);
    assert!(changed.tracks[1].selected && changed.tracks[3].selected);
    let vtt = p
        .subtitle(&auth(), first.tracks[2].resource.as_ref().unwrap())
        .await
        .unwrap();
    assert!(String::from_utf8(vtt).unwrap().contains("00:02:00.000"));
    task.await.unwrap();
    let requests = requests.lock().unwrap();
    assert!(requests[3].contains("\"AlwaysBurnInSubtitleWhenTranscoding\":true"));
    assert!(requests[3].contains("\"EnableDirectPlay\":false"));
    let body: Value = serde_json::from_str(requests[3].split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(body["MediaSourceId"], "source");
    assert_eq!(body["SubtitleStreamIndex"], 4);
    assert_eq!(body["AudioStreamIndex"], 2);
    assert!(requests[4].starts_with("GET /jellyfin/Videos/item/source/Subtitles/3/Stream.vtt?copyTimestamps=true&addVttTimeMap=false "));
}
#[tokio::test]
async fn incompatible_subtitles_codecs_and_provider_choices_fail_closed() {
    let mut s = source();
    s["MediaStreams"][0]["BitDepth"] = json!(10);
    s["TranscodingUrl"] = json!("/videos/item/master.m3u8?VideoCodec=h264&AudioCodec=aac");
    let (p, _, task) = provider(vec![reply(ITEM), reply(negotiation(s))], 1000).await;
    let playback = p
        .negotiate(
            &auth(),
            NegotiationRequest {
                item_id: "item",
                capabilities: caps(),
                position: Milliseconds::ZERO,
                selection: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(playback.method, PlaybackMethod::Transcode);
    task.await.unwrap();
    let (p, _, task) = provider(vec![reply(ITEM), reply(negotiation(source()))], 1000).await;
    let playback = p
        .negotiate(
            &auth(),
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
    assert!(!playback.tracks[1].available);
    assert_eq!(
        playback.tracks[2].subtitle_mode,
        Some(SubtitleMode::Unavailable)
    );
    assert_eq!(
        playback.tracks[3].subtitle_mode,
        Some(SubtitleMode::Unavailable)
    );
    assert!(playback.selection(None, Some(&playback.tracks[3])).is_err());
    task.await.unwrap();
    for query in [
        "VideoCodec=hevc",
        "VideoCodec=h264&CopyTimestamps=false",
        "VideoCodec=h264&StartTimeTicks=10000",
    ] {
        let mut s = source();
        s["SupportsDirectPlay"] = json!(false);
        s["TranscodingUrl"] = json!(format!("/videos/item/master.m3u8?{query}"));
        let (p, _, task) = provider(vec![reply(ITEM), reply(negotiation(s))], 1000).await;
        assert!(
            p.negotiate(
                &auth(),
                NegotiationRequest {
                    item_id: "item",
                    capabilities: caps(),
                    position: Milliseconds::ZERO,
                    selection: None
                }
            )
            .await
            .is_err()
        );
        task.await.unwrap();
    }
}
#[tokio::test]
async fn stream_drop_header_deadline_and_subtitle_body_guards() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0; 4096];
        assert!(socket.read(&mut buf).await.unwrap() > 0);
        socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 999999999\r\nContent-Type: video/mp4\r\n\r\nabc").await.unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), socket.read(&mut buf))
                .await
                .unwrap()
                .unwrap(),
            0
        );
    });
    let admin = LiveJellyfin::from_api_key(
        super::super::ConnectionConfig {
            source_key: "test".into(),
            base_url: format!("http://{address}/jellyfin/"),
            timeout_ms: 1000,
            max_response_bytes: 1024,
        },
        ApiKey::new("admin".into()).unwrap(),
    )
    .unwrap();
    let p = test_adapter(&admin, 1000);
    let stream = p
        .stream(
            &auth(),
            &resource(&p, "Videos/item/stream.mp4"),
            MediaRequest::default(),
        )
        .await
        .unwrap();
    drop(stream);
    task.await.unwrap();
    let (p, _, task) = provider(
        vec![Reply {
            delay: 100,
            headers: "Content-Type: video/mp4\r\n".into(),
            ..reply("abc")
        }],
        20,
    )
    .await;
    assert_eq!(
        p.stream(
            &auth(),
            &resource(&p, "Videos/item/stream.mp4"),
            MediaRequest::default()
        )
        .await
        .unwrap_err()
        .failure,
        Failure::Deadline
    );
    task.await.unwrap();
    for (body, mime) in [
        (
            "WEBVTT\nX-TIMESTAMP-MAP=MPEGTS:900000,LOCAL:00:00:00.000\n",
            "text/vtt",
        ),
        ("WEBVTT\n\ntest-owned-user-token\n", "text/vtt"),
        ("<html>error</html>", "text/html"),
        ("not webvtt", "text/vtt"),
    ] {
        let (p, _, task) = provider(
            vec![Reply {
                headers: format!("Content-Type: {mime}\r\n"),
                ..reply(body)
            }],
            1000,
        )
        .await;
        assert!(
            p.subtitle(
                &auth(),
                &resource(&p, "Videos/item/source/Subtitles/3/Stream.vtt")
            )
            .await
            .is_err()
        );
        task.await.unwrap();
    }
}
#[tokio::test]
async fn canonical_uuid_url_paths_and_playlist_budget_deduplication() {
    let (p, _, task) = provider(vec![], 1000).await;
    task.await.unwrap();
    let mut scope = scope();
    scope.item = "00112233445566778899aabbccddeeff".into();
    assert!(
        p.resource(
            &scope,
            &p.base_directory().unwrap(),
            "videos/00112233-4455-6677-8899-aabbccddeeff/master.m3u8",
            0
        )
        .is_ok()
    );
    let parent = resource(&p, "Videos/item/master.m3u8");
    let mut budget = PlaylistBudget::default();
    for _ in 0..3 {
        p.rewrite_playlist(
            &auth(),
            &parent,
            b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=100\nmain.m3u8?api_key=discard\n",
            &mut budget,
            |_| Ok("/playback/v1/media/opaque".into()),
        )
        .unwrap();
    }
    assert_eq!(budget.resources(), 1);
    assert!(!format!("{budget:?}").contains("http"));
}

#[tokio::test]
async fn progress_and_stopped_use_user_session_without_admin_attribution_override() {
    let (p, requests, task) = provider(vec![reply(""), reply(""), reply("")], 1000).await;
    for kind in [ReportKind::Start, ReportKind::Progress, ReportKind::Stopped] {
        p.report(
            &auth(),
            &transport(&p),
            PlaybackReport {
                kind,
                position: Milliseconds::parse_decimal("123456").unwrap(),
                paused: true,
                muted: true,
                volume: 0.25,
            },
        )
        .await
        .unwrap();
    }
    task.await.unwrap();
    for (request, path) in
        requests
            .lock()
            .unwrap()
            .iter()
            .zip(["Playing", "Playing/Progress", "Playing/Stopped"])
    {
        assert!(request.starts_with(&format!("POST /jellyfin/Sessions/{path} ")));
        assert!(request.contains("test-owned-user-token"));
        let body: Value = serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(body["PositionTicks"], 1234560000);
        assert_eq!(body["VolumeLevel"], 25);
        assert_eq!(body["IsPaused"], true);
        assert!(body.get("UserId").is_none());
    }
    let (p, requests, task) = provider(vec![], 1000).await;
    assert_eq!(
        p.report(
            &auth(),
            &transport(&p),
            PlaybackReport {
                kind: ReportKind::Start,
                position: Milliseconds::ZERO,
                paused: false,
                muted: false,
                volume: f64::NAN,
            }
        )
        .await
        .unwrap_err()
        .effect,
        Effect::NotDispatched
    );
    task.await.unwrap();
    assert!(requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn bounded_playlist_fetch_and_generated_track_labels_do_not_publish_provider_paths() {
    let mut s = source();
    s["MediaStreams"][1]["DisplayTitle"] = json!("/private/media/test-owned-user-token");
    s["MediaStreams"][1]["Language"] = json!("http://provider/private");
    let (p, _, task) = provider(
        vec![
            reply(ITEM),
            reply(negotiation(s)),
            chunked_reply(
                &format!("#EXTM3U\n{}", "a".repeat(4 * 1024 * 1024)),
                "application/vnd.apple.mpegurl",
            ),
        ],
        1000,
    )
    .await;
    let playback = p
        .negotiate(
            &auth(),
            NegotiationRequest {
                item_id: "item",
                capabilities: caps(),
                position: Milliseconds::ZERO,
                selection: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        playback.tracks[0].display_name,
        "Unknown language · AAC · Track 1"
    );
    assert!(playback.tracks[0].language.is_none());
    assert_eq!(
        p.playlist(
            &auth(),
            &resource(&p, "Videos/item/master.m3u8"),
            &mut PlaylistBudget::default(),
            |_| Ok("/playback/v1/media/opaque".into())
        )
        .await
        .unwrap_err()
        .failure,
        Failure::Bounds
    );
    task.await.unwrap();
}

#[tokio::test]
async fn lost_token_encoding_recovery_requires_exact_owned_device_and_verified_server() {
    let (p, requests, task) = provider(
        vec![reply(SERVER), reply(""), reply(SERVER), reply("")],
        1000,
    )
    .await;
    let i = intent();
    let encoding = EncodingIdentity::from_recorded(i.device.clone(), "play".into()).unwrap();
    p.recover_encoding(&i, &encoding).await.unwrap();
    p.recover_device(&i).await.unwrap();
    task.await.unwrap();
    {
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 4);
        assert!(requests[1].starts_with(&format!(
            "DELETE /jellyfin/Videos/ActiveEncodings?deviceId={}&playSessionId=play ",
            i.device.0
        )));
        assert!(requests[1].contains("test-owned-admin-key"));
        assert!(!requests[1].contains("test-owned-user-token"));
        assert!(requests[3].starts_with(&format!("DELETE /jellyfin/Devices?id={} ", i.device.0)));
    }
    let other =
        EncodingIdentity::from_recorded(GatewayDeviceId::generate().unwrap(), "play".into())
            .unwrap();
    assert_eq!(
        p.recover_encoding(&i, &other).await.unwrap_err().effect,
        Effect::NotDispatched
    );
}

#[test]
fn playback_transport_ignores_environment_proxy_configuration() {
    const CHILD: &str = "SCREENROOM_PLAYBACK_PROXY_TEST_CHILD";
    if std::env::var_os(CHILD).is_some() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let (p, _, task) = provider(
                vec![Reply {
                    headers: "Content-Type: video/mp4\r\n".into(),
                    ..reply("abc")
                }],
                1000,
            )
            .await;
            let stream = p
                .stream(
                    &auth(),
                    &resource(&p, "Videos/item/stream.mp4"),
                    MediaRequest::default(),
                )
                .await
                .unwrap();
            assert_eq!(stream.bounded_body(100).await.unwrap(), b"abc");
            task.await.unwrap();
        });
        return;
    }
    let mut child = std::process::Command::new(std::env::current_exe().unwrap());
    child
        .args([
            "--exact",
            "jellyfin::playback::tests::playback_transport_ignores_environment_proxy_configuration",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("NO_PROXY", "")
        .env("no_proxy", "");
    for key in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        child.env(key, "http://127.0.0.1:1");
    }
    assert!(child.status().unwrap().success());
}

#[tokio::test]
async fn official_sdr_master_metadata_is_supported_without_admitting_hdr() {
    let (p, _, task) = provider(vec![], 1000).await;
    task.await.unwrap();
    let parent = resource(&p, "Videos/item/master.m3u8");
    let master = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=8000000,AVERAGE-BANDWIDTH=7000000,CODECS=\"avc1.640028,mp4a.40.2\",RESOLUTION=1920x1080,FRAME-RATE=23.976,VIDEO-RANGE=SDR\nmain.m3u8\n";
    let rewritten = p
        .rewrite_playlist(
            &auth(),
            &parent,
            master.as_bytes(),
            &mut PlaylistBudget::default(),
            |_| Ok("/playback/v1/media/opaque".into()),
        )
        .unwrap();
    assert!(rewritten.contains("VIDEO-RANGE=SDR"));
    assert!(rewritten.contains("\n/playback/v1/media/opaque\n"));
    for unsupported in ["PQ", "HLG", "UNKNOWN", "\"SDR\""] {
        assert!(
            p.rewrite_playlist(
                &auth(),
                &parent,
                master
                    .replace("VIDEO-RANGE=SDR", &format!("VIDEO-RANGE={unsupported}"))
                    .as_bytes(),
                &mut PlaylistBudget::default(),
                |_| Ok("/playback/v1/media/opaque".into())
            )
            .is_err()
        );
    }
}

#[tokio::test]
async fn jellyfin_subtitles_off_encode_sentinel_is_pinned_on_dispatched_hls_request() {
    for method in [None, Some("External"), Some("Encode")] {
        for index in [None, Some("-1")] {
            for default_index in [None, Some(-1)] {
                let mut s = source();
                s["Container"] = json!("mkv");
                s["VideoType"] = json!("VideoFile");
                s["SupportsDirectPlay"] = json!(false);
                s["SupportsDirectStream"] = json!(false);
                s["DefaultSubtitleStreamIndex"] = json!(default_index);
                if default_index.is_none() {
                    s.as_object_mut()
                        .unwrap()
                        .remove("DefaultSubtitleStreamIndex");
                }
                s["MediaStreams"] = json!([
                    {"Type":"Video","Index":0,"Codec":"h264","BitDepth":8,"Level":41},
                    {"Type":"Audio","Index":1,"Codec":"dts","Channels":8}
                ]);
                for index in 2..=11 {
                    s["MediaStreams"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!({"Type":"Subtitle","Index":index,"Codec":"pgssub"}));
                }
                let mut url = String::from(
                    "/videos/item/master.m3u8?VideoCodec=h264&AudioCodec=aac&AudioStreamIndex=1",
                );
                if let Some(method) = method {
                    url.push_str(&format!("&SubtitleMethod={method}"));
                }
                if let Some(index) = index {
                    url.push_str(&format!("&SubtitleStreamIndex={index}"));
                }
                s["TranscodingUrl"] = json!(url);
                let mut item: Value = serde_json::from_str(ITEM).unwrap();
                item["MediaSources"] = json!([s.clone()]);
                let mut info: Value = serde_json::from_str(&negotiation(s)).unwrap();
                info["ErrorCode"] = Value::Null;
                let mut media = reply("#EXTM3U\n");
                media.headers = "Content-Type: application/vnd.apple.mpegurl\r\n".into();
                let (p, requests, task) = provider(
                    vec![reply(item.to_string()), reply(info.to_string()), media],
                    1000,
                )
                .await;
                let playback = p
                    .negotiate(
                        &auth(),
                        NegotiationRequest {
                            item_id: "item",
                            capabilities: caps(),
                            position: Milliseconds::ZERO,
                            selection: None,
                        },
                    )
                    .await
                    .unwrap();
                assert_eq!(playback.method, PlaybackMethod::Transcode);
                assert!(
                    playback
                        .tracks
                        .iter()
                        .all(|track| { track.kind != TrackKind::Subtitle || !track.selected })
                );
                p.stream(&auth(), &playback.resource, MediaRequest::default())
                    .await
                    .unwrap()
                    .bounded_body(1024)
                    .await
                    .unwrap();
                task.await.unwrap();
                let requests = requests.lock().unwrap();
                let target = requests[2].split_whitespace().nth(1).unwrap();
                let dispatched = Url::parse(&format!("http://test{target}")).unwrap();
                let indexes: Vec<_> = dispatched
                    .query_pairs()
                    .filter(|(key, _)| key.eq_ignore_ascii_case("SubtitleStreamIndex"))
                    .map(|(_, value)| value.into_owned())
                    .collect();
                assert_eq!(indexes, ["-1"]);
                assert_eq!(
                    dispatched
                        .query_pairs()
                        .find(|(key, _)| key == "SubtitleMethod")
                        .map(|(_, value)| value.into_owned()),
                    method.map(str::to_owned)
                );
            }
        }
    }
}

#[tokio::test]
async fn subtitles_off_rejects_contradictory_or_unknown_hls_choices_before_streaming() {
    for query in [
        "SubtitleMethod=Encode&SubtitleStreamIndex=2",
        "SubtitleMethod=Encode&SubtitleStreamIndex=invalid",
        "SubtitleMethod=Encode&SubtitleStreamIndex=",
        "SubtitleMethod=Encode&SubtitleStreamIndex=-2",
        "SubtitleMethod=Encode&SubtitleStreamIndex=0",
        "SubtitleMethod=Unknown",
        "SubtitleMethod=Embed&SubtitleStreamIndex=-1",
        "SubtitleMethod=Encode&SubtitleStreamIndex=-1&subtitlestreamindex=2",
    ] {
        let mut s = source();
        s["SupportsDirectPlay"] = json!(false);
        s["DefaultSubtitleStreamIndex"] = json!(-1);
        s["TranscodingUrl"] = json!(format!(
            "/videos/item/master.m3u8?VideoCodec=h264&AudioCodec=aac&{query}"
        ));
        let (p, requests, task) = provider(vec![reply(ITEM), reply(negotiation(s))], 1000).await;
        let error = p
            .negotiate(
                &auth(),
                NegotiationRequest {
                    item_id: "item",
                    capabilities: caps(),
                    position: Milliseconds::ZERO,
                    selection: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(
            error.failure,
            Failure::Unsupported | Failure::ResourceRejected
        ));
        task.await.unwrap();
        assert_eq!(requests.lock().unwrap().len(), 2);
    }
}

#[tokio::test]
async fn initial_negotiation_pins_discovered_source_and_turns_provider_default_burn_in_off() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let mut default = source();
        default["SupportsDirectPlay"] = json!(false);
        default["DefaultSubtitleStreamIndex"] = json!(4);
        default["TranscodingUrl"] = json!(
            "/videos/item/master.m3u8?VideoCodec=h264&AudioCodec=aac&SubtitleMethod=Encode&SubtitleStreamIndex=4"
        );
        let mut item: Value = serde_json::from_str(ITEM).unwrap();
        item["MediaSources"] = json!([default.clone()]);
        let (mut socket, _) = listener.accept().await.unwrap();
        let discovery = read_request(&mut socket).await;
        assert!(discovery.starts_with(
            "GET /jellyfin/Users/personal/Items/item?Fields=MediaSources%2CMediaStreams "
        ));
        let body = item.to_string();
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        drop(socket);
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = read_request(&mut socket).await;
        let body: Value = serde_json::from_str(request.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        // Model MediaInfoHelper: null/mismatching source leaves the provider's burn-in default.
        let pinned = body["MediaSourceId"] == "source" && body["SubtitleStreamIndex"] == -1;
        if pinned {
            default["DefaultSubtitleStreamIndex"] = json!(-1);
            default["TranscodingUrl"] =
                json!("/videos/item/master.m3u8?VideoCodec=h264&AudioCodec=aac");
        }
        let reply = negotiation(default);
        socket
            .write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                    reply.len()
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        assert!(
            pinned,
            "initial explicit subtitles-off must target the discovered source"
        );
    });
    let p = LiveJellyfin::from_api_key(
        super::super::ConnectionConfig {
            source_key: "test".into(),
            base_url: format!("http://{address}/jellyfin/"),
            timeout_ms: 1000,
            max_response_bytes: 1024 * 1024,
        },
        ApiKey::new("admin".into()).unwrap(),
    )
    .unwrap()
    .playback_adapter_fenced(
        intent(),
        PlaybackTimeouts::default(),
        Arc::new(AllowTestDispatch),
    )
    .unwrap();
    let playback = p
        .negotiate(
            &auth(),
            NegotiationRequest {
                item_id: "item",
                capabilities: caps(),
                position: Milliseconds::ZERO,
                selection: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(playback.method, PlaybackMethod::Transcode);
    assert!(
        playback
            .tracks
            .iter()
            .filter(|track| track.kind == TrackKind::Subtitle)
            .all(|track| !track.selected)
    );
    assert!(
        !playback
            .resource
            .url
            .as_str()
            .to_ascii_lowercase()
            .contains("subtitlemethod")
    );
    task.await.unwrap();
}

#[tokio::test]
async fn negotiation_rejects_unrequested_actual_subtitles_and_source_substitution() {
    for (default_index, url, source_id) in [
        (
            4,
            "/videos/item/master.m3u8?VideoCodec=h264&AudioCodec=aac",
            "source",
        ),
        (
            -1,
            "/videos/item/master.m3u8?VideoCodec=h264&AudioCodec=aac&SubtitleMethod=Encode&SubtitleStreamIndex=4",
            "source",
        ),
        (
            -1,
            "/videos/item/master.m3u8?VideoCodec=h264&AudioCodec=aac",
            "other",
        ),
    ] {
        let mut s = source();
        s["SupportsDirectPlay"] = json!(false);
        s["DefaultSubtitleStreamIndex"] = json!(default_index);
        s["TranscodingUrl"] = json!(url);
        s["Id"] = json!(source_id);
        let (p, requests, task) = provider(vec![reply(ITEM), reply(negotiation(s))], 1000).await;
        assert_eq!(
            p.negotiate(
                &auth(),
                NegotiationRequest {
                    item_id: "item",
                    capabilities: caps(),
                    position: Milliseconds::ZERO,
                    selection: None
                }
            )
            .await
            .unwrap_err()
            .failure,
            Failure::Unsupported
        );
        task.await.unwrap();
        let requests = requests.lock().unwrap();
        assert_eq!(
            requests.len(),
            2,
            "no automatic retry or alternate source negotiation"
        );
        let body: Value =
            serde_json::from_str(requests[1].split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert_eq!(body["MediaSourceId"], "source");
        assert_eq!(body["SubtitleStreamIndex"], -1);
    }
}

#[tokio::test]
async fn omitted_child_query_scope_is_authoritatively_bound_before_upstream_io() {
    let (p, requests, task) = provider(
        vec![
            Reply {
                headers: "Content-Type: video/mp4\r\n".into(),
                ..reply("abc")
            },
            Reply {
                headers: "Content-Type: application/vnd.apple.mpegurl\r\n".into(),
                ..reply("#EXTM3U\n")
            },
            Reply {
                headers: "Content-Type: video/mp2t\r\n".into(),
                ..reply("abc")
            },
        ],
        1000,
    )
    .await;
    let parent = resource(&p, "Videos/item/master.m3u8?ParentOnly=not-inherited");
    let mut children = Vec::new();
    for body in [
        "#EXTM3U\n#EXTINF:6,\nstream.mp4?static=true\n#EXT-X-ENDLIST\n",
        "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=100\nmain.m3u8\n",
        "#EXTM3U\n#EXTINF:6,\nhls1/main/0.ts\n#EXT-X-ENDLIST\n",
    ] {
        p.rewrite_playlist(
            &auth(),
            &parent,
            body.as_bytes(),
            &mut PlaylistBudget::default(),
            |resource| {
                children.push(resource);
                Ok(format!("/playback/v1/media/opaque-{}", children.len()))
            },
        )
        .unwrap();
    }
    for child in children {
        let _body = p
            .stream(&auth(), &child, MediaRequest::default())
            .await
            .unwrap()
            .bounded_body(100)
            .await
            .unwrap();
    }
    task.await.unwrap();
    for request in requests.lock().unwrap().iter() {
        let target = request
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap();
        let url = Url::parse(&format!("http://isolated{target}")).unwrap();
        let query: std::collections::BTreeMap<_, _> = url.query_pairs().collect();
        assert_eq!(
            query.get("mediaSourceId").map(|s| s.as_ref()),
            Some("source")
        );
        assert_eq!(
            query.get("deviceId").map(|s| s.as_ref()),
            Some(intent().device.0.as_str())
        );
        assert_eq!(query.get("playSessionId").map(|s| s.as_ref()), Some("play"));
        assert!(!query.contains_key("ParentOnly"));
        assert!(request.contains("test-owned-user-token"));
    }
    for suffix in [
        "mediaSourceId=other",
        "deviceId=other",
        "playSessionId=other",
    ] {
        assert!(
            p.resource(
                &scope(),
                &parent.url,
                &format!("stream.mp4?static=true&{suffix}"),
                0
            )
            .is_err()
        );
    }
    assert!(
        p.resource(&scope(), &parent.url, "other/Subtitles/3/Stream.vtt", 0)
            .is_err()
    );
}
