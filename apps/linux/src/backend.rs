use crate::{
    assets::Pixels,
    catalog::{self, Catalog},
    state::Screen,
};
use screenroom_client_core::{self as core, Client, ClientConfig, Page, SourceLibraryScope};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, OnceLock},
    time::Duration,
};

pub fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("Tokio runtime")
    })
}
pub fn client() -> Result<Arc<Client>, core::ClientError> {
    let endpoint =
        std::env::var("SCREENROOM_API_ENDPOINT").unwrap_or("http://127.0.0.1:50051".into());
    for alias in ["SCREENROOM_ENDPOINT", "SCREENROOM_E2E_API_ENDPOINT"] {
        if std::env::var(alias).is_ok_and(|value| value != endpoint) {
            return Err(core::ClientError::InvalidResponse {
                reason: format!("{alias} conflicts with SCREENROOM_API_ENDPOINT"),
            });
        }
    }
    Client::new(ClientConfig {
        endpoint,
        user_id: std::env::var("SCREENROOM_USER_ID").unwrap_or_default(),
        timeout_ms: 3000,
    })
}
fn invalid(reason: &str) -> core::ClientError {
    core::ClientError::InvalidResponse {
        reason: reason.into(),
    }
}
pub async fn operation<T>(
    work: impl std::future::Future<Output = Result<T, core::ClientError>>,
) -> Result<T, core::ClientError> {
    let budget = Duration::from_secs(120);
    #[cfg(feature = "e2e")]
    let budget = std::env::var("SCREENROOM_E2E_OPERATION_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| (1..=120_000).contains(value))
        .map(Duration::from_millis)
        .unwrap_or(budget);
    operation_until(tokio::time::Instant::now() + budget, work).await
}
async fn operation_until<T>(
    deadline: tokio::time::Instant,
    work: impl std::future::Future<Output = Result<T, core::ClientError>>,
) -> Result<T, core::ClientError> {
    tokio::time::timeout_at(deadline, work)
        .await
        .map_err(|_| invalid("operation deadline"))?
}
pub async fn paginate<T, F, Fut>(mut fetch: F) -> Result<Vec<T>, core::ClientError>
where
    F: FnMut(Page) -> Fut,
    Fut: std::future::Future<Output = Result<(Vec<T>, String), core::ClientError>>,
{
    tokio::time::timeout(Duration::from_secs(120), async {
        let mut records = Vec::new();
        let mut token = String::new();
        let mut seen = HashSet::new();
        for _ in 0..512 {
            let (page, next) = fetch(Page { size: 100, token }).await?;
            records.extend(page);
            if records.len() > 10_000 {
                return Err(invalid("pagination item limit"));
            }
            if next.is_empty() {
                return Ok(records);
            }
            if !seen.insert(next.clone()) {
                return Err(invalid("repeated page token"));
            }
            token = next;
        }
        Err(invalid("pagination page limit"))
    })
    .await
    .map_err(|_| invalid("pagination deadline"))?
}
pub struct Snapshot {
    pub catalog: Catalog,
    pub art: HashMap<String, Pixels>,
    pub capabilities: core::SearchCapabilities,
}
pub async fn load(
    client: Arc<Client>,
    scope: SourceLibraryScope,
    screen: Screen,
    target: Option<String>,
    detail: bool,
) -> Result<Snapshot, core::ClientError> {
    operation(load_snapshot(client, scope, screen, target, detail)).await
}
async fn load_snapshot(
    client: Arc<Client>,
    scope: SourceLibraryScope,
    screen: Screen,
    target: Option<String>,
    detail: bool,
) -> Result<Snapshot, core::ClientError> {
    let libraries = paginate(|page| {
        let client = client.clone();
        let source = scope.source_id.clone();
        async move {
            let r = client.list_libraries(source, page).await?;
            Ok((r.libraries, r.next_page_token))
        }
    })
    .await?;
    let capabilities = client.clone().get_search_capabilities().await?;
    let info = client.clone().get_catalog_info().await?;
    eprintln!(
        "core GetCatalogInfo: {} branding, {} notices",
        info.branding.len(),
        info.notices.len()
    );
    let mut data = Catalog {
        libraries,
        info,
        ..Default::default()
    };
    let items;
    if let Some(id) = target {
        let item = client.clone().get_item(id.clone(), scope.clone()).await?;
        let credits = if detail {
            paginate(|page| {
                let client = client.clone();
                let id = id.clone();
                let scope = scope.clone();
                async move {
                    let r = client.list_credits(id, page, scope).await?;
                    Ok((r.credits, r.next_page_token))
                }
            })
            .await?
        } else {
            Vec::new()
        };
        let mut parent = item.parent_id.clone();
        data.items.push(catalog::map_item(item, credits));
        let mut ancestors = HashSet::new();
        while let Some(id) = parent {
            if ancestors.len() >= 16 || !ancestors.insert(id.clone()) {
                return Err(invalid("invalid hierarchy"));
            }
            let ancestor = client.clone().get_item(id, scope.clone()).await?;
            parent = ancestor.parent_id.clone();
            data.items.push(catalog::map_item(ancestor, Vec::new()));
        }
        items = if detail {
            Vec::new()
        } else {
            let children = paginate(|page| {
                let client = client.clone();
                let id = id.clone();
                let scope = scope.clone();
                async move {
                    let r = client.list_children(id, page, scope).await?;
                    Ok((r.items, r.next_page_token))
                }
            })
            .await?;
            data.children
                .insert(id, children.iter().map(|i| i.id.clone()).collect());
            children
        };
    } else if screen == Screen::Home {
        let rows = client.clone().get_home(scope.clone()).await?;
        data.homes.insert(
            scope.library_id.clone(),
            rows.iter()
                .map(|r| {
                    (
                        r.title.clone(),
                        r.items
                            .iter()
                            .map(|i| (i.id.clone(), i.progress_percent.map(|p| p as u8)))
                            .collect(),
                    )
                })
                .collect(),
        );
        items = rows.into_iter().flat_map(|r| r.items).collect();
    } else if screen == Screen::Browse {
        items = paginate(|page| {
            let client = client.clone();
            let scope = scope.clone();
            async move {
                let r = client.list_library_items(scope, page).await?;
                Ok((r.items, r.next_page_token))
            }
        })
        .await?;
        data.members.insert(
            scope.library_id.clone(),
            items
                .iter()
                .filter(|i| i.parent_id.is_none())
                .map(|i| i.id.clone())
                .collect(),
        );
    } else {
        items = Vec::new();
    }
    for item in items {
        if !data.items.iter().any(|i| i.id == item.id) {
            data.items.push(catalog::map_item(item, Vec::new()));
        }
    }
    let records = data
        .items
        .iter()
        .filter_map(|i| i.art.clone())
        .chain(data.items.iter().flat_map(|i| {
            ["cast", "crew"].into_iter().flat_map(move |key| {
                i.metadata[key]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(catalog::person_art)
            })
        }))
        .chain(data.info.branding.iter().map(catalog::art_record))
        .collect::<Vec<_>>();
    let art = load_artwork_records(&records, |url| client.clone().load_artwork(url)).await?;
    eprintln!(
        "Core catalog ready: {} items, {} decoded artwork records",
        data.items.len(),
        art.len()
    );
    Ok(Snapshot {
        catalog: data,
        art,
        capabilities,
    })
}
pub async fn artwork_credits(
    client: Arc<Client>,
    scope: SourceLibraryScope,
) -> Result<Vec<serde_json::Value>, core::ClientError> {
    let records = paginate(|page| {
        let client = client.clone();
        let scope = scope.clone();
        async move {
            let r = client.list_artwork(scope, page).await?;
            Ok((r.artwork, r.next_page_token))
        }
    })
    .await?;
    eprintln!(
        "core ListArtwork complete: {} source artwork records",
        records.len()
    );
    Ok(records.iter().map(catalog::art_record).collect())
}
pub async fn search_artwork(
    client: Arc<Client>,
    items: &[core::Item],
) -> Result<HashMap<String, Pixels>, core::ClientError> {
    let records = items
        .iter()
        .filter_map(|item| item.artwork.as_ref().map(catalog::art_record))
        .collect::<Vec<_>>();
    load_artwork_records(&records, |url| client.clone().load_artwork(url)).await
}
async fn load_artwork_records<F, Fut>(
    records: &[serde_json::Value],
    mut fetch: F,
) -> Result<HashMap<String, Pixels>, core::ClientError>
where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<u8>, core::ClientError>>,
{
    let mut art = HashMap::new();
    for record in records {
        tokio::task::yield_now().await;
        let url = catalog::text(record, "file").to_owned();
        if art.contains_key(&url) {
            continue;
        }
        match fetch(url.clone()).await {
            Ok(bytes) => {
                eprintln!("core LoadArtwork: {url}, {} bytes", bytes.len());
                match image::load_from_memory(&bytes) {
                    Ok(image) => {
                        let image = image.to_rgba8();
                        art.insert(
                            url,
                            Pixels {
                                width: image.width() as i32,
                                height: image.height() as i32,
                                data: image.into_raw(),
                            },
                        );
                    }
                    Err(error) => eprintln!("Artwork unavailable {url}: {error}"),
                }
            }
            Err(core::ClientError::Cancelled) => return Err(core::ClientError::Cancelled),
            Err(error) => eprintln!("Artwork unavailable {url}: {error}"),
        }
    }
    Ok(art)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn whole_operation_budget_covers_metadata_and_unique_image_bodies() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(1, 1)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let observed = requests.clone();
        let server = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                observed.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(80)).await;
                let _ = stream.write_all(png.get_ref()).await;
            }
        });
        let (dropped, closed) = tokio::sync::oneshot::channel::<()>();
        let mut dropped = Some(dropped);
        let mut index = 0;
        let started = tokio::time::Instant::now();
        let result = operation_until(started + Duration::from_millis(190), async {
            tokio::time::sleep(Duration::from_millis(80)).await;
            let records = ["one", "two", "three"].map(|file| serde_json::json!({"file":file}));
            load_artwork_records(&records, |_| {
                index += 1;
                let dropped = if index == 2 { dropped.take() } else { None };
                async move {
                    let _held = dropped;
                    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
                    let mut body = Vec::new();
                    stream.read_to_end(&mut body).await.unwrap();
                    Ok(body)
                }
            })
            .await
        })
        .await;
        assert!(
            result
                .err()
                .unwrap()
                .to_string()
                .contains("operation deadline")
        );
        assert!(started.elapsed() < Duration::from_millis(350));
        assert_eq!(
            requests.load(Ordering::SeqCst),
            2,
            "later image must not be requested"
        );
        assert!(closed.await.is_err());
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(requests.load(Ordering::SeqCst), 2);
        server.abort();
    }
    fn artwork_records() -> Vec<serde_json::Value> {
        ["first", "second"]
            .map(|file| serde_json::json!({"file":file}))
            .to_vec()
    }
    #[tokio::test]
    async fn aborted_snapshot_drops_pending_artwork_without_more_requests() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let requests = Arc::new(AtomicUsize::new(0));
        let observed = requests.clone();
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let mut entered = Some(entered_tx);
        let mut release = Some(release_rx);
        let task = tokio::spawn(async move {
            load_artwork_records(&artwork_records(), move |_| {
                requests.fetch_add(1, Ordering::SeqCst);
                entered.take().unwrap().send(()).unwrap();
                let release = release.take().unwrap();
                async move {
                    let _ = release.await;
                    Ok(Vec::new())
                }
            })
            .await
        });
        tokio::time::timeout(Duration::from_secs(1), entered_rx)
            .await
            .unwrap()
            .unwrap();
        task.abort();
        assert!(matches!(task.await, Err(error) if error.is_cancelled()));
        assert_eq!(observed.load(Ordering::SeqCst), 1);
        assert!(release_tx.send(()).is_err());
    }
    #[tokio::test]
    async fn missing_artwork_leaves_a_placeholder_and_loads_later_images() {
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(1, 1)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let mut requests = Vec::new();
        let pixels = load_artwork_records(&artwork_records(), |url| {
            requests.push(url.clone());
            std::future::ready(if url == "first" {
                Err(core::ClientError::NotFound {
                    reason: "missing image".into(),
                })
            } else {
                Ok(png.get_ref().clone())
            })
        })
        .await
        .unwrap();
        assert_eq!(requests, ["first", "second"]);
        assert!(!pixels.contains_key("first"));
        assert_eq!(pixels["second"].data.len(), 4);
    }
    #[tokio::test]
    async fn all_continuations_preserve_order_beyond_one_hundred() {
        let mut requests = Vec::new();
        let records = paginate(|page| {
            requests.push(page.token.clone());
            let offset = if page.token.is_empty() {
                0
            } else {
                page.token.parse::<usize>().unwrap()
            };
            let end = (offset + page.size as usize).min(251);
            std::future::ready(Ok((
                (offset..end).collect::<Vec<_>>(),
                if end < 251 {
                    end.to_string()
                } else {
                    String::new()
                },
            )))
        })
        .await
        .unwrap();
        assert_eq!(records, (0..251).collect::<Vec<_>>());
        assert_eq!(requests, ["", "100", "200"]);
    }
    #[tokio::test]
    async fn repeated_token_fails() {
        assert!(
            paginate(|_| std::future::ready(Ok((vec![1], "same".into()))))
                .await
                .is_err()
        );
    }
    #[tokio::test]
    async fn continuation_failure_never_succeeds_partially() {
        assert!(
            paginate(|p| std::future::ready(if p.token.is_empty() {
                Ok((vec![1], "next".into()))
            } else {
                Err(invalid("failed"))
            }))
            .await
            .is_err()
        );
    }
    #[tokio::test]
    async fn page_limit_fails() {
        let mut count = 0;
        assert!(
            paginate::<u8, _, _>(|_| {
                count += 1;
                std::future::ready(Ok((vec![], count.to_string())))
            })
            .await
            .is_err()
        );
        assert_eq!(count, 512);
    }
    #[tokio::test]
    async fn artwork_cancel_stops_requests() {
        let records = vec![
            serde_json::json!({"file":"one"}),
            serde_json::json!({"file":"two"}),
        ];
        let mut count = 0;
        assert!(matches!(
            load_artwork_records(&records, |_| {
                count += 1;
                std::future::ready(Err(core::ClientError::Cancelled))
            })
            .await,
            Err(core::ClientError::Cancelled)
        ));
        assert_eq!(count, 1);
    }
    #[tokio::test]
    async fn unavailable_or_corrupt_artwork_never_gets_fallback_bytes() {
        let records = vec![
            serde_json::json!({"file":"unavailable"}),
            serde_json::json!({"file":"corrupt"}),
        ];
        let art = load_artwork_records(&records, |url| {
            std::future::ready(if url == "unavailable" {
                Err(invalid("unavailable"))
            } else {
                Ok(b"not an image".to_vec())
            })
        })
        .await
        .unwrap();
        assert!(art.is_empty());
    }
}
