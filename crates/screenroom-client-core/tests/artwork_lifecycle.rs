#![cfg(not(target_arch = "wasm32"))]

use screenroom_client_core::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn server(response: Vec<u8>) -> (String, Arc<AtomicUsize>, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let count = Arc::new(AtomicUsize::new(0));
    let observed = count.clone();
    let task = tokio::spawn(async move {
        loop {
            let (mut stream, _) = listener.accept().await.unwrap();
            observed.fetch_add(1, Ordering::SeqCst);
            let response = response.clone();
            tokio::spawn(async move {
                let mut request = [0; 4096];
                let _ = stream.read(&mut request).await;
                tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                let _ = stream.write_all(&response).await;
            });
        }
    });
    (endpoint, count, task)
}
fn client(endpoint: &str, user: &str) -> Arc<Client> {
    Client::new(ClientConfig {
        endpoint: endpoint.into(),
        user_id: user.into(),
        timeout_ms: 1000,
    })
    .unwrap()
}
fn artwork(endpoint: &str) -> String {
    format!("{endpoint}/artwork/00000000-0000-0000-0000-000000000001/revisions/1/native")
}
#[tokio::test]
async fn artwork_dedup_cache_and_account_endpoint_isolation() {
    let response = b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc".to_vec();
    let (endpoint, count, task) = server(response.clone()).await;
    let alice = client(&endpoint, "00000000-0000-0000-0000-000000000001");
    let url = artwork(&endpoint);
    let (a, b) = tokio::join!(
        alice.clone().load_artwork(url.clone()),
        alice.clone().load_artwork(url.clone())
    );
    assert_eq!(a.unwrap(), b"abc");
    assert_eq!(b.unwrap(), b"abc");
    alice.clone().load_artwork(url.clone()).await.unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 1);
    client(&endpoint, "00000000-0000-0000-0000-000000000002")
        .load_artwork(url)
        .await
        .unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 2);
    let (other, other_count, other_task) = server(response).await;
    client(&other, "00000000-0000-0000-0000-000000000001")
        .load_artwork(artwork(&other))
        .await
        .unwrap();
    assert_eq!(other_count.load(Ordering::SeqCst), 1);
    task.abort();
    other_task.abort();
}
#[tokio::test]
async fn artwork_failures_are_not_cached_redirects_not_followed_and_size_bounded() {
    for (response, expected) in [
        ("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned(), "not_found"),
        ("HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/should-not-follow\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned(), "unavailable"),
        ("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 3\r\nConnection: close\r\n\r\nabc".to_owned(), "invalid_response"),
        ("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 4194305\r\nConnection: close\r\n\r\n".to_owned(), "resource_exhausted"),
        (format!("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nConnection: close\r\n\r\n{}", "x".repeat(4194305)), "resource_exhausted"),
        ("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: 8\r\nConnection: close\r\n\r\nabc".to_owned(), "unavailable"),
    ] {
        let (endpoint, count, task) = server(response.into_bytes()).await;
        let c = client(&endpoint, "");
        for _ in 0..2 {
            let error = c.clone().load_artwork(artwork(&endpoint)).await.unwrap_err();
            let actual = match error {
                ClientError::NotFound { .. } => "not_found", ClientError::Unavailable { .. } => "unavailable",
                ClientError::InvalidResponse { .. } => "invalid_response", ClientError::ResourceExhausted { .. } => "resource_exhausted",
                e => panic!("unexpected error: {e:?}"),
            };
            assert_eq!(actual, expected);
        }
        assert_eq!(count.load(Ordering::SeqCst), 2);
        task.abort();
    }
}
