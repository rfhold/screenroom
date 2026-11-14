use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn address_policy_allows_lan_and_loopback_but_rejects_special_destinations() {
    for address in [
        "127.0.0.1",
        "10.0.0.1",
        "172.16.1.1",
        "192.168.1.8",
        "100.64.0.1",
        "::1",
        "fd12::1",
        "2001:db8::1",
        "::ffff:127.0.0.1",
    ] {
        assert!(allowed(address.parse().unwrap()), "{address}");
    }
    for address in [
        "0.0.0.0",
        "0.1.2.3",
        "224.0.0.1",
        "255.255.255.255",
        "192.168.1.255",
        "169.254.169.254",
        "100.100.100.200",
        "192.0.0.192",
        "::",
        "ff02::1",
        "fe80::1",
        "febf::1",
        "::ffff:169.254.169.254",
        "::ffff:0.0.0.0",
        "fd00:ec2::254",
        "64:ff9b::a9fe:a9fe",
        "2002:a9fe:a9fe::1",
        "::169.254.169.254",
    ] {
        assert!(!allowed(address.parse().unwrap()), "{address}");
    }
}

struct FakeLookup {
    calls: Arc<AtomicUsize>,
    answers: Vec<SocketAddr>,
}
impl Lookup for FakeLookup {
    fn lookup(&self, _: String) -> Resolving {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let addresses = if call == 0 {
            self.answers.clone()
        } else {
            vec!["169.254.169.254:0".parse().unwrap()]
        };
        Box::pin(async move { Ok(Box::new(addresses.into_iter()) as Addrs) })
    }
}

#[tokio::test]
async fn validated_dns_sockets_preserve_original_host_and_prevent_second_lookup() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let calls = Arc::new(AtomicUsize::new(0));
    let resolver = GuardedResolver {
        lookup: Arc::new(FakeLookup {
            calls: calls.clone(),
            answers: vec!["127.0.0.1:0".parse().unwrap()],
        }),
        timeout: Duration::from_secs(1),
    };
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 4096];
        let n = socket.read(&mut bytes).await.unwrap();
        let request = String::from_utf8_lossy(&bytes[..n]).to_lowercase();
        assert!(request.contains(&format!("host: jellyfin.invalid:{port}")));
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n[]")
            .await
            .unwrap();
    });
    let client = reqwest::Client::builder()
        .no_proxy()
        .dns_resolver(Arc::new(resolver))
        .build()
        .unwrap();
    assert_eq!(
        client
            .get(format!("http://jellyfin.invalid:{port}/base/Users"))
            .send()
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap(),
        "[]"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    server.await.unwrap();
    let error = client
        .get(format!("http://jellyfin.invalid:{port}/base/Users"))
        .send()
        .await
        .unwrap_err();
    assert_eq!(classify(&error), Some(Error::DestinationRejected));
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn all_dns_answers_are_validated_before_any_connection_and_answers_are_bounded() {
    for (answers, expected) in [
        (
            vec![
                "127.0.0.1:0".parse().unwrap(),
                "169.254.169.254:0".parse().unwrap(),
            ],
            Error::DestinationRejected,
        ),
        (vec!["127.0.0.1:0".parse().unwrap(); 65], Error::Resolution),
        (vec![], Error::Resolution),
    ] {
        let calls = Arc::new(AtomicUsize::new(0));
        let resolver = GuardedResolver {
            lookup: Arc::new(FakeLookup {
                calls: calls.clone(),
                answers,
            }),
            timeout: Duration::from_secs(1),
        };
        let client = reqwest::Client::builder()
            .no_proxy()
            .dns_resolver(Arc::new(resolver))
            .build()
            .unwrap();
        let error = client
            .get("http://jellyfin.invalid:1234/")
            .send()
            .await
            .unwrap_err();
        assert_eq!(classify(&error), Some(expected));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

struct PendingLookup;
impl Lookup for PendingLookup {
    fn lookup(&self, _: String) -> Resolving {
        Box::pin(std::future::pending())
    }
}
#[tokio::test]
async fn dns_lookup_has_its_own_deadline() {
    let resolver = GuardedResolver {
        lookup: Arc::new(PendingLookup),
        timeout: Duration::from_millis(1),
    };
    let client = reqwest::Client::builder()
        .no_proxy()
        .dns_resolver(Arc::new(resolver))
        .build()
        .unwrap();
    let error = client
        .get("http://jellyfin.invalid/")
        .send()
        .await
        .unwrap_err();
    assert_eq!(classify(&error), Some(Error::Deadline));
}
