#![cfg(not(target_arch = "wasm32"))]

mod support;
use screenroom_client_core::*;

fn client(endpoint: &str, user_id: &str, timeout_ms: u32) -> std::sync::Arc<Client> {
    Client::new(ClientConfig {
        endpoint: endpoint.into(),
        user_id: user_id.into(),
        timeout_ms,
    })
    .unwrap()
}
const ALICE: &str = "00000000-0000-0000-0000-000000000001";
const BOB: &str = "00000000-0000-0000-0000-000000000002";

#[tokio::test]
async fn cache_dedup_invalidation_context_deadline_failure_and_cancellation() {
    let (endpoint, counts, task) = support::start().await;
    let c = client(&endpoint, ALICE, 3000);
    let (a, b) = tokio::join!(c.clone().get_catalog_info(), c.clone().get_catalog_info());
    assert_eq!(a.unwrap(), b.unwrap());
    c.clone().get_catalog_info().await.unwrap();
    assert_eq!(counts.lock().unwrap()[&(ALICE.into(), "info".into())], 1);
    c.invalidate_reads();
    c.clone().get_catalog_info().await.unwrap();
    assert_eq!(counts.lock().unwrap()[&(ALICE.into(), "info".into())], 2);
    let bob = client(&endpoint, BOB, 3000);
    bob.get_catalog_info().await.unwrap();
    assert_eq!(counts.lock().unwrap()[&(BOB.into(), "info".into())], 1);
    c.invalidate_reads();
    let mut pending = Box::pin(c.clone().get_catalog_info());
    assert!(futures::poll!(pending.as_mut()).is_pending());
    c.cancel_all();
    assert!(matches!(pending.await, Err(ClientError::Cancelled)));
    assert!(matches!(
        client(&endpoint, ALICE, 10).get_catalog_info().await,
        Err(ClientError::DeadlineExceeded)
    ));
    let anonymous = client(&endpoint, "", 3000);
    assert!(
        anonymous
            .clone()
            .get_home(SourceLibraryScope::default())
            .await
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        anonymous.get_current_user().await,
        Err(ClientError::InvalidArgument { .. })
    ));
    let missing = c
        .clone()
        .get_item("missing".into(), SourceLibraryScope::default())
        .await;
    assert!(matches!(missing, Err(ClientError::NotFound { .. })));
    c.get_item("missing".into(), SourceLibraryScope::default())
        .await
        .unwrap_err();
    assert_eq!(
        counts.lock().unwrap()[&(ALICE.into(), "item".into())],
        2,
        "failures must not enter read cache"
    );
    task.abort();
    let socket = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", socket.local_addr().unwrap());
    drop(socket);
    assert!(matches!(
        client(&endpoint, ALICE, 1000).get_catalog_info().await,
        Err(ClientError::Unavailable { .. })
    ));
}

#[test]
fn explicit_local_context_and_client_owned_settings() {
    for user_id in [
        "alice",
        "test-profile",
        "00000000000000000000000000000001",
        "00000000-0000-0000-0000-00000000000g",
    ] {
        assert!(
            Client::new(ClientConfig {
                endpoint: "http://127.0.0.1:50051".into(),
                user_id: user_id.into(),
                timeout_ms: 1000
            })
            .is_err()
        );
    }
    for endpoint in [
        "http://user@localhost:50051",
        "http://localhost:50051/path",
        "http://localhost:50051?x",
        "http://localhost:50051#x",
        "http://localhost:50051/",
    ] {
        assert!(
            Client::new(ClientConfig {
                endpoint: endpoint.into(),
                user_id: ALICE.into(),
                timeout_ms: 1000
            })
            .is_err()
        );
    }
    let defaults = default_settings();
    assert_eq!(
        defaults.scope.as_ref().unwrap(),
        &SourceLibraryScope::default()
    );
    assert!(defaults.types.as_ref().unwrap().episode);
    assert_eq!(validate_settings(defaults.clone()).unwrap(), defaults);
    for value in ["all", "jellyfin"] {
        let mut settings = defaults.clone();
        settings.source = value.into();
        assert!(validate_settings(settings).is_ok());
    }
    for value in ["tmdb", "unknown", ""] {
        let mut invalid = defaults.clone();
        invalid.source = value.into();
        assert!(validate_settings(invalid).is_err());
    }
    let mut invalid = defaults.clone();
    invalid.types = Some(SearchTypes::default());
    assert!(validate_settings(invalid).is_err());
    let mut invalid = defaults;
    invalid.methods = Some(SearchMethods {
        title_only: true,
        semantic: true,
        ..Default::default()
    });
    assert!(validate_settings(invalid).is_err());
}

#[tokio::test]
async fn artwork_origin_path_and_credentials_rejected_before_network() {
    let c = client("http://127.0.0.1:50051", ALICE, 10);
    for url in [
        "http://user@127.0.0.1:50051/artwork/00000000-0000-0000-0000-000000000001/revisions/1/native",
        "http://other.test/artwork/00000000-0000-0000-0000-000000000001/revisions/1/native",
        "http://127.0.0.1:50051/artwork/file.jpg",
        "http://127.0.0.1:50051/artwork/00000000-0000-0000-0000-000000000001/revisions/0/native",
        "http://127.0.0.1:50051/artwork/00000000-0000-0000-0000-000000000001/revisions/1/native?x",
        "http://127.0.0.1:50051/artwork/00000000-0000-0000-0000-000000000001/revisions/1/native#x",
    ] {
        assert!(
            matches!(
                c.clone().load_artwork(url.into()).await,
                Err(ClientError::InvalidArgument { .. })
            ),
            "{url}"
        );
    }
}
