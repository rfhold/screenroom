#![cfg(not(target_arch = "wasm32"))]
use screenroom_client_core::*;
use screenroom_proto::user::v1 as w;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tonic::{Request, Response, Status};

#[derive(Clone, Default)]
struct Double {
    link: Arc<Mutex<Option<w::JellyfinLink>>>,
    writes: Arc<AtomicUsize>,
    reads: Arc<AtomicUsize>,
}
#[tonic::async_trait]
impl w::user_service_server::UserService for Double {
    async fn get_current_user(
        &self,
        _: Request<w::GetCurrentUserRequest>,
    ) -> Result<Response<w::GetCurrentUserResponse>, Status> {
        Err(Status::unimplemented("unused"))
    }
    async fn list_jellyfin_sources(
        &self,
        r: Request<w::ListJellyfinSourcesRequest>,
    ) -> Result<Response<w::ListJellyfinSourcesResponse>, Status> {
        assert!(r.metadata().contains_key("x-screenroom-user-id"));
        let page = r.into_inner().page.unwrap();
        Ok(Response::new(w::ListJellyfinSourcesResponse {
            sources: vec![w::JellyfinSource {
                id: page.token,
                display_name: page.size.to_string(),
            }],
            next_page_token: "next".into(),
        }))
    }
    async fn list_jellyfin_profiles(
        &self,
        r: Request<w::ListJellyfinProfilesRequest>,
    ) -> Result<Response<w::ListJellyfinProfilesResponse>, Status> {
        let body = r.into_inner();
        Ok(Response::new(w::ListJellyfinProfilesResponse {
            profiles: vec![w::JellyfinProfile {
                id: body.page.unwrap().token,
                display_name: body.source_id,
            }],
            next_page_token: "next-profile".into(),
        }))
    }
    async fn get_jellyfin_link(
        &self,
        _: Request<w::GetJellyfinLinkRequest>,
    ) -> Result<Response<w::GetJellyfinLinkResponse>, Status> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        let link = self.link.lock().unwrap().clone();
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        Ok(Response::new(w::GetJellyfinLinkResponse { link }))
    }
    async fn set_jellyfin_link(
        &self,
        r: Request<w::SetJellyfinLinkRequest>,
    ) -> Result<Response<w::SetJellyfinLinkResponse>, Status> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        let b = r.into_inner();
        let link = w::JellyfinLink {
            source_id: b.source_id,
            jellyfin_user_id: b.jellyfin_user_id,
        };
        *self.link.lock().unwrap() = Some(link.clone());
        if link.jellyfin_user_id == "timeout" {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        Ok(Response::new(w::SetJellyfinLinkResponse {
            link: Some(link),
        }))
    }
    async fn clear_jellyfin_link(
        &self,
        r: Request<w::ClearJellyfinLinkRequest>,
    ) -> Result<Response<w::ClearJellyfinLinkResponse>, Status> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        *self.link.lock().unwrap() = None;
        Ok(Response::new(w::ClearJellyfinLinkResponse {
            source_id: r.into_inner().source_id,
        }))
    }
}
async fn start(ms: u32) -> (Arc<Client>, Double, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let double = Double::default();
    let server = double.clone();
    let incoming = futures::stream::unfold(listener, |listener| async {
        Some((listener.accept().await.map(|(s, _)| s), listener))
    });
    let task = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(w::user_service_server::UserServiceServer::new(server))
            .serve_with_incoming(incoming)
            .await
            .unwrap();
    });
    let client = Client::new(ClientConfig {
        endpoint,
        user_id: "00000000-0000-0000-0000-000000000001".into(),
        timeout_ms: ms,
    })
    .unwrap();
    (client, double, task)
}
#[tokio::test]
async fn repeated_mutations_execute_and_cancel_old_reads() {
    let (c, d, task) = start(1000).await;
    assert!(
        c.clone()
            .get_jellyfin_link("source".into())
            .await
            .unwrap()
            .is_none()
    );
    c.invalidate_reads();
    let old = tokio::spawn(c.clone().get_jellyfin_link("source".into()));
    while d.reads.load(Ordering::SeqCst) < 2 {
        tokio::task::yield_now().await;
    }
    for _ in 0..2 {
        c.clone()
            .set_jellyfin_link("source".into(), "profile".into())
            .await
            .unwrap();
    }
    assert!(matches!(old.await.unwrap(), Err(ClientError::Cancelled)));
    assert_eq!(
        c.clone()
            .get_jellyfin_link("source".into())
            .await
            .unwrap()
            .unwrap()
            .jellyfin_user_id,
        "profile"
    );
    let (a, b) = tokio::join!(
        c.clone().clear_jellyfin_link("source".into()),
        c.clone().clear_jellyfin_link("source".into())
    );
    a.unwrap();
    b.unwrap();
    assert_eq!(d.writes.load(Ordering::SeqCst), 4);
    assert!(
        c.get_jellyfin_link("source".into())
            .await
            .unwrap()
            .is_none()
    );
    task.abort();
}
#[tokio::test]
async fn ambiguous_timeout_forces_authoritative_read_and_pagination_is_distinct() {
    let (c, d, task) = start(150).await;
    c.clone().get_jellyfin_link("source".into()).await.unwrap();
    assert!(matches!(
        c.clone()
            .set_jellyfin_link("source".into(), "timeout".into())
            .await,
        Err(ClientError::DeadlineExceeded)
    ));
    assert_eq!(
        c.clone()
            .get_jellyfin_link("source".into())
            .await
            .unwrap()
            .unwrap()
            .jellyfin_user_id,
        "timeout"
    );
    assert_eq!(d.reads.load(Ordering::SeqCst), 2);
    let a = c
        .clone()
        .list_jellyfin_sources(Page {
            size: 2,
            token: "a".into(),
        })
        .await
        .unwrap();
    let b = c
        .clone()
        .list_jellyfin_sources(Page {
            size: 3,
            token: "b".into(),
        })
        .await
        .unwrap();
    assert_eq!(a.sources[0].id, "a");
    assert_eq!(b.sources[0].display_name, "3");
    assert_eq!(a.next_page_token, "next");
    let p = c
        .list_jellyfin_profiles(
            "source".into(),
            Page {
                size: 2,
                token: "p".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(p.profiles[0].id, "p");
    assert_eq!(p.next_page_token, "next-profile");
    task.abort();
}

#[tokio::test]
async fn context_required_cancellation_and_dropped_mutation_leave_fresh_reads() {
    let (c, d, task) = start(1000).await;
    let anonymous = Client::new(ClientConfig {
        endpoint: "http://127.0.0.1:1".into(),
        user_id: String::new(),
        timeout_ms: 10,
    })
    .unwrap();
    assert!(matches!(
        anonymous
            .clone()
            .list_jellyfin_sources(Page::default())
            .await,
        Err(ClientError::InvalidArgument { .. })
    ));
    assert!(matches!(
        anonymous.clone().get_jellyfin_link("source".into()).await,
        Err(ClientError::InvalidArgument { .. })
    ));
    assert!(matches!(
        anonymous
            .set_jellyfin_link("source".into(), "profile".into())
            .await,
        Err(ClientError::InvalidArgument { .. })
    ));
    c.clone().get_jellyfin_link("source".into()).await.unwrap();
    let mutation = tokio::spawn(
        c.clone()
            .set_jellyfin_link("source".into(), "timeout".into()),
    );
    while d.writes.load(Ordering::SeqCst) == 0 {
        tokio::task::yield_now().await;
    }
    let during = tokio::spawn(c.clone().get_jellyfin_link("source".into()));
    while d.reads.load(Ordering::SeqCst) < 2 {
        tokio::task::yield_now().await;
    }
    c.cancel_all();
    assert!(matches!(
        mutation.await.unwrap(),
        Err(ClientError::Cancelled)
    ));
    assert!(matches!(during.await.unwrap(), Err(ClientError::Cancelled)));
    assert_eq!(
        c.clone()
            .get_jellyfin_link("source".into())
            .await
            .unwrap()
            .unwrap()
            .jellyfin_user_id,
        "timeout"
    );
    let mutation = tokio::spawn(
        c.clone()
            .set_jellyfin_link("source".into(), "timeout".into()),
    );
    while d.writes.load(Ordering::SeqCst) < 2 {
        tokio::task::yield_now().await;
    }
    mutation.abort();
    assert!(mutation.await.unwrap_err().is_cancelled());
    c.clone().get_jellyfin_link("source".into()).await.unwrap();
    assert_eq!(d.reads.load(Ordering::SeqCst), 4);
    task.abort();
}

#[test]
fn home_progress_error_is_optional_and_typed() {
    let row: HomeRow = serde_json::from_value(serde_json::json!({"id":"continue", "title":"Continue Watching", "items":[], "read_error":{"code":"progress_unavailable", "message":"Progress unavailable"}})).unwrap();
    assert_eq!(row.read_error.unwrap().code, "progress_unavailable");
    let row: HomeRow = serde_json::from_value(
        serde_json::json!({"id":"ordinary", "title":"Ordinary", "items":[]}),
    )
    .unwrap();
    assert!(row.read_error.is_none());
}
