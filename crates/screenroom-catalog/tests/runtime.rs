use base64::Engine;
use screenroom_backend::{
    Uuid,
    catalog::{CatalogRepository, CreditWrite, ItemWrite},
    ingest::{CheckpointWrite, Completion, IngestRepository, PageCommit, PolicyWrite},
    sqlx::{self, postgres::PgPoolOptions},
};
use screenroom_catalog::Catalog;
use screenroom_proto::user::internal::v1::{
    ContinueWatchingEntry, GetContinueWatchingRequest, GetContinueWatchingResponse,
    playback_state_service_server::{PlaybackStateService, PlaybackStateServiceServer},
};
use screenroom_proto::{
    catalog::{
        operator::v1::{
            GetPolicyRequest, GetRunRequest, StartRunRequest, UpdatePolicyRequest,
            ingestion_service_client::IngestionServiceClient,
        },
        v1::*,
    },
    common::v1::{SearchMethods, SearchTypes, SourceLibraryScope},
};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio_stream::StreamExt;
struct SlowObject {
    bytes: Vec<u8>,
    stage: u8,
    pause: std::pin::Pin<Box<tokio::time::Sleep>>,
    dropped: Arc<AtomicUsize>,
    truncate: Option<tokio::sync::oneshot::Receiver<()>>,
}
impl Drop for SlowObject {
    fn drop(&mut self) {
        self.dropped.fetch_add(1, Ordering::SeqCst);
    }
}
impl tokio_stream::Stream for SlowObject {
    type Item = Result<Vec<u8>, std::io::Error>;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        match self.stage {
            0 => {
                self.stage = 1;
                std::task::Poll::Ready(Some(Ok(self.bytes[..self.bytes.len() / 2].to_vec())))
            }
            1 => {
                if let Some(gate) = self.truncate.as_mut() {
                    if std::future::Future::poll(std::pin::Pin::new(gate), cx).is_pending() {
                        return std::task::Poll::Pending;
                    }
                    self.stage = 2;
                    return std::task::Poll::Ready(Some(Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "synthetic object truncation",
                    ))));
                }
                if std::future::Future::poll(self.pause.as_mut(), cx).is_pending() {
                    return std::task::Poll::Pending;
                }
                self.stage = 2;
                std::task::Poll::Ready(Some(Ok(self.bytes[self.bytes.len() / 2..].to_vec())))
            }
            _ => std::task::Poll::Ready(None),
        }
    }
}

const ALICE: &str = "00000000-0000-0000-0000-000000000001";
const BOB: &str = "00000000-0000-0000-0000-000000000002";
const UNLINKED: &str = "00000000-0000-0000-0000-000000000003";
#[derive(Clone)]
struct SyntheticProgress {
    source: String,
    unavailable: Arc<AtomicBool>,
}
#[tonic::async_trait]
impl PlaybackStateService for SyntheticProgress {
    async fn get_continue_watching(
        &self,
        request: tonic::Request<GetContinueWatchingRequest>,
    ) -> Result<tonic::Response<GetContinueWatchingResponse>, tonic::Status> {
        if self.unavailable.load(Ordering::SeqCst) {
            return Err(tonic::Status::unavailable("synthetic provider outage"));
        }
        let r = request.into_inner();
        assert_eq!(r.page_size, 50);
        assert!(r.page_token.is_empty());
        assert_eq!(r.scopes.len(), 1);
        assert_eq!(r.scopes[0].source_id, self.source);
        assert_eq!(r.scopes[0].library_id, "movies");
        let entries = match r.user_id.as_str() {
            ALICE => ["item-0", "item-0", "not-indexed"]
                .into_iter()
                .map(|id| ContinueWatchingEntry {
                    source_id: self.source.clone(),
                    external_item_id: id.into(),
                    progress_percent: 42,
                    ..Default::default()
                })
                .collect(),
            BOB => vec![ContinueWatchingEntry {
                source_id: self.source.clone(),
                external_item_id: "item-1".into(),
                progress_percent: 67,
                ..Default::default()
            }],
            UNLINKED => vec![],
            _ => return Err(tonic::Status::not_found("user not enabled")),
        };
        Ok(tonic::Response::new(GetContinueWatchingResponse {
            entries,
            next_page_token: String::new(),
        }))
    }
}
fn home(source: &str, user: &str) -> tonic::Request<GetHomeRequest> {
    let mut request = tonic::Request::new(GetHomeRequest {
        source_id: source.into(),
        library_id: "movies".into(),
    });
    request
        .metadata_mut()
        .insert("x-screenroom-user-id", user.parse().unwrap());
    request
}

async fn http(address: std::net::SocketAddr, method: &str, path: &str, extra: &str) -> Vec<u8> {
    http_body(address, method, path, extra, &[]).await
}
async fn http_body(
    address: std::net::SocketAddr,
    method: &str,
    path: &str,
    extra: &str,
    body: &[u8],
) -> Vec<u8> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
    socket
        .write_all(
            format!(
                "{method} {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n{extra}\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    socket.write_all(body).await.unwrap();
    let mut response = vec![];
    socket
        .read_to_end(&mut response)
        .await
        .unwrap_or_else(|error| {
            panic!(
                "HTTP {method} {path}: {error}; partial response: {:?}",
                String::from_utf8_lossy(&response)
            );
        });
    response
}

fn message_fields(mut bytes: &[u8]) -> Vec<(u32, &[u8])> {
    fn varint(bytes: &mut &[u8]) -> u64 {
        let mut value = 0;
        for shift in (0..64).step_by(7) {
            let byte = bytes[0];
            *bytes = &bytes[1..];
            value |= u64::from(byte & 127) << shift;
            if byte & 128 == 0 {
                return value;
            }
        }
        panic!("invalid protobuf varint");
    }
    let mut fields = vec![];
    while !bytes.is_empty() {
        let key = varint(&mut bytes);
        assert_eq!(key & 7, 2);
        let length = varint(&mut bytes) as usize;
        fields.push(((key >> 3) as u32, &bytes[..length]));
        bytes = &bytes[length..];
    }
    fields
}

async fn web_catalog_info(address: std::net::SocketAddr) -> (String, Vec<Vec<u8>>) {
    web_rpc(
        address,
        "/screenroom.catalog.v1.CatalogService/GetCatalogInfo",
        &[],
    )
    .await
}

fn bytes_field(output: &mut Vec<u8>, field: u32, bytes: &[u8]) {
    for mut value in [u64::from(field) << 3 | 2, bytes.len() as u64] {
        while value >= 128 {
            output.push((value as u8 & 127) | 128);
            value >>= 7;
        }
        output.push(value as u8);
    }
    output.extend_from_slice(bytes);
}

async fn web_rpc(
    address: std::net::SocketAddr,
    path: &str,
    message: &[u8],
) -> (String, Vec<Vec<u8>>) {
    let mut frame = vec![0];
    frame.extend_from_slice(&(message.len() as u32).to_be_bytes());
    frame.extend_from_slice(message);
    let response = http_body(
        address,
        "POST",
        path,
        &format!(
            "Content-Type: application/grpc-web+proto\r\nX-Grpc-Web: 1\r\nContent-Length: {}\r\n",
            frame.len()
        ),
        &frame,
    )
    .await;
    let split = response
        .windows(4)
        .position(|value| value == b"\r\n\r\n")
        .unwrap()
        + 4;
    let headers = String::from_utf8_lossy(&response[..split]);
    assert!(headers.starts_with("HTTP/1.1 200"));
    assert!(headers.contains("content-type: application/grpc-web"));
    let mut body = response[split..].to_vec();
    if headers.contains("transfer-encoding: chunked") {
        let mut remaining = body.as_slice();
        let mut decoded = vec![];
        loop {
            let end = remaining
                .windows(2)
                .position(|value| value == b"\r\n")
                .unwrap();
            let length =
                usize::from_str_radix(std::str::from_utf8(&remaining[..end]).unwrap(), 16).unwrap();
            if length == 0 {
                break;
            }
            remaining = &remaining[end + 2..];
            decoded.extend_from_slice(&remaining[..length]);
            remaining = &remaining[length + 2..];
        }
        body = decoded;
    }
    let mut code = headers
        .lines()
        .find_map(|line| line.strip_prefix("grpc-status: "))
        .map(str::to_owned);
    let mut messages = vec![];
    let mut remaining = body.as_slice();
    while !remaining.is_empty() {
        let flag = remaining[0];
        let length = u32::from_be_bytes(remaining[1..5].try_into().unwrap()) as usize;
        let payload = &remaining[5..5 + length];
        if flag == 0x80 {
            code = String::from_utf8_lossy(payload)
                .lines()
                .find_map(|line| line.strip_prefix("grpc-status:"))
                .map(|value| value.trim().to_owned());
        } else {
            assert_eq!(flag, 0);
            messages.push(payload.to_vec());
        }
        remaining = &remaining[5 + length..];
    }
    (code.expect("gRPC-Web status"), messages)
}

#[tokio::test]
#[ignore = "requires explicit task-owned disposable PostgreSQL 18 + pgvector URL"]
async fn persisted_catalog_contract() {
    let url = std::env::var("SCREENROOM_G_TEST_DATABASE_URL").expect("task-owned database URL");
    let installation = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .unwrap();
    let database = format!("screenroom_g_runtime_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {database}"))
        .execute(&installation)
        .await
        .unwrap();
    let url = format!(
        "{}/{database}",
        url.rsplit_once('/')
            .expect("task-owned database URL path")
            .0
    );
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .unwrap();
    screenroom_backend::db::migrate(&admin).await.unwrap();
    let repo = CatalogRepository::new(admin.clone());
    let ingest = IngestRepository::new(admin.clone());
    let key = format!("synthetic-{}", Uuid::new_v4());
    let canonical_notices: Vec<_> = (0..7).map(|n| json!({
        "id":format!("canonical-{n}"), "section":"Provenance", "title":format!("Canonical notice {n}"),
        "text":format!("Canonical attribution {n}"), "link_label":"Original", "link_url":format!("https://example.com/notices/{n}"),
    })).collect();
    let source = repo
        .configure_source(
            &key,
            "jellyfin",
            &json!({"notices":canonical_notices, "branding_artwork_ids":["synthetic-image","synthetic-image"]}),
            &json!({}),
        )
        .await
        .unwrap();
    let mirror_key = format!("{key}-mirror");
    repo.configure_source(
        &mirror_key,
        "tmdb",
        &json!({"notices":canonical_notices}),
        &json!({}),
    )
    .await
    .unwrap();
    repo.configure_library(source.id, "movies", "movies", "Movies")
        .await
        .unwrap();
    let policy = ingest
        .create_policy(
            source.id,
            &PolicyWrite {
                enabled: true,
                selection_json: json!({"library_ids":["movies"]}),
                max_items: 500,
                max_requests: 50,
                incremental_seconds: 60,
                reconcile_seconds: 3600,
                enrich_metadata: false,
                enrich_people: false,
                enrich_artwork: false,
            },
        )
        .await
        .unwrap();
    ingest.start_run(policy.id).await.unwrap();
    let claim = ingest.claim("g-test", 60).await.unwrap().unwrap();
    let items=(0..205).map(|n|ItemWrite {public_id:format!("{key}-{n:03}"),kind:match n {1=>"series",2=>"season",3=>"episode",_=>"movie"}.into(),parent_public_id:match n {1..=3=>Some(format!("{key}-{:03}",n-1)),_=>None},external_id:format!("item-{n}"),source_revision:Some("r1".into()),metadata:json!({"title":format!("Synthetic {n}"),"rating":{"provider":"canonical","value":8.1,"scale":10,"votes":null},"runtime":{"seconds":null,"minimum_seconds":60,"note":"bounded runtime"},"provider_references":[{"provider":"canonical","status":"unavailable","url":"https://example.com/provenance"}]}),provenance:json!({}),content_hash:vec![n as u8;32],library_external_ids:vec!["movies".into()],external_ids:vec![],credits:["role-a","role-b","role-c"].into_iter().map(|role|CreditWrite {external_id:"person".into(),name:"Synthetic Person".into(),metadata:json!({}),department:"acting".into(),role:role.into(),ordinal:0}).collect(),document:"synthetic searchable document".into(),document_title:format!("Synthetic {n}"),document_version:1}).collect();
    let mut items: Vec<ItemWrite> = items;
    items[1].parent_public_id = None;
    items[5].kind = "season".into();
    items[5].parent_public_id = Some(format!("{key}-001"));
    items[6].kind = "episode".into();
    items[6].parent_public_id = Some(format!("{key}-002"));
    let snapshot = "2025-03-04T05:06:07.123456Z";
    for item in &mut items {
        item.metadata["metadata_retrieved_at"] = json!("2024-01-02T03:04:05Z");
    }
    items[0].metadata["metadata_retrieved_at"] = json!(snapshot);
    items[1].metadata["metadata_retrieved_at"] = json!("invalid-provider-date");
    ingest
        .commit_page(
            &claim,
            &PageCommit {
                libraries: vec![],
                items,
                feeds: vec![],
                artwork_inputs: (0..205)
                    .map(|n| screenroom_backend::media::ArtworkInputWrite {
                        reference: screenroom_backend::media::ArtworkReferenceInput {
                            source_artwork_id: if n == 0 {
                                "synthetic-image".into()
                            } else {
                                format!("synthetic-image-{n}")
                            },
                            kind: "poster".into(),
                            rights: json!({"label":"Synthetic provenance", "creator":"Synthetic creator", "license":"CC0", "source":"https://example.com/original", "licenseUrl":"https://example.com/license", "derivationNote":"Synthetic native raster"}),
                            item_external_id: Some(format!("item-{n}")),
                            person_external_id: (n == 0).then(|| "person".into()),
                            role: "poster".into(),
                            ordinal: 0,
                        },
                        manifest: screenroom_backend::media::ArtworkInputManifest {
                            version: 1,
                            variants: vec![screenroom_backend::media::ArtworkVariantInput {
                                variant: screenroom_backend::media::ArtworkVariant::Native,
                                sha256: [
                                    67, 28, 237, 105, 22, 162, 162, 26, 21, 110, 56, 112, 26, 254,
                                    85, 187, 215, 248, 137, 105, 251, 191, 197, 109, 127, 224, 153,
                                    212, 127, 38, 84, 96,
                                ],
                            }],
                        },
                    })
                    .collect(),
                artwork_omissions: vec![],
                followups: vec![],
                checkpoint: CheckpointWrite {
                    scope_key: "selected".into(),
                    cursor: json!({"done":true}),
                    watermark: None,
                },
                requests: 1,
                completion: Completion::Complete {
                    evidence: json!({"synthetic":true}),
                },
            },
        )
        .await
        .unwrap();
    let hydration_scope = screenroom_backend::catalog::Scope {
        source_keys: vec![key.clone()],
        library_ids: vec!["movies".into()],
        ..Default::default()
    };
    let stored = repo
        .get(&hydration_scope, &format!("{key}-000"))
        .await
        .unwrap()
        .unwrap();
    let media = screenroom_backend::media::MediaRepository::new(admin.clone());
    assert!(
        media
            .primary_item_artwork(&hydration_scope, &[stored.source_item_id])
            .await
            .unwrap()
            .is_empty()
    );
    let person = repo
        .credits(&hydration_scope, &stored.public_id, None, 10)
        .await
        .unwrap()[0]
        .person_id;
    assert!(
        media
            .person_artwork(&hydration_scope, &[person])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        media
            .branding(&hydration_scope, 8)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        media
            .list(&hydration_scope, None, 200)
            .await
            .unwrap()
            .is_empty()
    );
    let not_ready =
        Catalog::new(admin.clone(), "http://localhost:50052".into(), None, None).unwrap();
    let item = catalog_service_server::CatalogService::get_item(
        &not_ready,
        tonic::Request::new(GetItemRequest {
            id: stored.public_id.clone(),
            scope: Some(SourceLibraryScope {
                source_id: key.clone(),
                library_id: "movies".into(),
            }),
        }),
    )
    .await
    .unwrap()
    .into_inner()
    .item
    .unwrap();
    assert!(item.artwork.is_none());
    let unready_scope = Some(SourceLibraryScope {
        source_id: key.clone(),
        library_id: "movies".into(),
    });
    let credits = catalog_service_server::CatalogService::list_credits(
        &not_ready,
        tonic::Request::new(ListCreditsRequest {
            item_id: stored.public_id.clone(),
            scope: unready_scope.clone(),
            page: None,
        }),
    )
    .await
    .unwrap()
    .into_inner();
    assert!(
        credits
            .credits
            .iter()
            .all(|credit| credit.artwork.is_none())
    );
    let info = catalog_service_server::CatalogService::get_catalog_info(
        &not_ready,
        tonic::Request::new(GetCatalogInfoRequest {}),
    )
    .await
    .unwrap()
    .into_inner()
    .info
    .unwrap();
    assert!(info.branding.is_empty());
    let artwork = catalog_service_server::CatalogService::list_artwork(
        &not_ready,
        tonic::Request::new(ListArtworkRequest {
            scope: unready_scope,
            page: None,
        }),
    )
    .await
    .unwrap()
    .into_inner();
    assert!(artwork.artwork.is_empty());
    let png=base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=").unwrap();
    let unavailable = Arc::new(AtomicBool::new(false));
    let fail = unavailable.clone();
    let storage_mode = Arc::new(AtomicUsize::new(0));
    let mode = storage_mode.clone();
    let dropped = Arc::new(AtomicUsize::new(0));
    let object_dropped = dropped.clone();
    let (truncate_release, truncate_gate) = tokio::sync::oneshot::channel();
    let truncate_gate = Arc::new(std::sync::Mutex::new(Some(truncate_gate)));
    let image = png.clone();
    let synthetic_storage = axum::Router::new().fallback(move |method: axum::http::Method| {
        let image = image.clone();
        let fail = fail.clone();
        let mode = mode.clone();
        let dropped = object_dropped.clone();
        let truncate_gate = truncate_gate.clone();
        async move {
            if fail.load(Ordering::SeqCst) {
                return axum::response::Response::builder()
                    .status(503)
                    .body(axum::body::Body::empty())
                    .unwrap();
            }
            let length = if method == axum::http::Method::PUT {
                0
            } else {
                image.len() + usize::from(mode.load(Ordering::SeqCst) == 2)
            };
            axum::response::Response::builder()
                .header("content-length", length)
                .header("content-type", "image/png")
                .body(if method == axum::http::Method::GET {
                    let mode = mode.load(Ordering::SeqCst);
                    if mode == 1 || mode == 3 {
                        axum::body::Body::from_stream(SlowObject {
                            bytes: image,
                            stage: 0,
                            pause: Box::pin(tokio::time::sleep(std::time::Duration::from_secs(2))),
                            dropped,
                            truncate: if mode == 3 {
                                truncate_gate.lock().unwrap().take()
                            } else {
                                None
                            },
                        })
                    } else if mode == 2 {
                        axum::body::Body::from_stream(
                            tokio_stream::iter([Ok::<_, std::io::Error>(image)])
                                .chain(tokio_stream::pending()),
                        )
                    } else {
                        axum::body::Body::from(image)
                    }
                } else {
                    axum::body::Body::empty()
                })
                .unwrap()
        }
    });
    let storage_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let storage_address = storage_listener.local_addr().unwrap();
    let storage_server = tokio::spawn(async move {
        axum::serve(storage_listener, synthetic_storage)
            .await
            .unwrap();
    });
    let storage = screenroom_backend::storage::ObjectStorage::new(
        screenroom_backend::storage::StorageConfig {
            endpoint: format!("http://{storage_address}"),
            region: "synthetic".into(),
            bucket: "synthetic".into(),
            access_key_id: "synthetic".into(),
            secret_access_key: "synthetic".into(),
            timeout: std::time::Duration::from_secs(2),
        },
    )
    .unwrap();
    let object = storage
        .upload_verified(
            source.id,
            &screenroom_backend::media::ValidatedImage::raster(png.clone()).unwrap(),
        )
        .await
        .unwrap();
    let media = screenroom_backend::media::MediaRepository::new(admin.clone());
    let mut art = None;
    while let Some(claim) = ingest.claim("g-test-art", 60).await.unwrap() {
        let published = media.publish(&claim, &object).await.unwrap();
        if claim.input["source_artwork_id"] == "synthetic-image" {
            art = Some(published);
        }
    }
    let art = art.expect("published synthetic image");
    assert_eq!(
        media
            .primary_item_artwork(&hydration_scope, &[stored.source_item_id])
            .await
            .unwrap()[&stored.source_item_id]
            .id,
        art
    );
    assert_eq!(
        media
            .person_artwork(&hydration_scope, &[person])
            .await
            .unwrap()[&person]
            .id,
        art
    );
    assert_eq!(media.branding(&hydration_scope, 8).await.unwrap().len(), 1);
    let excluded_scope = screenroom_backend::catalog::Scope {
        source_keys: vec!["not-selected".into()],
        ..Default::default()
    };
    assert!(
        media
            .primary_item_artwork(&excluded_scope, &[stored.source_item_id])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        media
            .person_artwork(&excluded_scope, &[person])
            .await
            .unwrap()
            .is_empty()
    );
    assert!(media.branding(&excluded_scope, 8).await.unwrap().is_empty());
    let pending = ingest.start_run(policy.id).await.unwrap();
    let pending_claim = ingest.claim("g-pending-art", 60).await.unwrap().unwrap();
    ingest
        .commit_page(
            &pending_claim,
            &PageCommit {
                libraries: vec![],
                items: vec![],
                feeds: vec![],
                artwork_inputs: vec![screenroom_backend::media::ArtworkInputWrite {
                    reference: screenroom_backend::media::ArtworkReferenceInput {
                        source_artwork_id: "synthetic-image".into(),
                        kind: "poster".into(),
                        rights: json!({"label":"Pending replacement must not be exposed"}),
                        item_external_id: Some("item-0".into()),
                        person_external_id: Some("person".into()),
                        role: "poster".into(),
                        ordinal: 0,
                    },
                    manifest: screenroom_backend::media::ArtworkInputManifest {
                        version: 1,
                        variants: vec![screenroom_backend::media::ArtworkVariantInput {
                            variant: screenroom_backend::media::ArtworkVariant::Native,
                            sha256: [99; 32],
                        }],
                    },
                }],
                artwork_omissions: vec![],
                followups: vec![],
                checkpoint: CheckpointWrite {
                    scope_key: "selected".into(),
                    cursor: json!({"done":true}),
                    watermark: None,
                },
                requests: 1,
                completion: Completion::Complete {
                    evidence: json!({"synthetic":true}),
                },
            },
        )
        .await
        .unwrap();
    assert_eq!(
        media
            .primary_item_artwork(&hydration_scope, &[stored.source_item_id])
            .await
            .unwrap()[&stored.source_item_id]
            .revision,
        1
    );
    assert_eq!(
        media
            .person_artwork(&hydration_scope, &[person])
            .await
            .unwrap()[&person]
            .revision,
        1
    );
    assert_eq!(
        media.branding(&hydration_scope, 8).await.unwrap()[0].revision,
        1
    );
    ingest.cancel_run(pending.id).await.unwrap();
    let runtime = PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|c, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE screenroom_catalog")
                    .execute(c)
                    .await?;
                Ok(())
            })
        })
        .connect(&url)
        .await
        .unwrap();
    let operator = screenroom_catalog::Operator::new(runtime.clone());
    let progress_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let progress_address = progress_listener.local_addr().unwrap();
    let (progress_stop, progress_stopped) = tokio::sync::oneshot::channel();
    let progress_unavailable = Arc::new(AtomicBool::new(false));
    let progress = SyntheticProgress {
        source: key.clone(),
        unavailable: progress_unavailable.clone(),
    };
    let progress_server = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(PlaybackStateServiceServer::new(progress))
            .serve_with_incoming_shutdown(
                tokio_stream::wrappers::TcpListenerStream::new(progress_listener),
                async {
                    let _ = progress_stopped.await;
                },
            )
            .await
            .unwrap();
    });
    let catalog = Catalog::new(
        runtime.clone(),
        "http://localhost:50052".into(),
        Some(storage),
        Some(format!("http://{progress_address}")),
    )
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(screenroom_catalog::serve(
        listener,
        async {
            let _ = stopped.await;
        },
        vec![],
        catalog,
    ));
    let channel = tonic::transport::Endpoint::from_shared(format!("http://{address}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut browse = browse_service_client::BrowseServiceClient::new(channel.clone());
    let mut details = catalog_service_client::CatalogServiceClient::new(channel.clone());
    use browse_service_server::BrowseService;
    let missing_private =
        Catalog::new(runtime.clone(), "http://localhost:50052".into(), None, None).unwrap();
    assert_eq!(
        missing_private
            .get_home(home(&key, ALICE))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unavailable
    );
    let alice = browse
        .get_home(home(&key, ALICE))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(alice.rows[0].items.len(), 1);
    assert_eq!(alice.rows[0].items[0].id, format!("{key}-000"));
    assert_eq!(alice.rows[0].items[0].progress_percent, Some(42));
    assert_eq!(
        alice.rows[0].items[0].artwork.as_ref().unwrap().id,
        art.to_string()
    );
    assert_eq!(
        alice.rows[1].items[0].artwork.as_ref().unwrap().id,
        art.to_string()
    );
    assert_eq!(
        alice.rows[1]
            .items
            .iter()
            .take(4)
            .map(|item| item.kind.as_str())
            .collect::<Vec<_>>(),
        vec!["Movie", "Series", "Season", "Episode"]
    );
    assert!(
        alice.rows[1]
            .items
            .iter()
            .all(|i| i.progress_percent.is_none())
    );
    let bob = browse.get_home(home(&key, BOB)).await.unwrap().into_inner();
    assert_eq!(bob.rows[0].items[0].id, format!("{key}-001"));
    assert_eq!(bob.rows[0].items[0].progress_percent, Some(67));
    assert!(
        browse
            .get_home(home(&key, UNLINKED))
            .await
            .unwrap()
            .into_inner()
            .rows[0]
            .items
            .is_empty()
    );
    assert_eq!(
        browse
            .get_home(home(&key, "00000000-0000-0000-0000-000000000004"))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::NotFound
    );
    assert_eq!(
        browse
            .get_home(home(&key, "invalid"))
            .await
            .unwrap_err()
            .code(),
        tonic::Code::InvalidArgument
    );
    progress_unavailable.store(true, Ordering::SeqCst);
    assert_eq!(
        browse.get_home(home(&key, ALICE)).await.unwrap_err().code(),
        tonic::Code::Unavailable
    );
    progress_unavailable.store(false, Ordering::SeqCst);
    let art_page = details
        .list_artwork(ListArtworkRequest {
            scope: Some(SourceLibraryScope {
                source_id: key.clone(),
                library_id: "movies".into(),
            }),
            page: Some(Page {
                size: 200,
                token: String::new(),
            }),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(art_page.artwork.len(), 200);
    let art_tail = details
        .list_artwork(ListArtworkRequest {
            scope: Some(SourceLibraryScope {
                source_id: key.clone(),
                library_id: "movies".into(),
            }),
            page: Some(Page {
                size: 200,
                token: art_page.next_page_token,
            }),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(art_tail.artwork.len(), 5);
    assert!(art_tail.next_page_token.is_empty());
    let ids: std::collections::BTreeSet<_> = art_page
        .artwork
        .iter()
        .chain(art_tail.artwork.iter())
        .map(|a| &a.id)
        .collect();
    assert_eq!(ids.len(), 205);
    let path = format!("/artwork/{art}/revisions/1/native");
    for target in [
        "/artwork/unknown.jpg",
        "/artwork/%2e%2e%2fCargo.toml",
        "/artwork/%2fetc%2fpasswd",
        "/artwork/not-a-uuid/revisions/1/native",
        "/unknown-http-path",
    ] {
        for method in ["GET", "HEAD"] {
            let response = http(address, method, target, "").await;
            let split = response
                .windows(4)
                .position(|bytes| bytes == b"\r\n\r\n")
                .unwrap()
                + 4;
            let headers = String::from_utf8_lossy(&response[..split]);
            assert!(
                headers.starts_with("HTTP/1.1 404"),
                "{method} {target}: {headers}"
            );
            assert!(!headers.contains("application/grpc"));
            assert!(
                response[split..].is_empty(),
                "HTTP fallback must not expose file or image bytes"
            );
        }
    }
    for target in [path.as_str(), "/ready"] {
        let response = http(address, "POST", target, "Content-Length: 0\r\n").await;
        assert!(response.starts_with(b"HTTP/1.1 405"));
        assert!(!String::from_utf8_lossy(&response).contains("application/grpc"));
    }
    // Unknown-service rejection must not require an unread protobuf request body.
    for target in [
        "/screenroom.catalog.operator.v1.IngestionService/GetPolicy",
        "/screenroom.user.internal.v1.PlaybackStateService/GetContinueWatching",
        "/screenroom.user.v1.PreferencesService/GetPreferences",
        "/screenroom.unknown.v1.Unknown/Unknown",
    ]
    .into_iter()
    .cycle()
    .take(64)
    {
        let response = http_body(
            address,
            "POST",
            target,
            "Content-Type: application/grpc-web+proto\r\nX-Grpc-Web: 1\r\nContent-Length: 0\r\n",
            &[],
        )
        .await;
        let response = String::from_utf8_lossy(&response);
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.contains("content-type: application/grpc-web"));
        assert!(response.contains("grpc-status: 12"), "{target}: {response}");
    }
    let response = http(address, "GET", &path, "").await;
    let end = response.windows(4).position(|b| b == b"\r\n\r\n").unwrap() + 4;
    let headers = String::from_utf8(response[..end].to_vec()).unwrap();
    assert!(headers.starts_with("HTTP/1.1 200"));
    assert!(headers.contains("content-type: image/png"));
    assert!(headers.contains("private, max-age=0, must-revalidate"));
    assert_eq!(&response[end..], &png);
    assert!(!headers.contains("synthetic"));
    let etag = headers
        .lines()
        .find_map(|s| s.strip_prefix("etag: "))
        .unwrap();
    assert!(
        String::from_utf8_lossy(&http(address, "HEAD", &path, "").await)
            .starts_with("HTTP/1.1 200")
    );
    assert!(
        String::from_utf8_lossy(
            &http(address, "GET", &path, &format!("If-None-Match: {etag}\r\n")).await
        )
        .starts_with("HTTP/1.1 304")
    );
    unavailable.store(true, Ordering::SeqCst);
    assert!(
        String::from_utf8_lossy(&http(address, "GET", &path, "").await).starts_with("HTTP/1.1 503")
    );
    unavailable.store(false, Ordering::SeqCst);
    storage_mode.store(2, Ordering::SeqCst);
    {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut socket = tokio::net::TcpStream::connect(storage_address)
            .await
            .unwrap();
        socket
            .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut response = vec![];
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while !response.windows(4).any(|value| value == b"\r\n\r\n") {
                let mut chunk = [0; 1024];
                let length = socket.read(&mut chunk).await.unwrap();
                assert!(length > 0, "invalid length headers must reach the wire");
                response.extend_from_slice(&chunk[..length]);
            }
        })
        .await
        .unwrap();
        assert!(response.starts_with(b"HTTP/1.1 200"));
        assert!(
            String::from_utf8_lossy(&response)
                .contains(&format!("content-length: {}\r\n", png.len() + 1))
        );
    }
    assert!(
        String::from_utf8_lossy(&http(address, "GET", &path, "").await).starts_with("HTTP/1.1 503")
    );
    storage_mode.store(1, Ordering::SeqCst);
    {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let before = dropped.load(Ordering::SeqCst);
        let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
        socket
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        let mut response = vec![];
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                let mut chunk = [0; 1024];
                let length = socket.read(&mut chunk).await.unwrap();
                assert!(length > 0);
                response.extend_from_slice(&chunk[..length]);
                if let Some(headers) = response.windows(4).position(|value| value == b"\r\n\r\n")
                    && response.len() > headers + 4
                {
                    break;
                }
            }
        })
        .await
        .expect("first body chunk must not wait for the complete object");
        let headers = response
            .windows(4)
            .position(|value| value == b"\r\n\r\n")
            .unwrap()
            + 4;
        assert!(response.starts_with(b"HTTP/1.1 200"));
        assert!(response.len() - headers < png.len());
        drop(socket);
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while dropped.load(Ordering::SeqCst) == before {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("client cancellation must drop the upstream object stream");
    }
    storage_mode.store(3, Ordering::SeqCst);
    {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut socket = tokio::net::TcpStream::connect(address).await.unwrap();
        socket
            .write_all(
                format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        let mut response = vec![];
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                let mut chunk = [0; 1024];
                let length = socket.read(&mut chunk).await.unwrap();
                assert!(
                    length > 0,
                    "stream closed before public headers and partial bytes"
                );
                response.extend_from_slice(&chunk[..length]);
                if let Some(headers) = response.windows(4).position(|value| value == b"\r\n\r\n")
                    && response.len() > headers + 4
                {
                    break;
                }
            }
        })
        .await
        .expect("public partial response must precede upstream truncation");
        assert!(response.starts_with(b"HTTP/1.1 200"));
        let headers = response
            .windows(4)
            .position(|value| value == b"\r\n\r\n")
            .unwrap()
            + 4;
        assert_eq!(&response[headers..], &png[..response.len() - headers]);
        assert!(response.len() - headers < png.len());
        truncate_release
            .send(())
            .expect("upstream must remain gated until partial public bytes arrive");
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            socket.read_to_end(&mut response),
        )
        .await
        .unwrap();
        assert!(
            response.len() - headers < png.len(),
            "truncation must abort the already-started response"
        );
    }
    storage_mode.store(0, Ordering::SeqCst);
    let mirror = repo
        .sources()
        .await
        .unwrap()
        .into_iter()
        .find(|source| source.source_key == mirror_key)
        .unwrap();
    repo.configure_library(mirror.id, "other", "other", "Other")
        .await
        .unwrap();
    let mirror_policy = ingest
        .create_policy(
            mirror.id,
            &PolicyWrite {
                enabled: true,
                selection_json: json!({"feeds":["MoviePopular"]}),
                max_items: 100,
                max_requests: 100,
                incremental_seconds: 60,
                reconcile_seconds: 3600,
                enrich_metadata: false,
                enrich_people: false,
                enrich_artwork: false,
            },
        )
        .await
        .unwrap();
    ingest.start_run(mirror_policy.id).await.unwrap();
    let mirror_claim = ingest.claim("g-scoped-roots", 60).await.unwrap().unwrap();
    ingest
        .commit_page(
            &mirror_claim,
            &PageCommit {
                libraries: vec![],
                items: vec![ItemWrite {
                    public_id: format!("{key}-000"),
                    kind: "movie".into(),
                    parent_public_id: None,
                    external_id: "mirror-root".into(),
                    source_revision: Some("synthetic-mirror".into()),
                    metadata: json!({"title":"Mirror root"}),
                    provenance: json!({}),
                    content_hash: vec![7; 32],
                    library_external_ids: vec!["other".into()],
                    external_ids: vec![],
                    credits: vec![],
                    document: "mirror root".into(),
                    document_title: "Mirror root".into(),
                    document_version: 1,
                }],
                feeds: vec![],
                artwork_inputs: vec![],
                artwork_omissions: vec![],
                followups: vec![],
                checkpoint: CheckpointWrite {
                    scope_key: "selected".into(),
                    cursor: json!({"done":true}),
                    watermark: None,
                },
                requests: 1,
                completion: Completion::Complete {
                    evidence: json!({"synthetic":true}),
                },
            },
        )
        .await
        .unwrap();
    let expected_roots: Vec<_> = (0..205)
        .filter(|n| ![2, 3, 5, 6].contains(n))
        .map(|n| format!("{key}-{n:03}"))
        .collect();
    for web in [false, true] {
        let mut token = String::new();
        let mut roots = vec![];
        loop {
            if web {
                let mut message = vec![];
                bytes_field(&mut message, 3, key.as_bytes());
                let mut page = vec![8, 1];
                bytes_field(&mut page, 2, token.as_bytes());
                bytes_field(&mut message, 2, &page);
                bytes_field(&mut message, 1, b"movies");
                let (status, messages) = web_rpc(
                    address,
                    "/screenroom.catalog.v1.BrowseService/ListLibraryItems",
                    &message,
                )
                .await;
                assert_eq!(status, "0");
                assert_eq!(messages.len(), 1);
                let fields = message_fields(&messages[0]);
                let items: Vec<_> = fields.iter().filter(|(field, _)| *field == 1).collect();
                assert_eq!(items.len(), 1);
                let item = message_fields(items[0].1);
                assert!(!item.iter().any(|(field, _)| *field == 4));
                assert!(item.iter().any(|(field,bytes)| *field == 3 && matches!(*bytes, b"Movie" | b"Series")));
                assert!(item.iter().any(|(field, _)| *field == 14));
                roots.push(
                    std::str::from_utf8(item.iter().find(|(field, _)| *field == 1).unwrap().1)
                        .unwrap()
                        .to_owned(),
                );
                token = fields
                    .iter()
                    .find(|(field, _)| *field == 2)
                    .map(|(_, bytes)| std::str::from_utf8(bytes).unwrap().to_owned())
                    .unwrap_or_default();
            } else {
                let response = browse
                    .list_library_items(ListLibraryItemsRequest {
                        source_id: key.clone(),
                        library_id: "movies".into(),
                        page: Some(Page { size: 1, token }),
                    })
                    .await
                    .unwrap()
                    .into_inner();
                assert_eq!(response.items.len(), 1);
                assert!(response.items[0].parent_id.is_none());
                assert!(matches!(
                    response.items[0].kind.as_str(),
                    "Movie" | "Series"
                ));
                assert!(response.items[0].artwork.is_some());
                roots.push(response.items[0].id.clone());
                token = response.next_page_token;
            }
            assert!(roots.len() <= expected_roots.len());
            if token.is_empty() {
                break;
            }
        }
        assert_eq!(roots, expected_roots);
    }
    for (source, library, title) in [
        (key.as_str(), "other", None),
        (mirror_key.as_str(), "other", Some("Mirror root")),
        ("", "other", Some("Mirror root")),
        (mirror_key.as_str(), "movies", None),
    ] {
        let response = browse
            .list_library_items(ListLibraryItemsRequest {
                source_id: source.into(),
                library_id: library.into(),
                page: Some(Page {
                    size: 1,
                    token: String::new(),
                }),
            })
            .await
            .unwrap()
            .into_inner();
        assert_eq!(response.items.len(), usize::from(title.is_some()));
        if let Some(title) = title {
            assert_eq!(response.items[0].title, title);
            assert!(response.items[0].artwork.is_none());
        }
        assert!(response.next_page_token.is_empty());
        let mut message = vec![];
        bytes_field(&mut message, 3, source.as_bytes());
        bytes_field(&mut message, 2, &[8, 1]);
        bytes_field(&mut message, 1, library.as_bytes());
        let (status, messages) = web_rpc(
            address,
            "/screenroom.catalog.v1.BrowseService/ListLibraryItems",
            &message,
        )
        .await;
        assert_eq!(status, "0");
        let fields = message_fields(&messages[0]);
        let items: Vec<_> = fields.iter().filter(|(field, _)| *field == 1).collect();
        assert_eq!(items.len(), usize::from(title.is_some()));
        if let Some(title) = title {
            let item = message_fields(items[0].1);
            assert_eq!(
                item.iter().find(|(field, _)| *field == 2).unwrap().1,
                title.as_bytes()
            );
            assert!(!item.iter().any(|(field, _)| *field == 14));
        }
    }
    let first = browse
        .list_library_items(ListLibraryItemsRequest {
            source_id: key.clone(),
            library_id: "all".into(),
            page: Some(Page {
                size: 200,
                token: String::new(),
            }),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(first.items.len(), 200);
    assert!(!first.next_page_token.is_empty());
    let second = browse
        .list_library_items(ListLibraryItemsRequest {
            source_id: key.clone(),
            library_id: "all".into(),
            page: Some(Page {
                size: 200,
                token: first.next_page_token.clone(),
            }),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(second.items.len(), 1);
    assert!(second.next_page_token.is_empty());
    assert!(
        browse
            .list_library_items(ListLibraryItemsRequest {
                source_id: "other".into(),
                library_id: "all".into(),
                page: Some(Page {
                    size: 200,
                    token: first.next_page_token
                })
            })
            .await
            .is_err()
    );
    let scope = Some(SourceLibraryScope {
        source_id: key.clone(),
        library_id: "movies".into(),
    });
    assert!(
        first
            .items
            .iter()
            .chain(&second.items)
            .all(|item| item.parent_id.is_none()
                && matches!(item.kind.as_str(), "Movie" | "Series")
                && item.artwork.is_some())
    );
    let item_id = first.items[0].id.clone();
    for (n, kind) in ["Movie", "Series", "Season", "Episode"]
        .into_iter()
        .enumerate()
    {
        let detail = details
            .get_item(GetItemRequest {
                id: format!("{key}-{n:03}"),
                scope: scope.clone(),
            })
            .await
            .unwrap()
            .into_inner()
            .item
            .unwrap();
        assert_eq!(detail.kind, kind);
        assert!(detail.artwork.is_some());
        if n >= 2 {
            let mut token = String::new();
            let mut children = vec![];
            loop {
                let page = details
                    .list_children(ListChildrenRequest {
                        parent_id: format!("{key}-{:03}", n - 1),
                        scope: scope.clone(),
                        page: Some(Page { size: 1, token }),
                    })
                    .await
                    .unwrap()
                    .into_inner();
                assert_eq!(page.items.len(), 1);
                assert_eq!(page.items[0].kind, kind);
                assert!(page.items[0].artwork.is_some());
                children.push(page.items[0].id.clone());
                token = page.next_page_token;
                assert!(children.len() <= 2);
                if token.is_empty() {
                    break;
                }
            }
            assert_eq!(
                children,
                vec![format!("{key}-{n:03}"), format!("{key}-{:03}", n + 3)]
            );
        }
    }
    let item = details
        .get_item(GetItemRequest {
            id: item_id.clone(),
            scope: scope.clone(),
        })
        .await
        .unwrap()
        .into_inner()
        .item
        .unwrap();
    assert_eq!(item.rating.unwrap().votes, None);
    assert_eq!(item.runtime.unwrap().seconds, None);
    assert_eq!(
        item.provider_references[0].url,
        "https://example.com/provenance"
    );
    assert_eq!(item.progress_percent, None);
    assert_eq!(item.metadata_retrieved_at, snapshot);
    let artwork = item.artwork.as_ref().unwrap();
    assert_eq!(
        artwork.url,
        format!("http://localhost:50052/artwork/{art}/revisions/1/native")
    );
    assert_eq!(artwork.label, "Synthetic provenance");
    assert_eq!(artwork.creator, "Synthetic creator");
    assert_eq!(artwork.license, "CC0");
    assert_eq!(artwork.source, "https://example.com/original");
    assert_eq!(artwork.license_url, "https://example.com/license");
    let info = details
        .get_catalog_info(GetCatalogInfoRequest {})
        .await
        .unwrap()
        .into_inner()
        .info
        .unwrap();
    assert_eq!(info.branding.len(), 1);
    assert_eq!(info.branding[0].id, art.to_string());
    assert_eq!(info.retrieved_at, snapshot);
    assert_eq!(info.notices.len(), 7);
    assert_eq!(
        info.notices
            .iter()
            .map(|notice| &notice.id)
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        7
    );
    for (notice, expected) in info.notices.iter().zip(&canonical_notices) {
        assert_eq!(
            json!({"id":notice.id,"section":notice.section,"title":notice.title,"text":notice.text,
            "link_label":notice.link_label,"link_url":notice.link_url}),
            *expected
        );
    }
    let (code, messages) = web_catalog_info(address).await;
    assert_eq!(code, "0");
    assert_eq!(messages.len(), 1);
    let response = message_fields(&messages[0]);
    assert_eq!(response.len(), 1);
    assert_eq!(response[0].0, 1);
    let info_fields = message_fields(response[0].1);
    assert_eq!(
        info_fields.iter().find(|(field, _)| *field == 3).unwrap().1,
        snapshot.as_bytes()
    );
    let web_notices: Vec<_> = info_fields
        .iter()
        .filter(|(field, _)| *field == 2)
        .collect();
    assert_eq!(web_notices.len(), 7);
    assert_eq!(
        info_fields.iter().filter(|(field, _)| *field == 4).count(),
        1
    );
    for ((_, bytes), expected) in web_notices.iter().zip(&canonical_notices) {
        let fields = message_fields(bytes);
        assert_eq!(fields.len(), 6);
        for (n, name) in ["id", "section", "title", "text", "link_label", "link_url"]
            .into_iter()
            .enumerate()
        {
            assert_eq!(fields[n].0, n as u32 + 1);
            assert_eq!(
                std::str::from_utf8(fields[n].1).unwrap(),
                expected[name].as_str().unwrap()
            );
        }
    }
    let mut conflicting = canonical_notices.clone();
    conflicting[0]["text"] = json!("private conflicting content must not be returned");
    let malformed = [
        json!({"notices":conflicting}),
        json!({"notices":{}}),
        json!({"notices":null}),
        json!({"notices":[{"title":"missing ID"}]}),
        json!({"notices":[{"id":"bad","text":7}]}),
        json!({"notices":[{"id":" "}]}),
        json!({"notices":[false]}),
        json!({"notices":(0..33).map(|n| json!({"id":format!("extra-{n}")})).collect::<Vec<_>>()}),
    ];
    for configuration in malformed {
        repo.configure_source(&mirror_key, "tmdb", &configuration, &json!({}))
            .await
            .unwrap();
        let error = details
            .get_catalog_info(GetCatalogInfoRequest {})
            .await
            .unwrap_err();
        assert_eq!(error.code(), tonic::Code::FailedPrecondition);
        assert!(!error.message().contains("private conflicting content"));
        let (code, messages) = web_catalog_info(address).await;
        assert_eq!(code, "9");
        assert!(
            messages.is_empty(),
            "invalid notices must not return partial success"
        );
    }
    let unchanged = details
        .list_artwork(ListArtworkRequest {
            scope: Some(SourceLibraryScope {
                source_id: key.clone(),
                library_id: "movies".into(),
            }),
            page: Some(Page {
                size: 200,
                token: String::new(),
            }),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(unchanged.artwork, art_page.artwork);
    let unchanged_tail = details
        .list_artwork(ListArtworkRequest {
            scope: Some(SourceLibraryScope {
                source_id: key.clone(),
                library_id: "movies".into(),
            }),
            page: Some(Page {
                size: 200,
                token: unchanged.next_page_token,
            }),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(unchanged_tail.artwork, art_tail.artwork);
    repo.configure_source(
        &mirror_key,
        "tmdb",
        &json!({"notices":canonical_notices}),
        &json!({}),
    )
    .await
    .unwrap();
    let restored = details
        .get_catalog_info(GetCatalogInfoRequest {})
        .await
        .unwrap()
        .into_inner()
        .info
        .unwrap();
    assert_eq!(restored.notices, info.notices);
    assert_eq!(restored.branding, info.branding);
    let credits = details
        .list_credits(ListCreditsRequest {
            item_id: item_id.clone(),
            scope: scope.clone(),
            page: Some(Page {
                size: 2,
                token: String::new(),
            }),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(credits.credits.len(), 2);
    assert!(credits.credits.iter().all(|credit| {
        credit
            .artwork
            .as_ref()
            .is_some_and(|image| image.id == art.to_string())
    }));
    let tail = details
        .list_credits(ListCreditsRequest {
            item_id,
            scope,
            page: Some(Page {
                size: 2,
                token: credits.next_page_token,
            }),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(tail.credits.len(), 1);
    assert_eq!(tail.credits[0].role, "role-c");
    let mut search = search_service_client::SearchServiceClient::new(channel.clone());
    let found = search
        .search(SearchRequest {
            query: "synthetic".into(),
            source_id: key.clone(),
            methods: Some(SearchMethods {
                text: true,
                semantic: true,
                ..Default::default()
            }),
            ..Default::default()
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(found.items.len(), 50);
    assert!(found.items.iter().all(|item| item.artwork.is_some()));
    assert!(!found.effective_methods.unwrap().semantic);
    assert_eq!(found.degradation_reason, "embedding_unavailable");
    assert!(found.items.iter().all(|item| item.kind != "Season"));
    for (types, kind) in [
        (
            SearchTypes {
                movie: true,
                ..Default::default()
            },
            "Movie",
        ),
        (
            SearchTypes {
                series: true,
                ..Default::default()
            },
            "Series",
        ),
        (
            SearchTypes {
                episode: true,
                ..Default::default()
            },
            "Episode",
        ),
    ] {
        let response = search
            .search(SearchRequest {
                query: "synthetic".into(),
                source_id: key.clone(),
                types: Some(types),
                methods: Some(SearchMethods {
                    text: true,
                    ..Default::default()
                }),
                ..Default::default()
            })
            .await
            .unwrap()
            .into_inner();
        assert!(!response.items.is_empty());
        assert!(response.items.iter().all(|item| item.kind == kind));
    }
    assert_eq!(
        search
            .search(SearchRequest {
                query: "synthetic".into(),
                types: Some(SearchTypes::default()),
                ..Default::default()
            })
            .await
            .unwrap_err()
            .code(),
        tonic::Code::InvalidArgument
    );
    assert_eq!(
        search
            .search(SearchRequest {
                query: "synthetic".into(),
                methods: Some(SearchMethods {
                    title_only: true,
                    ..Default::default()
                }),
                ..Default::default()
            })
            .await
            .unwrap_err()
            .code(),
        tonic::Code::InvalidArgument
    );
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_screenroom-catalog"))
        .env("DATABASE_URL", &url)
        .env("SCREENROOM_CATALOG_LISTEN", "127.0.0.1:0")
        .env("SCREENROOM_CATALOG_PRIVATE_LISTEN", "127.0.0.1:0")
        .env("SCREENROOM_CATALOG_PUBLIC_ORIGIN", "http://localhost:50052")
        .env("RUST_LOG", "info")
        .env_remove("SCREENROOM_EMBEDDING_BASE_URL")
        .env_remove("SCREENROOM_MIGRATION_DATABASE_URL")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut lines =
        tokio::io::AsyncBufReadExt::lines(tokio::io::BufReader::new(child.stdout.take().unwrap()));
    let child_address = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while let Some(line) = lines.next_line().await.unwrap() {
            if let Ok(event) = serde_json::from_str::<serde_json::Value>(&line)
                && event["fields"]["message"] == "RPC server listening"
            {
                return event["fields"]["address"].as_str().unwrap().to_owned();
            }
        }
        panic!("catalog child did not become ready");
    })
    .await
    .unwrap();
    let child_channel = tonic::transport::Endpoint::from_shared(format!("http://{child_address}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut child_details = catalog_service_client::CatalogServiceClient::new(child_channel);
    let mut lock = admin.begin().await.unwrap();
    sqlx::query("LOCK TABLE catalog.sources IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *lock)
        .await
        .unwrap();
    let mut deadline_request = tonic::Request::new(GetCatalogInfoRequest {});
    deadline_request.set_timeout(std::time::Duration::from_millis(40));
    let started = std::time::Instant::now();
    let error = details
        .get_catalog_info(deadline_request)
        .await
        .unwrap_err();
    assert!(matches!(
        error.code(),
        tonic::Code::Cancelled | tonic::Code::DeadlineExceeded
    ));
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    let started = std::time::Instant::now();
    let response = http_body(address, "POST", "/screenroom.catalog.v1.CatalogService/GetCatalogInfo",
        "Content-Type: application/grpc-web+proto\r\nX-Grpc-Web: 1\r\nGrpc-Timeout: 40m\r\nContent-Length: 5\r\n", &[0;5]).await;
    let response = String::from_utf8_lossy(&response);
    assert!(
        response.contains("grpc-status: 1") || response.contains("grpc-status: 4"),
        "{response}"
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(1));
    let held = tokio::spawn(async move {
        child_details
            .get_catalog_info(GetCatalogInfoRequest {})
            .await
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let started = std::time::Instant::now();
    let signal = tokio::process::Command::new("kill")
        .args(["-TERM", &child.id().unwrap().to_string()])
        .status()
        .await
        .unwrap();
    assert!(signal.success());
    let exit = tokio::time::timeout(std::time::Duration::from_secs(7), child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(exit.success());
    assert!(started.elapsed() < std::time::Duration::from_secs(7));
    assert!(
        held.await.unwrap().is_err(),
        "held SQL must not publish a late successful response"
    );
    lock.rollback().await.unwrap();
    assert!(
        search
            .search(SearchRequest {
                query: "synthetic".into(),
                source_id: key.clone(),
                methods: Some(SearchMethods {
                    semantic: true,
                    ..Default::default()
                }),
                ..Default::default()
            })
            .await
            .is_err()
    );
    let mut private = IngestionServiceClient::new(channel);
    assert_eq!(
        private
            .get_policy(GetPolicyRequest {
                policy_id: policy.id.to_string()
            })
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unimplemented
    );
    let private_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let private_address = private_listener.local_addr().unwrap();
    let (private_stop, private_stopped) = tokio::sync::oneshot::channel();
    let private_server = tokio::spawn(screenroom_catalog::serve_operator(
        private_listener,
        async {
            let _ = private_stopped.await;
        },
        operator,
    ));
    let private_channel =
        tonic::transport::Endpoint::from_shared(format!("http://{private_address}"))
            .unwrap()
            .connect()
            .await
            .unwrap();
    let mut operator = IngestionServiceClient::new(private_channel);
    let current = operator
        .get_policy(GetPolicyRequest {
            policy_id: policy.id.to_string(),
        })
        .await
        .unwrap()
        .into_inner()
        .policy
        .unwrap();
    let started = operator
        .start_run(StartRunRequest {
            policy_id: policy.id.to_string(),
        })
        .await
        .unwrap()
        .into_inner()
        .run
        .unwrap();
    assert_eq!(started.policy.unwrap().version, current.version);
    assert!(
        operator
            .get_run(GetRunRequest { run_id: started.id })
            .await
            .unwrap()
            .into_inner()
            .run
            .is_some()
    );
    let mut invalid = current.clone();
    invalid.selection_json = r#"{"library_ids":["movies"],"url":"http://arbitrary"}"#.into();
    assert_eq!(
        operator
            .update_policy(UpdatePolicyRequest {
                policy: Some(invalid)
            })
            .await
            .unwrap_err()
            .code(),
        tonic::Code::InvalidArgument
    );
    let mut disabled = current;
    disabled.enabled = false;
    let updated = operator
        .update_policy(UpdatePolicyRequest {
            policy: Some(disabled),
        })
        .await
        .unwrap()
        .into_inner()
        .policy
        .unwrap();
    assert_eq!(updated.version, 2);
    assert!(!updated.enabled);
    progress_stop.send(()).unwrap();
    progress_server.await.unwrap();
    assert_eq!(
        browse.get_home(home(&key, ALICE)).await.unwrap_err().code(),
        tonic::Code::Unavailable
    );
    repo.set_source_state(source.id, false).await.unwrap();
    let unknown = details
        .get_catalog_info(GetCatalogInfoRequest {})
        .await
        .unwrap()
        .into_inner()
        .info
        .unwrap();
    assert!(unknown.retrieved_at.is_empty());
    let (code, messages) = web_catalog_info(address).await;
    assert_eq!(code, "0");
    assert_eq!(messages.len(), 1);
    let response = message_fields(&messages[0]);
    assert!(
        !message_fields(response[0].1)
            .iter()
            .any(|(field, _)| *field == 3)
    );
    assert!(
        String::from_utf8_lossy(
            &http(address, "GET", &path, &format!("If-None-Match: {etag}\r\n")).await
        )
        .starts_with("HTTP/1.1 404")
    );
    private_stop.send(()).unwrap();
    private_server.await.unwrap().unwrap();
    runtime.close().await;
    assert!(
        String::from_utf8_lossy(&http(address, "GET", "/ready", "").await)
            .starts_with("HTTP/1.1 503")
    );
    assert_eq!(
        details
            .get_catalog_info(GetCatalogInfoRequest {})
            .await
            .unwrap_err()
            .code(),
        tonic::Code::Unavailable
    );
    stop.send(()).unwrap();
    server.await.unwrap().unwrap();
    storage_server.abort();
    admin.close().await;
    sqlx::query(&format!("DROP DATABASE {database} WITH (FORCE)"))
        .execute(&installation)
        .await
        .unwrap();
    installation.close().await;
}
