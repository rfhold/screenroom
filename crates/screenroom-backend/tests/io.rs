use futures_util::StreamExt;
use screenroom_backend::{
    Error, Uuid,
    embedding::{DIMENSIONS, EmbeddingClient, EmbeddingConfig},
    media::ValidatedImage,
    storage::{ObjectLocation, ObjectStorage, StorageConfig},
};
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

async fn request(socket: &mut TcpStream) -> (String, Vec<u8>) {
    let mut bytes = Vec::new();
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
        .find_map(|l| {
            l.to_ascii_lowercase()
                .strip_prefix("content-length:")
                .and_then(|s| s.trim().parse::<usize>().ok())
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
async fn response(socket: &mut TcpStream, status: u16, body: &[u8], head: bool) {
    response_mime(socket, status, body, head, "application/json").await;
}
async fn response_mime(socket: &mut TcpStream, status: u16, body: &[u8], head: bool, mime: &str) {
    socket
        .write_all(
            format!(
                "HTTP/1.1 {status} test\r\nContent-Length: {}\r\nContent-Type: {mime}\r\nConnection: close\r\n\r\n", body.len()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    if !head {
        socket.write_all(body).await.unwrap();
    }
}
fn embedding_config(base: String) -> EmbeddingConfig {
    EmbeddingConfig {
        base_url: base,
        model: "test-model".into(),
        api_key: Some("explicit-test-key".into()),
        dimensions: DIMENSIONS,
        revision: "operator-r1".into(),
        timeout: Duration::from_secs(2),
    }
}
fn storage_config(endpoint: String) -> StorageConfig {
    StorageConfig {
        endpoint,
        region: "garage".into(),
        bucket: "private-art".into(),
        access_key_id: "test-access".into(),
        secret_access_key: "test-secret".into(),
        timeout: Duration::from_secs(2),
    }
}

#[tokio::test]
async fn embedding_http_once_reorders_and_classifies_errors() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for status in [200, 503, 400, 200, 200] {
            let (mut socket, _) = listener.accept().await.unwrap();
            let (headers, body) = request(&mut socket).await;
            assert!(headers.starts_with("POST /v1/embeddings "));
            assert!(
                headers
                    .to_ascii_lowercase()
                    .contains("authorization: bearer explicit-test-key")
            );
            let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(body["input"][0], "hello world");
            let mut vector = vec![0.0; DIMENSIONS];
            vector[0] = 1.0;
            let bytes = if status == 200 {
                serde_json::to_vec(&serde_json::json!({"model":"response-alias","data":[{"index":1,"embedding":vector},{"index":0,"embedding":vector}]})).unwrap()
            } else {
                vec![]
            };
            response(&mut socket, status, &bytes, false).await;
        }
    });
    let client = EmbeddingClient::new(embedding_config(base)).unwrap();
    let input = vec![" hello\n world ".into(), "second".into()];
    let batch = client.embed(&input).await.unwrap();
    assert_eq!(batch.vectors.len(), 2);
    assert_eq!(batch.response_model, "response-alias");
    assert!(matches!(
        client.embed(&input).await,
        Err(Error::Unavailable)
    ));
    assert!(matches!(
        client.embed(&input).await,
        Err(Error::Incompatible(_))
    ));
    assert!(matches!(
        client.embed(&[" hello\n world ".into()]).await,
        Err(Error::Incompatible(_))
    ));
    assert!(client.embed(&input).await.is_ok());
    server.await.unwrap();
    assert!(client.embed(&[]).await.is_err());
}

#[tokio::test]
async fn transport_deadlines_and_response_bounds() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (closed_tx, closed_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let _ = request(&mut socket).await;
        let mut byte = [0];
        let n = socket.read(&mut byte).await.unwrap();
        closed_tx.send(n == 0).unwrap();
    });
    let mut config = embedding_config(base);
    config.timeout = Duration::from_millis(50);
    let client = EmbeddingClient::new(config).unwrap();
    assert!(matches!(
        client.embed(&["test".into()]).await,
        Err(Error::Unavailable)
    ));
    assert!(
        tokio::time::timeout(Duration::from_secs(2), closed_rx)
            .await
            .unwrap()
            .unwrap()
    );
    server.await.unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let _ = request(&mut socket).await;
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3000000\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
    });
    let client = EmbeddingClient::new(embedding_config(base)).unwrap();
    assert!(matches!(
        client.embed(&["test".into()]).await,
        Err(Error::Incompatible(_))
    ));
    server.await.unwrap();
}

#[tokio::test]
async fn private_s3_header_signing_upload_head_digest_read() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let bytes = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/catalog/artwork/tmdb.png"),
    )
    .unwrap();
    let expected = bytes.clone();
    let server = tokio::spawn(async move {
        for method in ["PUT", "HEAD", "GET", "GET", "GET"] {
            let (mut socket, _) = listener.accept().await.unwrap();
            let (headers, body) = request(&mut socket).await;
            assert!(headers.starts_with(&format!("{method} /private-art/artwork/v1/sources/")));
            assert!(!headers.lines().next().unwrap().contains('?'));
            assert!(
                headers
                    .to_ascii_lowercase()
                    .contains("authorization: aws4-hmac-sha256")
            );
            assert!(
                headers
                    .to_ascii_lowercase()
                    .contains("x-amz-content-sha256:")
            );
            if method == "PUT" {
                assert_eq!(body, expected);
                response(&mut socket, 200, &[], false).await;
            } else {
                response_mime(&mut socket, 200, &expected, method == "HEAD", "image/png").await;
            }
        }
    });
    let storage = ObjectStorage::new(storage_config(endpoint)).unwrap();
    let source = Uuid::new_v4();
    let image = ValidatedImage::raster(bytes.clone()).unwrap();
    let verified = storage.upload_verified(source, &image).await.unwrap();
    assert_eq!(verified.source_id(), source);
    assert_eq!(storage.read(verified.location()).await.unwrap(), bytes);
    let mut corrupt = verified.location().clone();
    corrupt.sha256 = vec![0; 32];
    assert!(matches!(
        storage.read(&corrupt).await,
        Err(Error::Incompatible(_))
    ));
    let invalid = ObjectLocation {
        bucket: "wrong-bucket".into(),
        ..verified.location().clone()
    };
    assert!(matches!(
        storage.read(&invalid).await,
        Err(Error::Invalid(_))
    ));
    server.await.unwrap();
}

#[test]
fn configured_dimensions_and_declared_revision_are_mandatory() {
    let mut config = embedding_config("http://localhost:1/v1".into());
    config.dimensions = 768;
    assert!(EmbeddingClient::new(config.clone()).is_err());
    config.dimensions = 1024;
    config.revision.clear();
    assert!(EmbeddingClient::new(config).is_err());
}

fn stream_location(bytes: &[u8]) -> ObjectLocation {
    ObjectLocation {
        bucket: "private-art".into(),
        object_key: "test-object".into(),
        sha256: Sha256::digest(bytes).to_vec(),
        mime: "image/png".into(),
        byte_length: bytes.len() as i64,
    }
}
async fn stream_headers(socket: &mut TcpStream, length: usize) {
    socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {length}\r\nContent-Type: image/png\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
}

#[tokio::test]
async fn object_stream_delivers_before_completion_and_bounds_chunks() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let storage = ObjectStorage::new(storage_config(format!(
        "http://{}",
        listener.local_addr().unwrap()
    )))
    .unwrap();
    let bytes = vec![7u8; 200_000];
    let location = stream_location(&bytes);
    let (release, wait) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let (headers, _) = request(&mut socket).await;
        assert!(
            headers
                .to_ascii_lowercase()
                .contains("authorization: aws4-hmac-sha256")
        );
        stream_headers(&mut socket, bytes.len()).await;
        socket.write_all(&bytes[..100_000]).await.unwrap();
        wait.await.unwrap();
        socket.write_all(&bytes[100_000..]).await.unwrap();
    });
    let mut read = storage.read_stream(&location, 200_000).await.unwrap();
    assert_eq!(read.location.byte_length, 200_000);
    let first = tokio::time::timeout(Duration::from_secs(1), read.body.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(!first.is_empty() && first.len() <= 64 * 1024);
    assert!(!server.is_finished());
    release.send(()).unwrap();
    let mut total = first.len();
    while let Some(chunk) = read.body.next().await {
        let chunk = chunk.unwrap();
        assert!(chunk.len() <= 64 * 1024);
        total += chunk.len();
    }
    assert_eq!(total, 200_000);
    server.await.unwrap();
}

#[tokio::test]
async fn object_stream_drop_and_body_deadline_close_upstream() {
    for timeout in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut config = storage_config(format!("http://{}", listener.local_addr().unwrap()));
        if timeout {
            config.timeout = Duration::from_millis(150);
        }
        let storage = ObjectStorage::new(config).unwrap();
        let (closed, wait) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            request(&mut socket).await;
            stream_headers(&mut socket, 6).await;
            socket.write_all(b"abc").await.unwrap();
            let mut byte = [0];
            closed
                .send(socket.read(&mut byte).await.unwrap() == 0)
                .unwrap();
        });
        let mut read = storage
            .read_stream(&stream_location(b"abcdef"), 6)
            .await
            .unwrap();
        assert_eq!(read.body.next().await.unwrap().unwrap().as_ref(), b"abc");
        if timeout {
            assert!(matches!(
                read.body.next().await.unwrap(),
                Err(Error::Unavailable)
            ));
        }
        drop(read);
        assert!(
            tokio::time::timeout(Duration::from_secs(2), wait)
                .await
                .unwrap()
                .unwrap()
        );
        server.await.unwrap();
    }
}

#[tokio::test]
async fn object_stream_rejects_headers_and_preflight_limits() {
    for headers in [
        "HTTP/1.1 503 unavailable\r\nContent-Length: 6\r\nContent-Type: image/png\r\n",
        "HTTP/1.1 206 partial\r\nContent-Length: 6\r\nContent-Type: image/png\r\n",
        "HTTP/1.1 200 OK\r\nContent-Length: 7\r\nContent-Type: image/png\r\n",
        "HTTP/1.1 200 OK\r\nContent-Length: 20971521\r\nContent-Type: image/png\r\n",
        "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\n",
        "HTTP/1.1 200 OK\r\nContent-Length: 6\r\nContent-Type: image/jpeg\r\n",
        "HTTP/1.1 200 OK\r\nContent-Length: 6\r\n",
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let storage = ObjectStorage::new(storage_config(format!(
            "http://{}",
            listener.local_addr().unwrap()
        )))
        .unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            request(&mut socket).await;
            socket
                .write_all(format!("{headers}Connection: close\r\n\r\n").as_bytes())
                .await
                .unwrap();
        });
        assert!(
            storage
                .read_stream(&stream_location(b"abcdef"), 6)
                .await
                .is_err()
        );
        server.await.unwrap();
    }
    let storage = ObjectStorage::new(storage_config("http://127.0.0.1:1".into())).unwrap();
    assert!(matches!(
        storage.read_stream(&stream_location(b"abcdef"), 5).await,
        Err(Error::Invalid(_))
    ));
    assert!(
        storage
            .read_stream(&stream_location(b"abcdef"), 0)
            .await
            .is_err()
    );
    assert!(
        storage
            .read_stream(&stream_location(b"abcdef"), 20 * 1024 * 1024 + 1)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn object_stream_truncation_and_digest_failure_never_emit_full_length() {
    for body in [b"abc".as_slice(), b"ABCDEF".as_slice()] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let storage = ObjectStorage::new(storage_config(format!(
            "http://{}",
            listener.local_addr().unwrap()
        )))
        .unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            request(&mut socket).await;
            stream_headers(&mut socket, 6).await;
            socket.write_all(body).await.unwrap();
        });
        let mut read = storage
            .read_stream(&stream_location(b"abcdef"), 6)
            .await
            .unwrap();
        let mut emitted = 0;
        let mut failed = false;
        while let Some(chunk) = read.body.next().await {
            match chunk {
                Ok(bytes) => emitted += bytes.len(),
                Err(_) => {
                    failed = true;
                    break;
                }
            }
        }
        assert!(failed && emitted < 6);
        server.await.unwrap();
    }
}
