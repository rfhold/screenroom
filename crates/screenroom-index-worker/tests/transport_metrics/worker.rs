use super::{JobTransport, MetricKind, metric_kind};
use screenroom_backend::{
    Uuid,
    embedding::{DIMENSIONS, EmbeddingClient, EmbeddingConfig},
    media::ValidatedImage,
    storage::{ObjectLocation, ObjectStorage, StorageConfig},
};
use screenroom_catalog_adapter::{
    catalog_delta::{Checkpoint, Query, Scope, Timestamp, Window},
    jellyfin::{Artwork, JellyfinConfig, Library, LibraryType, LiveJellyfin},
};
use screenroom_observability::transport::{
    BodyDirection, RequestKind, RequestOutcome, TransportObservation, TransportObserver,
};
use sha2::{Digest, Sha256};
use std::{sync::Arc, sync::Mutex, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

#[derive(Default)]
struct Records {
    bytes: Mutex<Vec<(MetricKind, RequestKind, BodyDirection, u64)>>,
    outcomes: Mutex<Vec<(MetricKind, RequestKind, RequestOutcome)>>,
    changed: tokio::sync::Notify,
}

struct Observer {
    kind: MetricKind,
    records: Arc<Records>,
}

impl TransportObserver for Observer {
    fn body_chunk(&self, request: RequestKind, direction: BodyDirection, bytes: u64) {
        self.records
            .bytes
            .lock()
            .unwrap()
            .push((self.kind, request, direction, bytes));
        if direction == BodyDirection::DownloadConsumed {
            self.records.changed.notify_one();
        }
    }

    fn finished(&self, request: RequestKind, outcome: RequestOutcome, _: Duration) {
        self.records
            .outcomes
            .lock()
            .unwrap()
            .push((self.kind, request, outcome));
    }
}

impl Records {
    fn job(self: &Arc<Self>, kind: &str) -> JobTransport {
        JobTransport::new(kind, |kind| {
            TransportObservation::new(Arc::new(Observer {
                kind,
                records: self.clone(),
            }))
        })
    }

    fn bytes(&self, job: MetricKind, request: RequestKind, direction: BodyDirection) -> u64 {
        self.bytes
            .lock()
            .unwrap()
            .iter()
            .filter(|(j, r, d, _)| (*j, *r, *d) == (job, request, direction))
            .map(|(_, _, _, n)| n)
            .sum()
    }
}

async fn request(socket: &mut TcpStream) -> (String, Vec<u8>) {
    let mut bytes = vec![];
    let end = loop {
        let mut buffer = [0; 4096];
        let n = socket.read(&mut buffer).await.unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&buffer[..n]);
        if let Some(i) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
            break i + 4;
        }
        assert!(bytes.len() < 32768);
    };
    let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
    let length = headers
        .lines()
        .find_map(|line| {
            line.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .map(|n| n.trim().parse::<usize>().unwrap())
        })
        .unwrap_or(0);
    while bytes.len() - end < length {
        let mut buffer = [0; 4096];
        let n = socket.read(&mut buffer).await.unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&buffer[..n]);
    }
    (headers, bytes[end..].to_vec())
}

async fn response(socket: &mut TcpStream, body: &[u8], mime: &str, head: bool) {
    socket
        .write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: {mime}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    if !head {
        socket.write_all(body).await.unwrap();
    }
}

async fn provider(base_url: String) -> LiveJellyfin {
    let secret_file = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("worker-metrics-{}", Uuid::new_v4()));
    std::fs::write(&secret_file, "fake-local-test-token").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&secret_file, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let result = LiveJellyfin::from_config(JellyfinConfig {
        source_key: "test-source".into(),
        base_url,
        secret_file: secret_file.clone(),
        timeout_ms: 2000,
        max_response_bytes: 4096,
    })
    .await;
    std::fs::remove_file(secret_file).unwrap();
    result.unwrap()
}

fn embedding(base_url: String) -> EmbeddingClient {
    EmbeddingClient::new(EmbeddingConfig {
        base_url,
        model: "test-model".into(),
        api_key: None,
        dimensions: DIMENSIONS,
        revision: "test-revision".into(),
        timeout: Duration::from_secs(2),
    })
    .unwrap()
}

fn storage(endpoint: String) -> ObjectStorage {
    ObjectStorage::new(StorageConfig {
        endpoint,
        region: "garage".into(),
        bucket: "private-art".into(),
        access_key_id: "test-access".into(),
        secret_access_key: "test-secret".into(),
        timeout: Duration::from_secs(2),
    })
    .unwrap()
}

#[test]
fn job_kind_selection_is_closed() {
    for (input, expected) in [
        ("ingest", MetricKind::Ingest),
        ("embedding", MetricKind::Embedding),
        ("artwork", MetricKind::Artwork),
        ("maintenance", MetricKind::Other),
        ("unknown-source-or-id", MetricKind::Other),
    ] {
        assert_eq!(metric_kind(input), expected);
    }
}

#[tokio::test]
async fn provider_clones_keep_concurrent_job_attribution_without_duplicates() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = provider(format!("http://{}", listener.local_addr().unwrap())).await;
    let server = tokio::spawn(async move {
        for _ in 0..3 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let (headers, _) = request(&mut socket).await;
            assert!(headers.starts_with("GET /Library/VirtualFolders "));
            response(&mut socket, b"[]", "application/json", false).await;
        }
    });
    let records = Arc::new(Records::default());
    let ingest = records.job("ingest").provider(client.clone());
    let artwork = records.job("artwork").provider(client.clone());
    let (a, b) = tokio::join!(ingest.libraries(), artwork.libraries());
    assert!(a.unwrap().is_empty());
    assert!(b.unwrap().is_empty());
    assert!(client.libraries().await.unwrap().is_empty());
    server.await.unwrap();
    for kind in [MetricKind::Ingest, MetricKind::Artwork] {
        assert_eq!(
            records.bytes(kind, RequestKind::Provider, BodyDirection::DownloadConsumed),
            2
        );
        assert_eq!(
            records
                .outcomes
                .lock()
                .unwrap()
                .iter()
                .filter(|(job, _, _)| *job == kind)
                .copied()
                .collect::<Vec<_>>(),
            [(kind, RequestKind::Provider, RequestOutcome::Succeeded)]
        );
    }
    assert_eq!(records.outcomes.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn embedding_decode_failure_is_not_transport_failure_and_body_is_counted_once() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = embedding(format!("http://{}/v1", listener.local_addr().unwrap()));
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let (headers, body) = request(&mut socket).await;
        assert!(headers.starts_with("POST /v1/embeddings "));
        response(&mut socket, b"{}", "application/json", false).await;
        body
    });
    let records = Arc::new(Records::default());
    assert!(
        records
            .job("embedding")
            .embedding(&base)
            .embed(&["hello".into()])
            .await
            .is_err()
    );
    let uploaded = server.await.unwrap();
    assert_eq!(
        records.bytes(
            MetricKind::Embedding,
            RequestKind::Embedding,
            BodyDirection::UploadBodyConsumed
        ),
        uploaded.len() as u64
    );
    assert_eq!(
        records.bytes(
            MetricKind::Embedding,
            RequestKind::Embedding,
            BodyDirection::DownloadConsumed
        ),
        2
    );
    assert_eq!(
        *records.outcomes.lock().unwrap(),
        [(
            MetricKind::Embedding,
            RequestKind::Embedding,
            RequestOutcome::Succeeded
        )]
    );
}

#[tokio::test]
async fn verified_upload_observes_put_head_get_separately_under_the_actual_job() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = storage(format!("http://{}", listener.local_addr().unwrap()));
    let bytes = vec![
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 2,
        0, 0, 0, 144, 119, 83, 222, 0, 0, 0, 12, 73, 68, 65, 84, 120, 156, 99, 112, 72, 104, 0, 0,
        2, 4, 1, 33, 203, 215, 148, 162, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];
    let image = ValidatedImage::raster(bytes.clone()).unwrap();
    let expected = bytes.clone();
    let server = tokio::spawn(async move {
        for method in ["PUT", "HEAD", "GET"] {
            let (mut socket, _) = listener.accept().await.unwrap();
            let (headers, body) = request(&mut socket).await;
            assert!(headers.starts_with(&format!("{method} /private-art/")));
            if method == "PUT" {
                assert_eq!(body, expected);
                response(&mut socket, b"", "application/json", false).await;
            } else {
                response(&mut socket, &expected, "image/png", method == "HEAD").await;
            }
        }
    });
    let records = Arc::new(Records::default());
    records
        .job("ingest")
        .storage(&base)
        .upload_verified(Uuid::new_v4(), &image)
        .await
        .unwrap();
    server.await.unwrap();
    assert_eq!(
        records.bytes(
            MetricKind::Ingest,
            RequestKind::StorageUpload,
            BodyDirection::UploadBodyConsumed
        ),
        bytes.len() as u64
    );
    assert_eq!(
        records.bytes(
            MetricKind::Ingest,
            RequestKind::StorageRead,
            BodyDirection::DownloadConsumed
        ),
        bytes.len() as u64
    );
    assert_eq!(
        *records.outcomes.lock().unwrap(),
        [
            (
                MetricKind::Ingest,
                RequestKind::StorageUpload,
                RequestOutcome::Succeeded
            ),
            (
                MetricKind::Ingest,
                RequestKind::StorageHead,
                RequestOutcome::Succeeded
            ),
            (
                MetricKind::Ingest,
                RequestKind::StorageRead,
                RequestOutcome::Succeeded
            ),
        ]
    );
    assert!(
        records
            .bytes
            .lock()
            .unwrap()
            .iter()
            .all(|(kind, _, _, _)| *kind == MetricKind::Ingest)
    );
}

#[tokio::test]
async fn provider_partial_failure_and_cancelled_body_emit_once() {
    for cancel in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let records = Arc::new(Records::default());
        let client = records
            .job("artwork")
            .provider(provider(format!("http://{}", listener.local_addr().unwrap())).await);
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            request(&mut socket).await;
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n3\r\nabc\r\n")
                .await
                .unwrap();
            if cancel {
                let mut byte = [0];
                assert_eq!(socket.read(&mut byte).await.unwrap(), 0);
            }
        });
        let mut future = Box::pin(client.libraries());
        if cancel {
            tokio::time::timeout(Duration::from_secs(2), async {
                tokio::select! {
                    _ = &mut future => panic!("body unexpectedly completed"),
                    _ = records.changed.notified() => {}
                }
            })
            .await
            .unwrap();
        } else {
            assert!(future.as_mut().await.is_err());
        }
        drop(future);
        server.await.unwrap();
        assert_eq!(
            records.bytes(
                MetricKind::Artwork,
                RequestKind::Provider,
                BodyDirection::DownloadConsumed
            ),
            3
        );
        assert_eq!(
            *records.outcomes.lock().unwrap(),
            [(
                MetricKind::Artwork,
                RequestKind::Provider,
                if cancel {
                    RequestOutcome::Cancelled
                } else {
                    RequestOutcome::Failed
                }
            )]
        );
    }
}

#[tokio::test]
async fn delta_capability_page_and_artwork_are_observed_as_individual_requests() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let records = Arc::new(Records::default());
    let client = records
        .job("ingest")
        .provider(provider(format!("http://{}", listener.local_addr().unwrap())).await);
    let info = br#"{"Version":"10.11.1"}"#;
    let page = br#"{"Items":[],"TotalRecordCount":0,"StartIndex":0}"#;
    let server = tokio::spawn(async move {
        for (path, body) in [
            ("/System/Info/Public ", info.as_slice()),
            ("/Items?", page.as_slice()),
            ("/Items/item/Images/Primary?", b"abc".as_slice()),
        ] {
            let (mut socket, _) = listener.accept().await.unwrap();
            let (headers, _) = request(&mut socket).await;
            assert!(headers.starts_with(&format!("GET {path}")), "{headers}");
            response(&mut socket, body, "application/json", false).await;
        }
    });
    let timestamp = Timestamp::parse("2026-01-01T00:00:00Z").unwrap();
    let mut checkpoint = Checkpoint::new(
        Query::new(
            Scope::new("test-source", "movies").unwrap(),
            Window::new(timestamp.clone(), timestamp).unwrap(),
            1,
            1,
            1,
        )
        .unwrap(),
    );
    let library = Library {
        id: client.public_id("movies"),
        external_id: "movies".into(),
        name: "Movies".into(),
        kind: LibraryType::Movies,
    };
    assert!(
        client
            .library_delta_page(&library, &mut checkpoint)
            .await
            .unwrap()
            .page
            .items
            .is_empty()
    );
    assert_eq!(
        client
            .artwork_bytes(&Artwork {
                item_id: "item".into(),
                kind: "Primary".into(),
                tag: "tag".into()
            })
            .await
            .unwrap(),
        b"abc"
    );
    server.await.unwrap();
    assert_eq!(
        records.bytes(
            MetricKind::Ingest,
            RequestKind::Provider,
            BodyDirection::DownloadConsumed
        ),
        (info.len() + page.len() + 3) as u64
    );
    assert_eq!(
        *records.outcomes.lock().unwrap(),
        [(
            MetricKind::Ingest,
            RequestKind::Provider,
            RequestOutcome::Succeeded
        ); 3]
    );
}

#[tokio::test]
async fn embedding_and_storage_partial_failure_and_cancellation_keep_actual_bytes() {
    for request_kind in [RequestKind::Embedding, RequestKind::StorageRead] {
        for cancel in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}", listener.local_addr().unwrap());
            let records = Arc::new(Records::default());
            let transport = records.job("artwork");
            let embedding = transport.embedding(&embedding(endpoint.clone()));
            let storage = transport.storage(&storage(endpoint));
            let object = ObjectLocation {
                bucket: "private-art".into(),
                object_key: "test-object".into(),
                sha256: Sha256::digest(b"abcdef").to_vec(),
                mime: "image/png".into(),
                byte_length: 6,
            };
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                request(&mut socket).await;
                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nContent-Type: image/png\r\nConnection: close\r\n\r\nabc").await.unwrap();
                if cancel {
                    let mut byte = [0];
                    assert_eq!(socket.read(&mut byte).await.unwrap(), 0);
                }
            });
            let mut future = Box::pin(async {
                if request_kind == RequestKind::Embedding {
                    embedding.embed(&["hello".into()]).await.map(|_| ())
                } else {
                    storage.read(&object).await.map(|_| ())
                }
            });
            if cancel {
                tokio::time::timeout(Duration::from_secs(2), async {
                    tokio::select! {
                        _ = &mut future => panic!("body unexpectedly completed"),
                        _ = records.changed.notified() => {}
                    }
                })
                .await
                .unwrap();
            } else {
                assert!(future.as_mut().await.is_err());
            }
            drop(future);
            server.await.unwrap();
            assert_eq!(
                records.bytes(
                    MetricKind::Artwork,
                    request_kind,
                    BodyDirection::DownloadConsumed
                ),
                3
            );
            assert_eq!(
                *records.outcomes.lock().unwrap(),
                [(
                    MetricKind::Artwork,
                    request_kind,
                    if cancel {
                        RequestOutcome::Cancelled
                    } else {
                        RequestOutcome::Failed
                    }
                )]
            );
        }
    }
}
