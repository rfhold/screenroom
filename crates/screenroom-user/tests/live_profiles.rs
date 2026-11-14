use screenroom_backend::{Uuid, sqlx};
use screenroom_proto::{
    common::v1::SourceLibraryScope,
    user::{
        internal::v1::{
            GetContinueWatchingRequest, playback_state_service_client::PlaybackStateServiceClient,
        },
        v1::*,
    },
};
use screenroom_user::{UserService, repository::Repository};
mod support;
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tonic::{Code, Request};

fn selected<T>(body: T, id: Uuid) -> Request<T> {
    let mut request = Request::new(body);
    request.metadata_mut().insert(
        screenroom_proto::LOCAL_USER_ID_METADATA_KEY,
        id.to_string().parse().unwrap(),
    );
    request
}

struct Double {
    url: String,
    fault: Arc<AtomicBool>,
    entered: Arc<tokio::sync::Notify>,
    release: Arc<tokio::sync::Notify>,
    pause: Arc<AtomicBool>,
    changed: Arc<AtomicBool>,
    churn: Arc<AtomicBool>,
    requests: Arc<std::sync::Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Double {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Double {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let fault = Arc::new(AtomicBool::new(false));
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        let pause = Arc::new(AtomicBool::new(false));
        let changed = Arc::new(AtomicBool::new(false));
        let changed_server = changed.clone();
        let churn = Arc::new(AtomicBool::new(false));
        let churn_server = churn.clone();
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let requests_server = requests.clone();
        let (f, e, r, p) = (
            fault.clone(),
            entered.clone(),
            release.clone(),
            pause.clone(),
        );
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let (f, e, r, p) = (f.clone(), e.clone(), r.clone(), p.clone());
                let changed = changed_server.clone();
                let churn = churn_server.clone();
                let requests = requests_server.clone();
                tokio::spawn(async move {
                    let mut data = Vec::new();
                    loop {
                        let mut bytes = [0; 4096];
                        let n = socket.read(&mut bytes).await.unwrap();
                        if n == 0 {
                            return;
                        }
                        data.extend_from_slice(&bytes[..n]);
                        if data.windows(4).any(|b| b == b"\r\n\r\n") {
                            break;
                        }
                    }
                    let request = String::from_utf8(data).unwrap();
                    requests.lock().unwrap().push(request.clone());
                    assert!(
                        request
                            .to_lowercase()
                            .contains("authorization: mediabrowser")
                    );
                    assert!(request.contains("test-owned-token"));
                    assert!(!request.contains("00000000-0000-0000"));
                    let path = request
                        .lines()
                        .next()
                        .unwrap()
                        .split_whitespace()
                        .nth(1)
                        .unwrap();
                    let mut status = "200 OK";
                    let body = if f.load(Ordering::SeqCst) {
                        status = "503 Service Unavailable";
                        json!({"secret":"upstream-private-url-token"})
                    } else if path == "/System/Info" {
                        json!({"Id":"user-test","ServerName":"Owned double","Version":"10.11.0"})
                    } else if path == "/Users" {
                        json!([
                            {"Id":"hidden","Name":"Hidden enabled","Policy":{"IsHidden":true,"IsDisabled":false}},
                            {"Id":"second","Name":"Second","Policy":{"IsHidden":false,"IsDisabled":false}},
                            {"Id":"disabled","Name":"Disabled","Policy":{"IsHidden":false,"IsDisabled":true}}
                        ])
                    } else if path == "/Library/VirtualFolders" {
                        json!([
                            {"ItemId":"movies","Name":"Movies","CollectionType":"movies"},
                            {"ItemId":"shows","Name":"Shows","CollectionType":"tvshows"}
                        ])
                    } else if path.starts_with("/Users/") && path.contains("/Items/Resume?") {
                        if p.swap(false, Ordering::SeqCst) {
                            e.notify_one();
                            r.notified().await;
                        }
                        let query: std::collections::BTreeMap<_, _> = path
                            .split_once('?')
                            .unwrap()
                            .1
                            .split('&')
                            .filter_map(|s| s.split_once('='))
                            .collect();
                        let start: usize = query["StartIndex"].parse().unwrap();
                        if start > 0 && churn.load(Ordering::SeqCst) {
                            changed.store(true, Ordering::SeqCst);
                        }
                        let limit: usize = query["Limit"].parse().unwrap();
                        let library = query["ParentId"];
                        let total = if library == "movies" { 125 } else { 3 };
                        let items: Vec<_> = (start..(start+limit).min(total)).map(|i| json!({
                            "Id":format!("{library}-{i}"), "Name":"Test item", "Type":"Movie", "RunTimeTicks":10_000_000_000u64,
                            "UserData":{"PlaybackPositionTicks":(i as u64+1+u64::from(changed.load(Ordering::SeqCst)))*10_000_000,"Played":false,"LastPlayedDate":"2026-11-10T10:00:00Z"}
                        })).collect();
                        json!({"Items":items,"StartIndex":start,"TotalRecordCount":total})
                    } else {
                        panic!("unexpected request path");
                    };
                    let body = serde_json::to_vec(&body).unwrap();
                    let header = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    socket.write_all(header.as_bytes()).await.unwrap();
                    socket.write_all(&body).await.unwrap();
                });
            }
        });
        Self {
            url,
            fault,
            entered,
            release,
            pause,
            changed,
            churn,
            requests,
            task,
        }
    }
}

#[tokio::test]
#[ignore = "requires task-owned SCREENROOM_TEST_DATABASE_URL"]
async fn trusted_local_profile_selection_and_live_progress_over_transport() {
    let fixture = support::Fixture::new().await;
    let pool = fixture.user.clone();
    let admin = fixture.pool.clone();
    assert!(
        sqlx::query("SELECT id FROM catalog.source_items")
            .execute(&pool)
            .await
            .is_err()
    );
    let repository = Repository::new(pool);
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();
    let disabled = Uuid::new_v4();
    for id in [alice, bob, disabled] {
        repository.init_user(id, "Test user").await.unwrap();
    }
    sqlx::query("UPDATE identity.users SET state='disabled' WHERE id=$1")
        .bind(disabled)
        .execute(&admin)
        .await
        .unwrap();
    let double = Double::start().await;
    let path = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("user-dynamic-{}", Uuid::new_v4()));
    let library = "jellyfin:live:6d6f76696573".to_string();
    let service = UserService {
        repository: repository.clone(),
        connections: screenroom_backend::connections::ConnectionRepository::new(
            fixture.user.clone(),
            Some(screenroom_backend::connections::CredentialDirectory::new(path.clone()).unwrap()),
        ),
    };
    let empty = user_service_server::UserService::list_jellyfin_sources(
        &service,
        selected(ListJellyfinSourcesRequest { page: None }, alice),
    )
    .await
    .unwrap()
    .into_inner();
    assert!(empty.sources.is_empty());
    let _managed = support::managed_in(&admin, "live", &double.url, "hidden", path).await;
    let public = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let public_url = format!("http://{}", public.local_addr().unwrap());
    let private = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let private_url = format!("http://{}", private.local_addr().unwrap());
    let (stop, _) = tokio::sync::watch::channel(false);
    let mut s1 = stop.subscribe();
    let mut s2 = stop.subscribe();
    let t1 = tokio::spawn(screenroom_user::serve(
        service.clone(),
        public,
        async move {
            let _ = s1.changed().await;
        },
        vec![],
    ));
    let t2 = tokio::spawn(screenroom_user::serve_private(
        service.clone(),
        private,
        async move {
            let _ = s2.changed().await;
        },
    ));
    let mut client = user_service_client::UserServiceClient::new(
        tonic::transport::Endpoint::from_shared(public_url.clone())
            .unwrap()
            .connect()
            .await
            .unwrap(),
    );
    assert_eq!(
        client
            .list_jellyfin_sources(ListJellyfinSourcesRequest::default())
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    assert_eq!(
        client
            .list_jellyfin_sources(selected(ListJellyfinSourcesRequest::default(), disabled))
            .await
            .unwrap_err()
            .code(),
        Code::FailedPrecondition
    );
    let sources = client
        .list_jellyfin_sources(selected(ListJellyfinSourcesRequest::default(), alice))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(sources.sources.len(), 1);
    assert_eq!(sources.sources[0].id, "live");
    let discovery = ListJellyfinProfilesRequest {
        source_id: "live".into(),
        page: Some(Page {
            size: 1,
            token: String::new(),
        }),
    };
    assert_eq!(
        client
            .list_jellyfin_profiles(selected(
                ListJellyfinProfilesRequest {
                    source_id: "off".into(),
                    page: None
                },
                alice
            ))
            .await
            .unwrap_err()
            .code(),
        Code::Unavailable
    );
    assert_eq!(
        client
            .list_jellyfin_sources(selected(
                ListJellyfinSourcesRequest {
                    page: Some(Page {
                        size: 201,
                        token: String::new()
                    })
                },
                alice
            ))
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    let profiles = client
        .list_jellyfin_profiles(selected(discovery.clone(), alice))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(profiles.profiles[0].id, "hidden");
    assert!(!profiles.next_page_token.is_empty());
    let mut next_profiles = discovery;
    next_profiles.page.as_mut().unwrap().token = profiles.next_page_token;
    assert_eq!(
        client
            .list_jellyfin_profiles(selected(next_profiles.clone(), bob))
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    assert_eq!(
        client
            .list_jellyfin_profiles(selected(next_profiles, alice))
            .await
            .unwrap()
            .into_inner()
            .profiles[0]
            .id,
        "second"
    );
    for profile in ["disabled", "missing"] {
        assert!(
            client
                .set_jellyfin_link(selected(
                    SetJellyfinLinkRequest {
                        source_id: "live".into(),
                        jellyfin_user_id: profile.into()
                    },
                    alice
                ))
                .await
                .is_err()
        );
    }
    for id in [alice, bob] {
        let linked = client
            .set_jellyfin_link(selected(
                SetJellyfinLinkRequest {
                    source_id: "live".into(),
                    jellyfin_user_id: "hidden".into(),
                },
                id,
            ))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(linked.link.unwrap().jellyfin_user_id, "hidden");
    }
    let mut progress = PlaybackStateServiceClient::new(
        tonic::transport::Endpoint::from_shared(private_url)
            .unwrap()
            .connect()
            .await
            .unwrap(),
    );
    let request = GetContinueWatchingRequest {
        user_id: alice.to_string(),
        scopes: vec![
            SourceLibraryScope {
                source_id: "live".into(),
                library_id: String::new(),
            },
            SourceLibraryScope {
                source_id: "unsupported".into(),
                library_id: String::new(),
            },
        ],
        page_size: 50,
        page_token: String::new(),
    };
    let first = progress
        .get_continue_watching(request.clone())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(first.entries.len(), 50);
    let mut next = request.clone();
    next.page_token = first.next_page_token;
    let second = progress
        .get_continue_watching(next.clone())
        .await
        .unwrap()
        .into_inner();
    assert_eq!(second.entries.len(), 50);
    let mut last = next.clone();
    last.page_token = second.next_page_token;
    assert_eq!(
        progress
            .get_continue_watching(last)
            .await
            .unwrap()
            .into_inner()
            .entries
            .len(),
        28
    );
    let mut scoped = request.clone();
    double.changed.store(true, Ordering::SeqCst);
    assert_eq!(
        progress
            .get_continue_watching(next.clone())
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    double.changed.store(false, Ordering::SeqCst);
    double.churn.store(true, Ordering::SeqCst);
    assert_eq!(
        progress
            .get_continue_watching(request.clone())
            .await
            .unwrap_err()
            .code(),
        Code::Unavailable
    );
    double.churn.store(false, Ordering::SeqCst);
    double.changed.store(false, Ordering::SeqCst);
    scoped.scopes = vec![SourceLibraryScope {
        source_id: "live".into(),
        library_id: library,
    }];
    scoped.page_size = 200;
    assert_eq!(
        progress
            .get_continue_watching(scoped)
            .await
            .unwrap()
            .into_inner()
            .entries
            .len(),
        125
    );
    support::clear_link(&fixture.user, bob, "live")
        .await
        .unwrap();
    let mut bobs = request.clone();
    bobs.user_id = bob.to_string();
    assert!(
        progress
            .get_continue_watching(bobs)
            .await
            .unwrap()
            .into_inner()
            .entries
            .is_empty()
    );
    double.pause.store(true, Ordering::SeqCst);
    let mut concurrent = progress.clone();
    let concurrent_request = request.clone();
    let read =
        tokio::spawn(async move { concurrent.get_continue_watching(concurrent_request).await });
    double.entered.notified().await;
    support::set_link(&fixture.user, alice, "live", "second")
        .await
        .unwrap();
    support::set_link(&fixture.user, alice, "live", "hidden")
        .await
        .unwrap();
    double.release.notify_one();
    assert_eq!(read.await.unwrap().unwrap_err().code(), Code::Aborted);
    assert_eq!(
        progress
            .get_continue_watching(next)
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    double.fault.store(true, Ordering::SeqCst);
    let error = progress.get_continue_watching(request).await.unwrap_err();
    assert_eq!(error.code(), Code::Unavailable);
    assert!(!error.message().contains("upstream-private"));
    assert!(
        client
            .get_jellyfin_link(selected(
                GetJellyfinLinkRequest {
                    source_id: "live".into()
                },
                alice
            ))
            .await
            .unwrap()
            .into_inner()
            .link
            .is_some()
    );
    assert!(
        client
            .list_jellyfin_profiles(selected(
                ListJellyfinProfilesRequest {
                    source_id: "live".into(),
                    page: None
                },
                alice
            ))
            .await
            .is_err()
    );
    for source in ["live", "live", "off", "missing"] {
        client
            .clear_jellyfin_link(selected(
                ClearJellyfinLinkRequest {
                    source_id: source.into(),
                },
                alice,
            ))
            .await
            .unwrap();
    }
    assert!(
        client
            .get_jellyfin_link(selected(
                GetJellyfinLinkRequest {
                    source_id: "live".into()
                },
                alice
            ))
            .await
            .unwrap()
            .into_inner()
            .link
            .is_none()
    );
    let mut private_on_public = PlaybackStateServiceClient::new(
        tonic::transport::Endpoint::from_shared(public_url)
            .unwrap()
            .connect()
            .await
            .unwrap(),
    );
    assert_eq!(
        private_on_public
            .get_continue_watching(GetContinueWatchingRequest::default())
            .await
            .unwrap_err()
            .code(),
        Code::Unimplemented
    );
    stop.send(true).unwrap();
    for request in double.requests.lock().unwrap().iter() {
        for id in [alice, bob, disabled] {
            assert!(!request.contains(&id.to_string()));
        }
    }
    t1.await.unwrap().unwrap();
    t2.await.unwrap().unwrap();
    fixture.cleanup().await;
}
