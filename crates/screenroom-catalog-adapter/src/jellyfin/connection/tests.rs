use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn config(base_url: String) -> ConnectionConfig {
    ConnectionConfig {
        source_key: "home".into(),
        base_url,
        timeout_ms: 1000,
        max_response_bytes: 8192,
    }
}
async fn provider(
    rows: Vec<(&'static str, &'static str, &'static str)>,
) -> (LiveJellyfin, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!(
        "http://localhost:{}/base/jellyfin/",
        listener.local_addr().unwrap().port()
    );
    let server = tokio::spawn(async move {
        for (path, status, body) in rows {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = [0; 8192];
            let mut n = 0;
            loop {
                let read = socket.read(&mut bytes[n..]).await.unwrap();
                assert!(read > 0);
                n += read;
                if bytes[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8_lossy(&bytes[..n]);
            assert!(
                request.starts_with(&format!("GET /base/jellyfin/{path} ")),
                "unexpected request target for route {path}"
            );
            assert!(
                request
                    .to_lowercase()
                    .contains("authorization: mediabrowser token=\"only-test-key\""),
                "missing expected authorization"
            );
            assert!(
                !request.lines().next().unwrap().contains("only-test-key"),
                "credential appeared in request target"
            );
            socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
    });
    (
        LiveJellyfin::from_api_key(config(base), ApiKey::new("only-test-key".into()).unwrap())
            .unwrap(),
        server,
    )
}
const IDENTITY: &str = r#"{"Id":"server123","ServerName":"My\u0000 Server\n","Version":"10.11.0"}"#;
const LIBRARIES: &str = r#"[{"ItemId":"films","Name":"Films","CollectionType":"movies"},{"ItemId":"tv","Name":"TV","CollectionType":"tvshows"},{"ItemId":"music","Name":"Music","CollectionType":"music"},{"ItemId":"sets","Name":"Sets","CollectionType":"boxsets"}]"#;
const PROFILES: &str = r#"[{"Id":"one","Name":"Enabled","Policy":{"IsDisabled":false,"IsHidden":false}},{"Id":"hidden","Name":"Hidden","Policy":{"IsDisabled":false,"IsHidden":true}},{"Id":"disabled","Name":"Disabled","Policy":{"IsDisabled":true,"IsHidden":false}}]"#;

#[tokio::test]
async fn server_identity_uses_exactly_one_authenticated_observed_request() {
    use screenroom_observability::transport::{
        BodyDirection, RequestKind, RequestOutcome, TransportObservation, TransportObserver,
    };
    #[derive(Default)]
    struct Observer {
        bytes: std::sync::atomic::AtomicU64,
        finished: std::sync::Mutex<Vec<RequestOutcome>>,
    }
    impl TransportObserver for Observer {
        fn body_chunk(&self, kind: RequestKind, direction: BodyDirection, bytes: u64) {
            assert_eq!(kind, RequestKind::Provider);
            assert_eq!(direction, BodyDirection::DownloadConsumed);
            self.bytes
                .fetch_add(bytes, std::sync::atomic::Ordering::Relaxed);
        }
        fn finished(&self, kind: RequestKind, outcome: RequestOutcome, _: std::time::Duration) {
            assert_eq!(kind, RequestKind::Provider);
            self.finished.lock().unwrap().push(outcome);
        }
    }
    let observed = std::sync::Arc::new(Observer::default());
    let (provider, server) = provider(vec![("System/Info", "200 OK", IDENTITY)]).await;
    let identity = provider
        .with_transport_observer(TransportObservation::new(observed.clone()))
        .server_identity()
        .await
        .unwrap();
    assert_eq!(
        identity,
        ServerIdentity {
            id: "server123".into(),
            name: "My Server".into(),
            version: ServerVersion {
                major: 10,
                minor: 11,
                patch: 0
            },
        }
    );
    assert_eq!(
        observed.bytes.load(std::sync::atomic::Ordering::Relaxed),
        IDENTITY.len() as u64
    );
    assert_eq!(
        *observed.finished.lock().unwrap(),
        [RequestOutcome::Succeeded]
    );
    server.await.unwrap();
}

#[tokio::test]
async fn server_identity_rejects_invalid_identity_and_authorization_without_secret_diagnostics() {
    for (case, (status, body, expected)) in [
        ("200 OK", "{}", Error::InvalidServerIdentity),
        (
            "200 OK",
            r#"{"Id":"","ServerName":"Name","Version":"10.11.0"}"#,
            Error::InvalidServerIdentity,
        ),
        (
            "200 OK",
            r#"{"Id":"bad/id","ServerName":"Name","Version":"10.11.0"}"#,
            Error::InvalidServerIdentity,
        ),
        (
            "200 OK",
            r#"{"Id":"server","ServerName":"Name","Version":"invalid"}"#,
            Error::InvalidServerIdentity,
        ),
        (
            "200 OK",
            r#"{"Id":"server","ServerName":"","Version":"10.11.0"}"#,
            Error::InvalidServerIdentity,
        ),
        ("200 OK", r#"{"Id":42}"#, Error::InvalidResponse),
        ("200 OK", "not JSON only-test-key", Error::InvalidResponse),
        (
            "401 Unauthorized",
            "only-test-key private provider error",
            Error::Unauthorized,
        ),
        (
            "403 Forbidden",
            "only-test-key private provider error",
            Error::Unauthorized,
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let (provider, server) = provider(vec![("System/Info", status, body)]).await;
        let error = provider.server_identity().await.unwrap_err();
        let completion = tokio::time::timeout(std::time::Duration::from_secs(2), server).await;
        assert!(
            matches!(completion, Ok(Ok(()))),
            "identity case {case}, status {status}, result {error:?}: fake-server completion {completion:?}"
        );
        assert_eq!(
            error, expected,
            "identity case {case}, status {status}; fake server completed"
        );
        let diagnostics = format!("{error} {error:?} {provider:?}");
        assert!(!diagnostics.contains("only-test-key"));
        assert!(!diagnostics.contains("private provider error"));
    }
}

#[tokio::test]
async fn authenticated_discovery_preserves_basepath_identity_and_profile_eligibility() {
    let (provider, server) = provider(vec![
        ("System/Info", "200 OK", IDENTITY),
        ("Library/VirtualFolders", "200 OK", LIBRARIES),
        ("Users", "200 OK", PROFILES),
    ])
    .await;
    assert!(provider.authorization.is_sensitive());
    let probe = provider.probe_connection().await.unwrap();
    assert_eq!(probe.server.id, "server123");
    assert_eq!(probe.server.name, "My Server");
    assert_eq!(
        probe.server.version,
        ServerVersion {
            major: 10,
            minor: 11,
            patch: 0
        }
    );
    assert_eq!(probe.profiles.len(), 2);
    assert!(probe.profiles[1].hidden);
    assert_eq!(
        probe.libraries[2].kind,
        super::super::LibraryType::Unsupported("music".into())
    );
    assert_eq!(
        probe.libraries[3].kind,
        super::super::LibraryType::Collections
    );
    let debug = format!("{probe:?} {provider:?}");
    assert!(!debug.contains("only-test-key"));
    assert!(!debug.contains("localhost"));
    assert_eq!(provider.configuration_revision(), Err(Error::InvalidConfig));
    server.await.unwrap();
}

#[tokio::test]
async fn public_success_or_partial_permissions_never_establish_connection() {
    use tracing::instrument::WithSubscriber;

    #[derive(Clone, Default)]
    struct Log(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for Log {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    for rows in [
        vec![("System/Info", "401 Unauthorized", "secret upstream error")],
        vec![
            ("System/Info", "200 OK", IDENTITY),
            (
                "Library/VirtualFolders",
                "403 Forbidden",
                "secret upstream error",
            ),
        ],
        vec![
            ("System/Info", "200 OK", IDENTITY),
            ("Library/VirtualFolders", "200 OK", LIBRARIES),
            ("Users", "401 Unauthorized", "secret upstream error"),
        ],
    ] {
        let stage = match rows.len() {
            1 => "server_identity",
            2 => "libraries",
            _ => "profiles",
        };
        let (provider, server) = provider(rows).await;
        let log = Log::default();
        let writer = log.clone();
        let subscriber = tracing_subscriber::fmt()
            .json()
            .with_writer(move || writer.clone())
            .finish();
        assert_eq!(
            provider
                .probe_connection()
                .with_subscriber(subscriber)
                .await
                .unwrap_err(),
            Error::Unauthorized
        );
        let bytes = log.0.lock().unwrap();
        let event: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(event["fields"]["stage"], stage);
        assert_eq!(event["fields"]["category"], "unauthorized");
        assert_eq!(event["fields"]["outcome"], "failed");
        assert!(event["fields"]["elapsed_ms"].as_f64().unwrap() >= 0.0);
        let serialized = std::str::from_utf8(&bytes).unwrap();
        for private in [
            "only-test-key",
            "secret upstream error",
            "localhost",
            "server123",
        ] {
            assert!(!serialized.contains(private));
        }
        drop(bytes);
        server.await.unwrap();
    }
}

#[tokio::test]
async fn missing_invalid_identity_and_version_fail_closed() {
    for body in [
        r#"{}"#,
        r#"{"Id":"","ServerName":"Name","Version":"10.11.0"}"#,
        r#"{"Id":"../bad","ServerName":"Name","Version":"10.11.0"}"#,
        r#"{"Id":"server","ServerName":"Name","Version":"bogus"}"#,
        r#"{"Id":"server","ServerName":"","Version":"10.11.0"}"#,
    ] {
        let (provider, server) = provider(vec![("System/Info", "200 OK", body)]).await;
        assert_eq!(
            provider.probe_connection().await.unwrap_err(),
            Error::InvalidServerIdentity
        );
        server.await.unwrap();
    }
}

#[tokio::test]
async fn ambiguous_or_malformed_discovery_is_not_success() {
    for rows in [
        vec![
            ("System/Info", "200 OK", IDENTITY),
            (
                "Library/VirtualFolders",
                "200 OK",
                r#"[{"ItemId":"same","Name":"A","CollectionType":"movies"},{"ItemId":"same","Name":"B","CollectionType":"tvshows"}]"#,
            ),
            ("Users", "200 OK", "[]"),
        ],
        vec![
            ("System/Info", "200 OK", IDENTITY),
            ("Library/VirtualFolders", "200 OK", "[]"),
            (
                "Users",
                "200 OK",
                r#"[{"Id":"same","Name":"A","Policy":{"IsDisabled":false,"IsHidden":false}},{"Id":"same","Name":"B","Policy":{"IsDisabled":false,"IsHidden":false}}]"#,
            ),
        ],
        vec![
            ("System/Info", "200 OK", IDENTITY),
            ("Library/VirtualFolders", "200 OK", r#"[{"Name":"No ID"}]"#),
        ],
    ] {
        let (provider, server) = provider(rows).await;
        assert_eq!(
            provider.probe_connection().await.unwrap_err(),
            Error::InvalidResponse
        );
        server.await.unwrap();
    }
}

#[test]
fn key_bounds_character_rules_redaction_and_url_policy() {
    let key = ApiKey::new("very-private-api-key".into()).unwrap();
    assert!(!format!("{key:?}").contains("very-private"));
    assert!(ApiKey::new("x".repeat(4096)).is_ok());
    for key in [
        String::new(),
        "x".repeat(4097),
        "key\n".into(),
        "key\"".into(),
        "key\\".into(),
        "clé".into(),
    ] {
        assert_eq!(ApiKey::new(key).unwrap_err(), Error::Secret);
    }
    for base in [
        "http://@localhost",
        "http://user:pass@localhost",
        "http://localhost/?key=x",
        "http://localhost/#x",
        "http://localhost:0/",
        "http://localhost:/",
        "http://localhost:65536/",
        "http://[::1",
        "http://-bad/",
        "http://bad_.test/",
        "http://bad..test/",
        "http://localhost\\evil/",
        "http:localhost",
        " http://localhost",
        "http://localhost/\n",
        "http://localhost/base%00",
        "http://localhost/base%0A",
    ] {
        assert_eq!(
            LiveJellyfin::from_api_key(config(base.into()), ApiKey::new("key".into()).unwrap())
                .unwrap_err(),
            Error::InvalidConfig,
            "{base}"
        );
    }
    for base in [
        "http://0.0.0.0",
        "http://169.254.169.254/",
        "http://[::ffff:169.254.169.254]/",
        "http://224.0.0.1",
        "http://255.255.255.255",
    ] {
        assert_eq!(
            LiveJellyfin::from_api_key(config(base.into()), ApiKey::new("key".into()).unwrap())
                .unwrap_err(),
            Error::DestinationRejected
        );
    }
    for base in [
        "http://10.1.2.3/base",
        "http://192.168.1.4",
        "http://[fd00::1]/",
        "http://localhost/base",
        "https://jellyfin.lan",
    ] {
        let config = config(base.into());
        assert!(!format!("{config:?}").contains(base));
        assert!(LiveJellyfin::from_api_key(config, ApiKey::new("key".into()).unwrap()).is_ok());
    }
}

#[test]
fn environment_proxy_configuration_is_ignored() {
    const CHILD: &str = "SCREENROOM_ADAPTER_PROXY_TEST_CHILD";
    if std::env::var_os(CHILD).is_some() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let (provider, server) = provider(vec![("System/Info", "200 OK", "{}")]).await;
            assert_eq!(
                provider.probe_connection().await.unwrap_err(),
                Error::InvalidServerIdentity
            );
            server.await.unwrap();
        });
        return;
    }
    let mut child = std::process::Command::new(std::env::current_exe().unwrap());
    child
        .args([
            "--exact",
            "jellyfin::connection::tests::environment_proxy_configuration_is_ignored",
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
