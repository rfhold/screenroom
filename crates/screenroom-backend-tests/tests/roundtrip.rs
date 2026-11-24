use screenroom_client_core::{Client, ClientConfig, ClientError, Page, SourceLibraryScope};
use std::{
    collections::BTreeSet,
    future::Future,
    task::Poll,
    time::{Duration, Instant},
};
mod support;

const DEADLINE: Duration = Duration::from_secs(120);
const PAGE_SIZE: u32 = 100;

fn config() -> ClientConfig {
    let endpoint =
        std::env::var("SCREENROOM_API_ENDPOINT").expect("set SCREENROOM_API_ENDPOINT explicitly");
    let endpoint = endpoint.trim_end_matches('/').to_owned();
    endpoint
        .strip_prefix("http://127.0.0.1:")
        .and_then(|port| port.parse::<u16>().ok())
        .filter(|port| *port >= 1024)
        .expect("transport contract requires explicit http://127.0.0.1:<port> origin");
    let user_id = std::env::var("SCREENROOM_USER_ID")
        .expect("set SCREENROOM_USER_ID to an existing enabled local user");
    assert!(!user_id.is_empty(), "SCREENROOM_USER_ID must not be empty");
    ClientConfig {
        endpoint,
        user_id,
        timeout_ms: 15000,
    }
}

#[tokio::test]
#[ignore = "attached empty installation only; explicit SCREENROOM_API_ENDPOINT and SCREENROOM_USER_ID; no configured sources"]
async fn empty_installation_native_core_roundtrip() {
    tokio::time::timeout(DEADLINE, empty_roundtrip())
        .await
        .expect("whole empty roundtrip deadline");
}

#[tokio::test]
#[ignore = "attached indexed installation only; explicit SCREENROOM_API_ENDPOINT and SCREENROOM_USER_ID; managed source with indexed items and published artwork"]
async fn indexed_source_native_core_roundtrip() {
    tokio::time::timeout(DEADLINE, indexed_roundtrip())
        .await
        .expect("whole indexed roundtrip deadline");
}

async fn empty_roundtrip() {
    let started = Instant::now();
    let configuration = config();
    let client = Client::new(configuration.clone()).unwrap();
    assert_eq!(
        client.clone().get_current_user().await.unwrap().id,
        configuration.user_id
    );
    let info = client.clone().get_catalog_info().await.unwrap();
    assert!(
        info.branding.is_empty(),
        "empty installation must not invent catalog branding"
    );
    let sources = client
        .clone()
        .list_jellyfin_sources(Page {
            size: PAGE_SIZE,
            token: String::new(),
        })
        .await
        .unwrap();
    assert!(
        sources.sources.is_empty() && sources.next_page_token.is_empty(),
        "empty prerequisite: no managed connections"
    );
    let libraries = client
        .clone()
        .list_libraries(
            String::new(),
            Page {
                size: PAGE_SIZE,
                token: String::new(),
            },
        )
        .await
        .unwrap();
    assert!(
        libraries.libraries.is_empty() && libraries.next_page_token.is_empty(),
        "empty prerequisite: no libraries"
    );
    let scope = SourceLibraryScope::default();
    let items = client
        .clone()
        .list_library_items(
            scope.clone(),
            Page {
                size: PAGE_SIZE,
                token: String::new(),
            },
        )
        .await
        .unwrap();
    assert!(
        items.items.is_empty() && items.next_page_token.is_empty(),
        "empty prerequisite: no indexed items"
    );
    let artwork = client
        .clone()
        .list_artwork(
            scope.clone(),
            Page {
                size: PAGE_SIZE,
                token: String::new(),
            },
        )
        .await
        .unwrap();
    assert!(
        artwork.artwork.is_empty() && artwork.next_page_token.is_empty(),
        "empty installation must not invent artwork"
    );
    let home = client.clone().get_home(scope).await.unwrap();
    assert!(
        home.iter()
            .all(|row| row.items.is_empty() && row.read_error.is_none()),
        "empty catalog must remain readable without imaginary items or row errors"
    );
    cancellation_and_invalidation(&configuration).await;
    contract(configuration, started).await;
}

async fn indexed_roundtrip() {
    let started = Instant::now();
    let configuration = config();
    let client = Client::new(configuration.clone()).unwrap();
    assert_eq!(
        client.clone().get_current_user().await.unwrap().id,
        configuration.user_id
    );
    client.clone().get_catalog_info().await.unwrap();
    let sources = support::pages(
        |token| {
            let client = client.clone();
            async move {
                let page = client
                    .list_jellyfin_sources(Page {
                        size: PAGE_SIZE,
                        token,
                    })
                    .await
                    .map_err(|e| format!("{e:?}"))?;
                assert!(page.sources.len() <= PAGE_SIZE as usize);
                Ok((page.sources, page.next_page_token))
            }
        },
        512,
        10000,
        DEADLINE,
    )
    .await
    .unwrap();
    let mut source_ids = BTreeSet::new();
    for source in &sources {
        assert!(
            !source.id.is_empty() && source_ids.insert(source.id.clone()),
            "duplicate/empty source ID"
        );
    }
    let mut chosen = None;
    'sources: for source in sources {
        let libraries = support::pages(
            |token| {
                let (client, source) = (client.clone(), source.id.clone());
                async move {
                    let page = client
                        .list_libraries(
                            source,
                            Page {
                                size: PAGE_SIZE,
                                token,
                            },
                        )
                        .await
                        .map_err(|e| format!("{e:?}"))?;
                    assert!(page.libraries.len() <= PAGE_SIZE as usize);
                    Ok((page.libraries, page.next_page_token))
                }
            },
            512,
            10000,
            DEADLINE,
        )
        .await
        .unwrap();
        let mut library_ids = BTreeSet::new();
        for library in libraries {
            assert!(
                !library.id.is_empty() && library_ids.insert(library.id.clone()),
                "duplicate/empty library ID"
            );
            let scope = SourceLibraryScope {
                source_id: source.id.clone(),
                library_id: library.id,
            };
            let items = support::pages(
                |token| {
                    let (client, scope) = (client.clone(), scope.clone());
                    async move {
                        let page = client
                            .list_library_items(
                                scope,
                                Page {
                                    size: PAGE_SIZE,
                                    token,
                                },
                            )
                            .await
                            .map_err(|e| format!("{e:?}"))?;
                        assert!(page.items.len() <= PAGE_SIZE as usize);
                        Ok((page.items, page.next_page_token))
                    }
                },
                512,
                10000,
                DEADLINE,
            )
            .await
            .unwrap();
            let mut item_ids = BTreeSet::new();
            for item in &items {
                assert!(
                    !item.id.is_empty() && item_ids.insert(item.id.clone()),
                    "duplicate/empty item ID across pages"
                );
            }
            if let Some(item) = items.into_iter().find(|item| item.artwork.is_some()) {
                chosen = Some((scope, item));
                break 'sources;
            }
        }
    }
    let (scope, listed) = chosen.expect("indexed prerequisite: save a managed connection, complete canonical metadata/artwork indexing, and publish artwork for an item in a library within the 10,000-item traversal budget");
    let item = client
        .clone()
        .get_item(listed.id.clone(), scope.clone())
        .await
        .unwrap();
    assert_eq!(
        (&item.id, &item.title, &item.kind),
        (&listed.id, &listed.title, &listed.kind)
    );
    let home = client.clone().get_home(scope.clone()).await.unwrap();
    assert!(
        home.iter().all(|row| row.read_error.is_none()),
        "indexed home rows must be readable"
    );
    let image = item
        .artwork
        .clone()
        .expect("selected indexed item must retain published artwork");
    let bytes = client
        .clone()
        .load_artwork(image.url.clone())
        .await
        .unwrap();
    assert!(!bytes.is_empty(), "published artwork must have bytes");
    assert_eq!(
        bytes,
        client
            .clone()
            .load_artwork(image.url.clone())
            .await
            .unwrap()
    );
    let artwork = support::pages(
        |token| {
            let (client, scope) = (client.clone(), scope.clone());
            async move {
                let page = client
                    .list_artwork(
                        scope,
                        Page {
                            size: PAGE_SIZE,
                            token,
                        },
                    )
                    .await
                    .map_err(|e| format!("{e:?}"))?;
                assert!(page.artwork.len() <= PAGE_SIZE as usize);
                Ok((page.artwork, page.next_page_token))
            }
        },
        512,
        10000,
        DEADLINE,
    )
    .await
    .unwrap();
    let mut artwork_ids = BTreeSet::new();
    assert!(
        artwork
            .iter()
            .any(|art| art.id == image.id && art.url == image.url),
        "item artwork must appear in the same scoped artwork listing"
    );
    for art in artwork {
        assert!(
            !art.id.is_empty() && artwork_ids.insert(art.id),
            "duplicate/empty artwork ID across pages"
        );
    }
    client.invalidate_reads();
    let fresh = client
        .clone()
        .get_item(item.id.clone(), scope)
        .await
        .unwrap();
    assert_eq!(
        (&fresh.id, &fresh.title, &fresh.kind),
        (&item.id, &item.title, &item.kind)
    );
    assert_eq!(
        bytes,
        client.clone().load_artwork(image.url).await.unwrap(),
        "invalidation must permit fresh artwork reads"
    );
    cancellation_and_invalidation(&configuration).await;
    contract(configuration, started).await;
}

async fn cancellation_and_invalidation(configuration: &ClientConfig) {
    let client = Client::new(configuration.clone()).unwrap();
    cancel_pending_read(&client).await;
    let before = client.clone().get_catalog_info().await.unwrap();
    client.invalidate_reads();
    let after = client.get_catalog_info().await.unwrap();
    assert_eq!(before.notices, after.notices);
    assert_eq!(before.branding, after.branding);
}

async fn cancel_pending_read(client: &std::sync::Arc<Client>) {
    let pending = client.clone().get_catalog_info();
    tokio::pin!(pending);
    std::future::poll_fn(|cx| {
        assert!(
            matches!(pending.as_mut().poll(cx), Poll::Pending),
            "fresh native read must be pending before cancellation"
        );
        Poll::Ready(())
    })
    .await;
    client.cancel_all();
    assert!(
        matches!(pending.await, Err(ClientError::Cancelled)),
        "pending read must observe cancellation"
    );
}

#[tokio::test]
async fn pending_native_read_observes_cancellation_without_an_attached_service() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let client = Client::new(ClientConfig {
        endpoint: format!("http://{}", listener.local_addr().unwrap()),
        user_id: String::new(),
        timeout_ms: 1000,
    })
    .unwrap();
    tokio::time::timeout(Duration::from_secs(2), cancel_pending_read(&client))
        .await
        .expect("cancelled read must finish without a server response");
}

async fn contract(configuration: ClientConfig, started: Instant) {
    tokio::task::spawn_blocking(move || {
        let budget = DEADLINE
            .saturating_sub(started.elapsed())
            .saturating_sub(Duration::from_secs(1))
            .min(Duration::from_secs(60));
        assert!(!budget.is_zero(), "no remaining subprocess contract budget");
        let mut command = std::process::Command::new("bash");
        command
            .args([
                "scripts/backend-contract.sh",
                &configuration.endpoint,
                &configuration.user_id,
            ])
            .current_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."));
        support::process(command, budget, 1024 * 1024)
    })
    .await
    .unwrap()
    .unwrap();
}
