use screenroom_client_core::{Client, ClientConfig, ClientError};
use std::{
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

struct Observer {
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Drop for Observer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}

#[tokio::test]
async fn url_userinfo_rejected_before_any_http_request() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = listener.local_addr().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let requests = Arc::new(AtomicUsize::new(0));
    let worker_requests = requests.clone();
    let observer = Observer {
        stop: stop.clone(),
        thread: Some(thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        worker_requests.fetch_add(1, Ordering::SeqCst);
                        drop(stream);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("TCP observer: {error}"),
                }
            }
        })),
    };
    let config = |endpoint| ClientConfig {
        endpoint,
        user_id: String::new(),
        timeout_ms: 200,
    };
    let client = Client::new(config(format!("http://{address}"))).unwrap();
    for userinfo in ["username", "u:p", "u:"] {
        assert!(matches!(
            Client::new(config(format!("http://{userinfo}@{address}"))),
            Err(ClientError::InvalidArgument { .. })
        ));
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            client.clone().load_artwork(format!(
                "http://{userinfo}@{address}/artwork/00000000-0000-0000-0000-000000000001/revisions/1/native"
            )),
        )
        .await
        .expect("userinfo guard must finish without network I/O");
        assert!(
            matches!(result, Err(ClientError::InvalidArgument { .. })),
            "{userinfo}: {result:?}"
        );
    }
    drop(client);
    tokio::time::sleep(Duration::from_millis(20)).await;
    drop(observer);
    assert_eq!(requests.load(Ordering::SeqCst), 0);
}
