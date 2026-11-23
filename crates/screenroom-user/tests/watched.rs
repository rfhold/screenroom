use screenroom_backend::{
    Uuid,
    chrono::Utc,
    sqlx::{self, PgPool, postgres::PgPoolOptions},
    watched::{Binding, WatchedRepository},
};
use screenroom_catalog::Catalog;
use screenroom_catalog_adapter::user_state::{
    self, Acquisition, UserStateAdapter, UserStateScope, WatchedObservation, WatchedSnapshot,
};
use screenroom_proto::{
    catalog::v1::{
        browse_service_server::BrowseService, catalog_service_server::CatalogService,
        search_service_server::SearchService, *,
    },
    common::v1::{SearchMethods, SearchTypes},
};
use screenroom_user::{repository::Repository, watched::sync_binding};
mod support;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct OtherProvider {
    calls: Arc<AtomicUsize>,
    mode: usize,
}

struct Resume;
#[tonic::async_trait]
impl screenroom_proto::user::internal::v1::playback_state_service_server::PlaybackStateService
    for Resume
{
    async fn get_continue_watching(
        &self,
        _: tonic::Request<screenroom_proto::user::internal::v1::GetContinueWatchingRequest>,
    ) -> Result<
        tonic::Response<screenroom_proto::user::internal::v1::GetContinueWatchingResponse>,
        tonic::Status,
    > {
        use screenroom_proto::user::internal::v1::{
            ContinueWatchingEntry, GetContinueWatchingResponse,
        };
        Ok(tonic::Response::new(GetContinueWatchingResponse {
            entries: vec![ContinueWatchingEntry {
                source_id: "chosen".into(),
                external_item_id: "a".into(),
                progress_percent: 42,
                updated_at: Utc::now().to_rfc3339(),
                ..Default::default()
            }],
            next_page_token: String::new(),
        }))
    }
}
impl UserStateAdapter for OtherProvider {
    fn supports_watched(&self) -> bool {
        self.mode != 3
    }
    fn acquire_watched<'a>(&'a self, scope: &'a UserStateScope) -> Acquisition<'a> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(scope.profile_id, "other-profile");
            match self.mode {
                1 => Err(user_state::Error::Incomplete),
                2 => Err(user_state::Error::Unavailable),
                4 => Err(user_state::Error::ProfileUnavailable),
                _ => WatchedSnapshot::complete(vec![
                    WatchedObservation {
                        external_item_id: "a".into(),
                        watched: Some(true),
                    },
                    WatchedObservation {
                        external_item_id: "b".into(),
                        watched: Some(false),
                    },
                    WatchedObservation {
                        external_item_id: "c".into(),
                        watched: None,
                    },
                    WatchedObservation {
                        external_item_id: "episode".into(),
                        watched: Some(true),
                    },
                ]),
            }
        })
    }
}

async fn role(pool: &PgPool, role: &'static str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(2)
        .after_connect(move |connection, _| {
            Box::pin(async move {
                sqlx::query(&format!("SET ROLE {role}"))
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap()
}
fn selected<T>(value: T, user: Uuid) -> tonic::Request<T> {
    let mut request = tonic::Request::new(value);
    request
        .metadata_mut()
        .insert("x-screenroom-user-id", user.to_string().parse().unwrap());
    request
}
fn scope(source: &str) -> screenroom_proto::common::v1::SourceLibraryScope {
    screenroom_proto::common::v1::SourceLibraryScope {
        source_id: source.into(),
        library_id: String::new(),
    }
}

#[tokio::test]
#[ignore = "requires explicitly configured disposable PostgreSQL 18 + pgvector admin URL"]
async fn persisted_watched_engine_and_all_public_reads() {
    let url = std::env::var("SCREENROOM_TEST_DATABASE_URL").unwrap();
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .unwrap();
    let name = format!("screenroom_watched_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    let options = admin.connect_options().as_ref().clone().database(&name);
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .connect_with(options)
        .await
        .unwrap();
    screenroom_backend::db::migrate(&pool).await.unwrap();
    let user_pool = role(&pool, "screenroom_user").await;
    let catalog_pool = role(&pool, "screenroom_catalog").await;
    let users = Repository::new(user_pool.clone());
    let alice = Uuid::new_v4();
    let bob = Uuid::new_v4();
    users.init_user(alice, "Alice").await.unwrap();
    users.init_user(bob, "Bob").await.unwrap();
    support::set_link(&user_pool, alice, "chosen", "other-profile")
        .await
        .unwrap();
    support::set_link(&user_pool, bob, "chosen", "other-profile")
        .await
        .unwrap();
    let source: Uuid = sqlx::query_scalar(
        "INSERT INTO catalog.sources(source_key,provider) VALUES('chosen','other') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let other_source: Uuid = sqlx::query_scalar(
        "INSERT INTO catalog.sources(source_key,provider) VALUES('other','other') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let library: Uuid = sqlx::query_scalar("INSERT INTO catalog.libraries(source_id,public_id,external_id,title) VALUES($1,'lib','lib','Library') RETURNING id").bind(source).fetch_one(&pool).await.unwrap();
    let mut source_items = vec![];
    for (id, kind, parent) in [
        ("a", "movie", None),
        ("b", "movie", None),
        ("c", "movie", None),
        ("series", "series", None),
        ("season", "season", Some("series")),
        ("episode", "episode", Some("season")),
    ] {
        let item: Uuid = sqlx::query_scalar("INSERT INTO catalog.items(public_id,kind,parent_id) VALUES($1,$2,(SELECT id FROM catalog.items WHERE public_id=$3)) RETURNING id")
            .bind(id).bind(kind).bind(parent).fetch_one(&pool).await.unwrap();
        let row: Uuid = sqlx::query_scalar("INSERT INTO catalog.source_items(source_id,item_id,external_id,metadata,content_hash) VALUES($1,$2,$3,jsonb_build_object('title',$3::text),decode(repeat('00',32),'hex')) RETURNING id")
            .bind(source).bind(item).bind(id).fetch_one(&pool).await.unwrap();
        source_items.push(row);
        sqlx::query("INSERT INTO search.documents(source_item_id,document_version,content_hash,text,title) VALUES($1,1,decode(repeat('00',32),'hex'),$2,$2)").bind(row).bind(if id=="a" {"Alpha"} else {id}).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO catalog.library_items(library_id,source_item_id,source_id,last_seen_run_id) VALUES($1,$2,$3,$4)").bind(library).bind(row).bind(source).bind(Uuid::new_v4()).execute(&pool).await.unwrap();
        // Same canonical item, different source, same external ID: never borrow its personal state.
        sqlx::query("INSERT INTO catalog.source_items(source_id,item_id,external_id,metadata,content_hash) VALUES($1,$2,$3,jsonb_build_object('title',$3::text),decode(repeat('00',32),'hex'))")
            .bind(other_source).bind(item).bind(id).execute(&pool).await.unwrap();
    }
    let binding = Binding {
        user_id: alice,
        source_id: source,
        revision: users.revision(alice).await.unwrap(),
    };
    let repository = WatchedRepository::new(user_pool.clone());
    let reader = WatchedRepository::new(catalog_pool.clone());
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = OtherProvider {
        calls: calls.clone(),
        mode: 0,
    };
    let acquisition = UserStateScope {
        profile_id: "other-profile".into(),
        library_ids: vec!["lib".into()],
    };
    sync_binding(&repository, &binding, &provider, &acquisition)
        .await
        .unwrap();
    let before: String = sqlx::query_scalar(
        "SELECT fetched_at::text FROM personal.watched_snapshots WHERE user_id=$1",
    )
    .bind(alice)
    .fetch_one(&pool)
    .await
    .unwrap();
    for mode in [1, 2] {
        assert!(
            sync_binding(
                &repository,
                &binding,
                &OtherProvider {
                    calls: calls.clone(),
                    mode
                },
                &acquisition
            )
            .await
            .is_err()
        );
    }
    let after: String = sqlx::query_scalar(
        "SELECT fetched_at::text FROM personal.watched_snapshots WHERE user_id=$1",
    )
    .bind(alice)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(before, after, "failure cannot refresh old observations");
    assert_eq!(reader.read(alice, &source_items).await.unwrap().len(), 3);
    assert!(reader.read(bob, &source_items).await.unwrap().is_empty());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop_resume, rx) = tokio::sync::oneshot::channel();
    let resume=tokio::spawn(tonic::transport::Server::builder().add_service(screenroom_proto::user::internal::v1::playback_state_service_server::PlaybackStateServiceServer::new(Resume)).serve_with_incoming_shutdown(tokio_stream::wrappers::TcpListenerStream::new(listener),async {let _=rx.await;}));
    let catalog = Catalog::new(
        catalog_pool.clone(),
        "http://127.0.0.1:1".into(),
        None,
        Some(format!("http://{address}")),
    )
    .unwrap();
    let get = || GetItemRequest {
        id: "a".into(),
        scope: Some(scope("chosen")),
    };
    assert_eq!(
        catalog
            .get_item(selected(get(), alice))
            .await
            .unwrap()
            .into_inner()
            .item
            .unwrap()
            .watched,
        Some(true)
    );
    assert_eq!(
        catalog
            .get_item(selected(
                GetItemRequest {
                    id: "b".into(),
                    ..get()
                },
                alice
            ))
            .await
            .unwrap()
            .into_inner()
            .item
            .unwrap()
            .watched,
        Some(false)
    );
    for request in [
        tonic::Request::new(get()),
        selected(get(), bob),
        selected(
            GetItemRequest {
                scope: Some(scope("other")),
                ..get()
            },
            alice,
        ),
    ] {
        assert_eq!(
            catalog
                .get_item(request)
                .await
                .unwrap()
                .into_inner()
                .item
                .unwrap()
                .watched,
            None
        );
    }
    let home = catalog
        .get_home(selected(
            GetHomeRequest {
                source_id: "chosen".into(),
                library_id: String::new(),
            },
            alice,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        home.rows[1]
            .items
            .iter()
            .find(|i| i.id == "a")
            .unwrap()
            .watched,
        Some(true)
    );
    assert_eq!(home.rows[0].items[0].watched, Some(true));
    assert_eq!(home.rows[0].items[0].progress_percent, Some(42));
    let mut token = String::new();
    let mut found = vec![];
    loop {
        let response = catalog
            .list_library_items(selected(
                ListLibraryItemsRequest {
                    source_id: "chosen".into(),
                    library_id: "lib".into(),
                    page: Some(screenroom_proto::catalog::v1::Page { size: 1, token }),
                },
                alice,
            ))
            .await
            .unwrap()
            .into_inner();
        found.extend(response.items);
        token = response.next_page_token;
        if token.is_empty() {
            break;
        }
    }
    assert_eq!(
        found.iter().find(|i| i.id == "a").unwrap().watched,
        Some(true)
    );
    assert_eq!(
        found.iter().find(|i| i.id == "b").unwrap().watched,
        Some(false)
    );
    let children = catalog
        .list_children(selected(
            ListChildrenRequest {
                parent_id: "season".into(),
                scope: Some(scope("chosen")),
                page: None,
            },
            alice,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(children.items[0].watched, Some(true));
    let search = catalog
        .search(selected(
            SearchRequest {
                query: "Alpha".into(),
                source_id: "chosen".into(),
                methods: Some(SearchMethods {
                    text: true,
                    fuzzy: false,
                    semantic: false,
                    title_only: true,
                }),
                types: Some(SearchTypes {
                    movie: true,
                    series: false,
                    episode: false,
                }),
                ..Default::default()
            },
            alice,
        ))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        search.items.iter().find(|i| i.id == "a").unwrap().watched,
        Some(true)
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "public reads never acquire personal state"
    );
    let mut invalid = tonic::Request::new(get());
    invalid
        .metadata_mut()
        .insert("x-screenroom-user-id", "bad".parse().unwrap());
    assert_eq!(
        catalog.get_item(invalid).await.unwrap_err().code(),
        tonic::Code::InvalidArgument
    );

    let conflicting = WatchedSnapshot::complete(vec![WatchedObservation {
        external_item_id: "a".into(),
        watched: Some(false),
    }])
    .unwrap();
    for binding in [
        Binding {
            user_id: bob,
            revision: users.revision(bob).await.unwrap(),
            ..binding.clone()
        },
        Binding {
            source_id: other_source,
            ..binding.clone()
        },
    ] {
        repository
            .publish(
                &repository.begin_acquisition(&binding).await.unwrap(),
                &conflicting,
                Utc::now(),
            )
            .await
            .unwrap();
    }
    assert!(reader.read(alice, &source_items).await.unwrap()[&source_items[0]]);
    assert!(!reader.read(bob, &source_items).await.unwrap()[&source_items[0]]);
    let other_item: Uuid = sqlx::query_scalar(
        "SELECT id FROM catalog.source_items WHERE source_id=$1 AND external_id='a'",
    )
    .bind(other_source)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!reader.read(alice, &[other_item]).await.unwrap()[&other_item]);
    assert_eq!(
        catalog
            .get_item(selected(
                GetItemRequest {
                    scope: Some(scope("other")),
                    ..get()
                },
                alice
            ))
            .await
            .unwrap()
            .into_inner()
            .item
            .unwrap()
            .watched,
        Some(false)
    );

    let old_acquisition = repository.begin_acquisition(&binding).await.unwrap();
    support::set_link(&user_pool, alice, "chosen", "changed")
        .await
        .unwrap();
    support::set_link(&user_pool, alice, "chosen", "other-profile")
        .await
        .unwrap();
    assert!(reader.read(alice, &source_items).await.unwrap().is_empty());
    assert!(
        repository
            .publish(
                &old_acquisition,
                &provider.acquire_watched(&acquisition).await.unwrap(),
                Utc::now()
            )
            .await
            .is_err(),
        "remap-and-back fences in-flight acquisition"
    );
    let binding = Binding {
        revision: users.revision(alice).await.unwrap(),
        ..binding
    };
    sync_binding(&repository, &binding, &provider, &acquisition)
        .await
        .unwrap();
    sqlx::query("UPDATE personal.watched_snapshots SET fetched_at=now()-interval '901 seconds'")
        .execute(&pool)
        .await
        .unwrap();
    assert!(reader.read(alice, &source_items).await.unwrap().is_empty());
    sync_binding(&repository, &binding, &provider, &acquisition)
        .await
        .unwrap();
    sqlx::query("UPDATE identity.users SET state='disabled' WHERE id=$1")
        .bind(alice)
        .execute(&pool)
        .await
        .unwrap();
    assert!(reader.read(alice, &source_items).await.unwrap().is_empty());
    sqlx::query("UPDATE identity.users SET state='enabled' WHERE id=$1")
        .bind(alice)
        .execute(&pool)
        .await
        .unwrap();
    for mode in [3, 4] {
        sync_binding(
            &repository,
            &binding,
            &OtherProvider {
                calls: calls.clone(),
                mode,
            },
            &acquisition,
        )
        .await
        .unwrap();
        assert!(reader.read(alice, &source_items).await.unwrap().is_empty());
        sync_binding(&repository, &binding, &provider, &acquisition)
            .await
            .unwrap();
    }
    support::clear_link(&user_pool, alice, "chosen")
        .await
        .unwrap();
    assert!(reader.read(alice, &source_items).await.unwrap().is_empty());
    scheduled_jellyfin_and_shutdown(&pool, &user_pool, alice).await;
    stop_resume.send(()).unwrap();
    resume.await.unwrap().unwrap();
    user_pool.close().await;
    catalog_pool.close().await;
    pool.close().await;
    sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}

async fn scheduled_jellyfin_and_shutdown(pool: &PgPool, user_pool: &PgPool, user: Uuid) {
    use screenroom_user::UserService;
    use std::{sync::atomic::AtomicBool, time::Duration};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let source: Uuid = sqlx::query_scalar("INSERT INTO catalog.sources(source_key,provider) VALUES('scheduled','jellyfin') RETURNING id").fetch_one(pool).await.unwrap();
    sqlx::query("INSERT INTO catalog.libraries(source_id,public_id,external_id,title) VALUES($1,'scheduled-lib','selected-lib','Scheduled')").bind(source).execute(pool).await.unwrap();
    let repository = Repository::new(user_pool.clone());
    support::set_link(user_pool, user, "scheduled", "profile")
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let pause = Arc::new(AtomicBool::new(false));
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let requests = Arc::new(AtomicUsize::new(0));
    let identities = Arc::new(AtomicUsize::new(0));
    let identity_count = identities.clone();
    let (p, e, r, count) = (
        pause.clone(),
        entered.clone(),
        release.clone(),
        requests.clone(),
    );
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = vec![0; 8192];
            let n = socket.read(&mut bytes).await.unwrap();
            let request = String::from_utf8(bytes[..n].to_vec()).unwrap();
            count.fetch_add(1, Ordering::SeqCst);
            if p.load(Ordering::SeqCst) {
                e.notify_one();
                r.notified().await;
            }
            let body = if request.starts_with("GET /System/Info ") {
                identity_count.fetch_add(1, Ordering::SeqCst);
                r#"{"Id":"user-test","ServerName":"Owned double","Version":"10.11.0"}"#
            } else if request.starts_with("GET /Users ") {
                r#"[{"Id":"profile","Name":"Viewer","Policy":{"IsDisabled":false,"IsHidden":false}}]"#
            } else {
                assert!(request.starts_with("GET /Users/profile/Items?"));
                assert!(request.contains("ParentId=selected-lib"));
                r#"{"Items":[{"Id":"a","Type":"Movie","UserData":{"Played":true}}],"TotalRecordCount":1,"StartIndex":0}"#
            };
            let _ = socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await;
        }
    });
    let managed =
        support::managed(pool, "scheduled", &format!("http://{address}"), "profile").await;
    let service = UserService {
        repository,
        connections: screenroom_backend::connections::ConnectionRepository::new(
            user_pool.clone(),
            Some(managed.directory.clone()),
        ),
    };
    let (stop, rx) = tokio::sync::watch::channel(false);
    let worker = tokio::spawn(service.clone().run_watched_sync(rx));
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let complete: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM personal.watched_snapshots WHERE source_id=$1)",
            )
            .bind(source)
            .fetch_one(pool)
            .await
            .unwrap();
            if complete {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        requests.load(Ordering::SeqCst),
        5,
        "separate initial tick verifies identity and acquires two complete passes"
    );
    assert_eq!(identities.load(Ordering::SeqCst), 1);
    stop.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(1), worker)
        .await
        .unwrap()
        .unwrap();
    let before: String = sqlx::query_scalar(
        "SELECT fetched_at::text FROM personal.watched_snapshots WHERE source_id=$1",
    )
    .bind(source)
    .fetch_one(pool)
    .await
    .unwrap();
    pause.store(true, Ordering::SeqCst);
    let (stop, rx) = tokio::sync::watch::channel(false);
    let worker = tokio::spawn(service.run_watched_sync(rx));
    tokio::time::timeout(Duration::from_secs(1), entered.notified())
        .await
        .unwrap();
    stop.send(true).unwrap();
    tokio::time::timeout(Duration::from_secs(1), worker)
        .await
        .unwrap()
        .unwrap();
    let after: String = sqlx::query_scalar(
        "SELECT fetched_at::text FROM personal.watched_snapshots WHERE source_id=$1",
    )
    .bind(source)
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(
        before, after,
        "shutdown drops acquisition without publishing partial state"
    );
    release.notify_one();
    server.abort();
    let _ = server.await;
}

#[tokio::test]
#[ignore = "requires task-owned SCREENROOM_TEST_DATABASE_URL"]
async fn onboarding_library_bound_preserves_whole_watched_snapshot() {
    use screenroom_backend::connections::{
        ConnectionRepository, CredentialDirectory, MAX_SELECTED_LIBRARIES, SaveRequest,
    };
    use screenroom_user::UserService;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let fixture = support::Fixture::new().await;
    let repository = Repository::new(fixture.user.clone());
    let local = Uuid::new_v4();
    repository.init_user(local, "Local viewer").await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let counts = Arc::new(std::sync::Mutex::new(vec![0usize; MAX_SELECTED_LIBRARIES]));
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let (calls, observed, last, resume) = (
        requests.clone(),
        counts.clone(),
        entered.clone(),
        release.clone(),
    );
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = vec![0; 8192];
            let n = socket.read(&mut bytes).await.unwrap();
            let request = String::from_utf8(bytes[..n].to_vec()).unwrap();
            assert!(request.contains("owned-onboarding-key"));
            calls.fetch_add(1, Ordering::SeqCst);
            let body = if request.starts_with("GET /System/Info ") {
                serde_json::json!({"Id":"watched-onboarding","ServerName":"Owned server","Version":"10.11.0"})
            } else if request.starts_with("GET /Library/VirtualFolders ") {
                serde_json::Value::Array((0..=MAX_SELECTED_LIBRARIES).map(|i|serde_json::json!({"ItemId":format!("library-{i:02}"),"Name":format!("Library {i}"),"CollectionType":"movies"})).collect())
            } else if request.starts_with("GET /Users ") {
                serde_json::json!([{"Id":"profile","Name":"Viewer","Policy":{"IsDisabled":false,"IsHidden":false}}])
            } else {
                assert!(request.starts_with("GET /Users/profile/Items?"));
                let index = (0..=MAX_SELECTED_LIBRARIES)
                    .find(|i| request.contains(&format!("ParentId=library-{i:02}")))
                    .unwrap();
                assert!(
                    index < MAX_SELECTED_LIBRARIES,
                    "unselected library must never be acquired"
                );
                let finished = {
                    let mut counts = observed.lock().unwrap();
                    counts[index] += 1;
                    counts.iter().sum::<usize>()
                };
                if finished == MAX_SELECTED_LIBRARIES * 2 {
                    last.notify_one();
                    resume.notified().await;
                }
                serde_json::json!({"Items":[{"Id":format!("item-{index:02}"),"Type":"Movie","UserData":{"Played":true}}],"TotalRecordCount":1,"StartIndex":0})
            };
            let body = body.to_string();
            let _ = socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await;
        }
    });
    let path = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("user-onboarding-{}", Uuid::new_v4()));
    let directory = CredentialDirectory::new(path.clone()).unwrap();
    let connections = ConnectionRepository::new(fixture.pool.clone(), Some(directory.clone()));
    let mut request = SaveRequest {
        request_id: Uuid::new_v4(),
        base_url: url,
        api_key: "owned-onboarding-key".into(),
        server_id: "watched-onboarding".into(),
        profile_id: "profile".into(),
        library_ids: (0..=MAX_SELECTED_LIBRARIES)
            .map(|i| format!("library-{i:02}"))
            .collect(),
        local_user_id: local,
        refresh: false,
    };
    assert!(matches!(
        connections.save(&request).await,
        Err(screenroom_backend::Error::Connection("selection_capacity"))
    ));
    assert_eq!(requests.load(Ordering::SeqCst), 0);
    assert!(!path.exists());
    for table in [
        "catalog.sources",
        "catalog.libraries",
        "connections.jellyfin",
        "connections.save_receipts",
        "identity.jellyfin_user_links",
        "ingest.policies",
        "ingest.runs",
        "queue.jobs",
    ] {
        assert_eq!(
            sqlx::query_scalar::<_, i64>(&format!("SELECT count(*) FROM {table}"))
                .fetch_one(&fixture.pool)
                .await
                .unwrap(),
            0,
            "{table}"
        );
    }
    request.library_ids.pop();
    let saved = connections.save(&request).await.unwrap();
    assert!(saved.run_id.is_none());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM catalog.libraries")
            .fetch_one(&fixture.pool)
            .await
            .unwrap(),
        MAX_SELECTED_LIBRARIES as i64
    );
    let service = UserService {
        repository,
        connections: ConnectionRepository::new(fixture.user.clone(), Some(directory)),
    };
    let (stop, rx) = tokio::sync::watch::channel(false);
    let worker = tokio::spawn(service.run_watched_sync(rx));
    tokio::time::timeout(std::time::Duration::from_secs(10), entered.notified())
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM personal.watched_snapshots")
            .fetch_one(&fixture.pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM personal.watched_observations")
            .fetch_one(&fixture.pool)
            .await
            .unwrap(),
        0
    );
    release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM personal.watched_snapshots WHERE complete",
            )
            .fetch_one(&fixture.pool)
            .await
            .unwrap()
                == 1
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(*counts.lock().unwrap(), vec![2; MAX_SELECTED_LIBRARIES]);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM personal.watched_observations WHERE watched"
        )
        .fetch_one(&fixture.pool)
        .await
        .unwrap(),
        MAX_SELECTED_LIBRARIES as i64
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM queue.jobs")
            .fetch_one(&fixture.pool)
            .await
            .unwrap(),
        0
    );
    stop.send(true).unwrap();
    worker.await.unwrap();
    server.abort();
    let _ = server.await;
    for entry in std::fs::read_dir(&path).unwrap() {
        let path = entry.unwrap().path();
        assert_eq!(path.extension().unwrap(), "key");
        assert!(Uuid::parse_str(path.file_stem().unwrap().to_str().unwrap()).is_ok());
        std::fs::remove_file(path).unwrap();
    }
    std::fs::remove_dir(path).unwrap();
    fixture.cleanup().await;
}
