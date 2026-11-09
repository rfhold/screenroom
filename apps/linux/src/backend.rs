use crate::{
    assets::Pixels,
    catalog::{self, Catalog},
};
use screenroom_client_core::{self as core, Client, ClientConfig, Page};
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, OnceLock},
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
    Client::new(ClientConfig {
        endpoint: std::env::var("SCREENROOM_ENDPOINT").unwrap_or("http://127.0.0.1:50051".into()),
        mock_profile: std::env::var("SCREENROOM_MOCK_PROFILE").unwrap_or("linux-demo".into()),
        timeout_ms: 3000,
    })
}
pub async fn paginate<T, F, Fut>(mut fetch: F) -> Result<Vec<T>, core::ClientError>
where
    F: FnMut(Page) -> Fut,
    Fut: std::future::Future<Output = Result<(Vec<T>, String), core::ClientError>>,
{
    let mut records = Vec::new();
    let mut token = String::new();
    let mut seen = HashSet::new();
    loop {
        let (page, next) = fetch(Page { size: 100, token }).await?;
        records.extend(page);
        if next.is_empty() {
            return Ok(records);
        }
        if !seen.insert(next.clone()) {
            return Err(core::ClientError::InvalidResponse {
                reason: "repeated page token".into(),
            });
        }
        token = next;
    }
}
pub struct Snapshot {
    pub catalog: Catalog,
    pub art: HashMap<String, Pixels>,
    pub preferences: core::Preferences,
    pub capabilities: core::SearchCapabilities,
}
pub async fn load(client: Arc<Client>) -> Result<Snapshot, core::ClientError> {
    let libraries = client.clone().list_libraries().await?;
    eprintln!("core ListLibraries: {} libraries", libraries.len());
    let capabilities = client.clone().get_search_capabilities().await?;
    eprintln!("core GetSearchCapabilities: {:?}", capabilities.methods);
    let preferences = client.clone().get_preferences().await?;
    eprintln!(
        "core GetPreferences: subtitles={}, types={:?}",
        preferences.subtitle_preference, preferences.search_types
    );
    let info = client.clone().get_catalog_info().await?;
    eprintln!(
        "core GetCatalogInfo: {} artwork, {} notices, retrieved {}",
        info.artwork.len(),
        info.notices.len(),
        info.retrieved_at
    );
    let mut data = Catalog {
        art: info.artwork.iter().map(catalog::art_record).collect(),
        info,
        libraries: libraries.clone(),
        ..Catalog::default()
    };
    let mut queue = Vec::new();
    for library in libraries {
        let rows = client.clone().get_home(library.id.clone()).await?;
        eprintln!("core GetHome({}): {} rows", library.id, rows.len());
        data.homes.insert(
            library.id.clone(),
            rows.into_iter()
                .map(|r| {
                    (
                        r.title,
                        r.items
                            .into_iter()
                            .map(|i| (i.id, i.progress_percent.map(|p| p as u8)))
                            .collect(),
                    )
                })
                .collect(),
        );
        let items = paginate(|page| {
            let client = client.clone();
            let id = library.id.clone();
            async move {
                let response = client.list_library_items(id.clone(), page).await?;
                eprintln!(
                    "core ListLibraryItems({id}): {} items",
                    response.items.len()
                );
                Ok((response.items, response.next_page_token))
            }
        })
        .await?;
        data.members
            .insert(library.id, items.iter().map(|i| i.id.clone()).collect());
        queue.extend(items);
    }
    let mut seen = HashSet::new();
    while let Some(summary) = queue.pop() {
        if !seen.insert(summary.id.clone()) {
            continue;
        }
        let item = client.clone().get_item(summary.id.clone()).await?;
        eprintln!("core GetItem: {}", item.id);
        if item.kind == "Series" || item.kind == "Season" {
            let children = paginate(|page| {
                let client = client.clone();
                let id = item.id.clone();
                async move {
                    let response = client.list_children(id.clone(), page).await?;
                    eprintln!("core ListChildren({id}): {} items", response.items.len());
                    Ok((response.items, response.next_page_token))
                }
            })
            .await?;
            data.children.insert(
                item.id.clone(),
                children.iter().map(|i| i.id.clone()).collect(),
            );
            queue.extend(children);
        }
        let credits = paginate(|page| {
            let client = client.clone();
            let id = item.id.clone();
            async move {
                let response = client.list_credits(id.clone(), page).await?;
                eprintln!("core ListCredits({id}): {} credits", response.credits.len());
                Ok((response.credits, response.next_page_token))
            }
        })
        .await?;
        for art in item
            .artwork
            .iter()
            .chain(credits.iter().filter_map(|c| c.artwork.as_ref()))
        {
            if !data.art.iter().any(|a| a["file"] == art.url) {
                data.art.push(catalog::art_record(art));
            }
        }
        data.items.push(catalog::map_item(item, credits));
    }
    data.items.reverse();
    let art = load_artwork_records(&data.art, |url| client.clone().load_artwork(url)).await?;
    eprintln!(
        "Core catalog ready: {} items, {} decoded artwork records",
        data.items.len(),
        art.len()
    );
    Ok(Snapshot {
        catalog: data,
        art,
        preferences,
        capabilities,
    })
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
        // Honor task aborts even when every core cache read is immediately ready.
        tokio::task::yield_now().await;
        let url = catalog::text(record, "file").to_owned();
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
    fn artwork_records() -> Vec<serde_json::Value> {
        ["first", "second"]
            .map(|file| serde_json::json!({"file":file}))
            .to_vec()
    }
    #[tokio::test]
    async fn cancelled_artwork_stops_before_the_next_request() {
        let mut requests = 0;
        let result = load_artwork_records(&artwork_records(), |_| {
            requests += 1;
            std::future::ready(Err(core::ClientError::Cancelled))
        })
        .await;
        assert!(matches!(result, Err(core::ClientError::Cancelled)));
        assert_eq!(requests, 1);
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
        tokio::time::timeout(std::time::Duration::from_secs(1), entered_rx)
            .await
            .unwrap()
            .unwrap();
        task.abort_handle().abort();
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
    async fn continuation_failure_does_not_report_partial_success() {
        let error = paginate(|page| {
            std::future::ready(if page.token.is_empty() {
                Ok((vec![1], "next".into()))
            } else {
                Err(core::ClientError::Unavailable {
                    reason: "second page".into(),
                })
            })
        })
        .await
        .unwrap_err();
        assert!(matches!(error, core::ClientError::Unavailable { .. }));
    }
    #[tokio::test]
    async fn repeated_continuation_fails_without_an_infinite_loop() {
        let mut requests = 0;
        let error = paginate(|_| {
            requests += 1;
            std::future::ready(Ok((vec![1], "repeat".into())))
        })
        .await
        .unwrap_err();
        assert!(matches!(error, core::ClientError::InvalidResponse { .. }));
        assert_eq!(requests, 2);
    }
}
