//! Compile-only here. Running requires new, explicit disposable-database authority.
use super::*;
use axum::{
    Router,
    body::Body,
    extract::State,
    http::{Method, StatusCode, header},
    response::Response,
};
use screenroom_backend::{Uuid, db::MIGRATOR};
use serde_json::json;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use tokio::sync::{Mutex, oneshot};

#[derive(Default)]
struct FaultProvider {
    requests: Mutex<Vec<(String, String, Vec<u8>)>>,
    negotiations: AtomicUsize,
    lose_start: AtomicBool,
    lose_exchange: AtomicBool,
}

async fn provider_route(
    State(state): State<Arc<FaultProvider>>,
    request: axum::extract::Request,
) -> Response {
    let (parts, body) = request.into_parts();
    let body = axum::body::to_bytes(body, 65536).await.unwrap().to_vec();
    let path = parts.uri.path();
    let method = parts.method;
    let authorization = parts
        .headers
        .get(header::AUTHORIZATION)
        .unwrap()
        .to_str()
        .unwrap();
    if path.starts_with("/Sessions/Playing") || path.starts_with("/Videos/") {
        assert!(authorization.contains("fixture-user-token"));
        assert!(!authorization.contains("fixture-admin-token"));
    }
    assert!(
        !parts
            .uri
            .query()
            .unwrap_or("")
            .contains("fixture-user-token")
    );
    state
        .requests
        .lock()
        .await
        .push((method.to_string(), parts.uri.to_string(), body));
    let (status,mime,body)=match (method,path) {
        (Method::GET,"/System/Info")=>(200,"application/json",json!({"Id":"fixture-server","ServerName":"Owned","Version":"10.10.7"}).to_string()),
        (Method::GET,"/Users")=>(200,"application/json",json!([{"Id":"fixture-personal","Name":"Personal","Policy":{"IsDisabled":false}}]).to_string()),
        (Method::GET,"/QuickConnect/Enabled")=>(200,"application/json","true".into()),
        (Method::POST,"/QuickConnect/Initiate")=>(200,"application/json",json!({"Secret":"fixture-quick-connect-secret","Code":"123456"}).to_string()),
        (Method::POST,"/QuickConnect/Authorize")=>{
            assert!(parts.uri.query().unwrap().contains("code=123456"));
            assert!(parts.uri.query().unwrap().contains("userId=fixture-personal"));
            (200,"application/json","true".into())
        },
        (Method::POST,"/Users/AuthenticateWithQuickConnect") if state.lose_exchange.load(Ordering::SeqCst)=>(500,"application/json","".into()),
        (Method::POST,"/Users/AuthenticateWithQuickConnect")=>(200,"application/json",json!({"AccessToken":"fixture-user-token","ServerId":"fixture-server","User":{"Id":"fixture-personal"}}).to_string()),
        (Method::GET,"/Users/Me")=>(200,"application/json",json!({"Id":"fixture-personal","Policy":{"IsDisabled":false}}).to_string()),
        (Method::GET,"/Users/fixture-personal/Items/private-fixture-item")=>(200,"application/json",json!({"Id":"private-fixture-item","Type":"Movie","RunTimeTicks":1000000000i64,"UserData":{"PlaybackPositionTicks":90000000},"MediaSources":[{"Id":"private-fixture-source","Protocol":"File"}]}).to_string()),
        (Method::POST,"/Items/private-fixture-item/PlaybackInfo")=>{
            let n=state.negotiations.fetch_add(1,Ordering::SeqCst);
            (200,"application/json",json!({"PlaySessionId":format!("fixture-play-{n}"),"MediaSources":[{"Id":"private-fixture-source","Protocol":"File","Container":"mp4","SupportsDirectPlay":true,"DefaultAudioStreamIndex":1,"RunTimeTicks":1000000000i64,"MediaStreams":[{"Type":"Video","Index":0,"Codec":"h264"},{"Type":"Audio","Index":1,"Codec":"aac"}]}]}).to_string())
        },
        (Method::GET|Method::HEAD,"/Videos/private-fixture-item/stream.mp4")=>{
            if parts.headers.get(header::RANGE).is_some_and(|v|v=="bytes=1000-") {
                return Response::builder().status(416).header(header::CONTENT_RANGE,"bytes */100").header(header::CONTENT_LENGTH,"0").body(Body::empty()).unwrap();
            }
            if parts.headers.get(header::RANGE).is_some() {
                let mut response=Response::builder().status(206).header(header::CONTENT_TYPE,"video/mp4")
                    .header(header::CONTENT_LENGTH,"3").header(header::CONTENT_RANGE,"bytes 0-2/100")
                    .header(header::ACCEPT_RANGES,"bytes").header(header::ETAG,"\"owned\"").body(Body::from("abc")).unwrap();
                response.headers_mut().insert("set-cookie","must-not-forward=secret".parse().unwrap());
                return response;
            }
            (200,"video/mp4","owned-video".into())
        },
        (Method::POST,"/Sessions/Playing") if state.lose_start.load(Ordering::SeqCst)=>(500,"application/json","".into()),
        (Method::POST,"/Sessions/Playing"|"/Sessions/Playing/Progress"|"/Sessions/Playing/Stopped"|"/Sessions/Playing/Ping")=>(204,"application/json","".into()),
        (Method::DELETE,"/Devices"|"/Videos/ActiveEncodings")=>(204,"application/json","".into()),
        _=>panic!("unexpected isolated provider route: {path}"),
    };
    Response::builder()
        .status(StatusCode::from_u16(status).unwrap())
        .header(header::CONTENT_TYPE, mime)
        .header(header::CONTENT_LENGTH, body.len())
        .body(Body::from(body))
        .unwrap()
}

struct Fixture {
    admin: sqlx::PgPool,
    ddl: sqlx::PgPool,
    runtime: sqlx::PgPool,
    name: String,
    user: Uuid,
    service: PlaybackService,
    provider: Arc<FaultProvider>,
    provider_stop: oneshot::Sender<()>,
    provider_task: tokio::task::JoinHandle<()>,
    root: PathBuf,
}

impl Fixture {
    async fn new() -> Self {
        // This function is reachable only from explicitly ignored tests.
        let url = std::env::var("SCREENROOM_PLAYBACK_TEST_DATABASE_URL")
            .expect("explicit disposable target required");
        let admin = sqlx::PgPool::connect(&url).await.unwrap();
        let name = format!("gateway_{}", Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE DATABASE {name}"))
            .execute(&admin)
            .await
            .unwrap();
        let options = admin.connect_options().as_ref().clone().database(&name);
        let ddl = sqlx::postgres::PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options.clone())
            .await
            .unwrap();
        MIGRATOR.run(&ddl).await.unwrap();
        let runtime = sqlx::postgres::PgPoolOptions::new()
            .max_connections(6)
            .after_connect(|connection, _| {
                Box::pin(async move {
                    sqlx::query("SET ROLE screenroom_playback")
                        .execute(connection)
                        .await?;
                    Ok(())
                })
            })
            .connect_with(options)
            .await
            .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let provider = Arc::new(FaultProvider::default());
        let app = Router::new()
            .fallback(provider_route)
            .with_state(provider.clone());
        let (provider_stop, stopped) = oneshot::channel();
        let provider_task = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = stopped.await;
                })
                .await
                .unwrap();
        });
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("gateway-owned-{}", Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let credentials = CredentialDirectory::new(root.join("managed")).unwrap();
        let secrets = PlaybackSecrets::new(root.join("playback")).unwrap();
        let key = Uuid::new_v4();
        PlaybackSecrets::new(root.join("managed"))
            .unwrap()
            .publish(key, "fixture-admin-token")
            .unwrap();
        let user = Uuid::new_v4();
        let source = Uuid::new_v4();
        let connection = Uuid::new_v4();
        let library = Uuid::new_v4();
        let item = Uuid::new_v4();
        let source_item = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO identity.users(id,public_id,display_name) VALUES($1,$2,'Fixture')",
        )
        .bind(user)
        .bind(user.to_string())
        .execute(&ddl)
        .await
        .unwrap();
        sqlx::query("INSERT INTO catalog.sources(id,source_key,provider,configuration) VALUES($1,'fixture-source','jellyfin',$2)")
            .bind(source).bind(json!({"connection_id":connection,"connection_generation":"1","locator_revision":"a".repeat(64)})).execute(&ddl).await.unwrap();
        sqlx::query("INSERT INTO connections.jellyfin(id,source_id,server_id,server_name,server_version,base_url,credential_ref,revision,indexing_profile_id) VALUES($1,$2,'fixture-server','Fixture','10.10.7',$3,$4,$5,'not-personal')")
            .bind(connection).bind(source).bind(format!("http://{address}")).bind(key).bind("a".repeat(64)).execute(&ddl).await.unwrap();
        sqlx::query("INSERT INTO identity.jellyfin_user_links(user_id,source_id,jellyfin_user_id) VALUES($1,'fixture-source','fixture-personal')").bind(user).execute(&ddl).await.unwrap();
        sqlx::query("INSERT INTO catalog.libraries(id,source_id,public_id,external_id,title) VALUES($1,$2,'fixture-library','private-library','Fixture')").bind(library).bind(source).execute(&ddl).await.unwrap();
        sqlx::query(
            "INSERT INTO catalog.items(id,public_id,kind) VALUES($1,'public-fixture-item','movie')",
        )
        .bind(item)
        .execute(&ddl)
        .await
        .unwrap();
        sqlx::query("INSERT INTO catalog.source_items(id,source_id,item_id,external_id,metadata,content_hash) VALUES($1,$2,$3,'private-fixture-item','{}',decode(repeat('00',32),'hex'))")
            .bind(source_item).bind(source).bind(item).execute(&ddl).await.unwrap();
        sqlx::query("INSERT INTO catalog.library_items(library_id,source_item_id,source_id,last_seen_run_id) VALUES($1,$2,$3,$4)")
            .bind(library).bind(source_item).bind(source).bind(Uuid::new_v4()).execute(&ddl).await.unwrap();
        let repository = PlaybackRepository::new(runtime.clone(), Default::default()).unwrap();
        let connections = ConnectionRepository::new(runtime.clone(), Some(credentials.clone()));
        let service = PlaybackService::new(
            repository,
            connections,
            credentials,
            secrets,
            "https://api.test".into(),
        )
        .unwrap();
        Self {
            admin,
            ddl,
            runtime,
            name,
            user,
            service,
            provider,
            provider_stop,
            provider_task,
            root,
        }
    }
    fn request<T>(&self, body: T) -> Request<T> {
        let mut request = Request::new(body);
        request.metadata_mut().insert(
            "x-screenroom-user-id",
            self.user.to_string().parse().unwrap(),
        );
        request
    }
    fn create(&self) -> CreatePlaybackSessionRequest {
        CreatePlaybackSessionRequest {
            request_id: Uuid::new_v4().to_string(),
            item_id: "public-fixture-item".into(),
            scope: Some(screenroom_proto::common::v1::SourceLibraryScope {
                source_id: "fixture-source".into(),
                library_id: "fixture-library".into(),
            }),
            client_capabilities: Some(PlaybackClientCapabilities {
                mp4: true,
                h264: true,
                aac: true,
                ..Default::default()
            }),
            start_policy: PlaybackStartPolicy::Resume as i32,
            position_ms: None,
        }
    }
    async fn close(self) {
        self.provider_stop.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), self.provider_task)
            .await
            .unwrap()
            .unwrap();
        self.runtime.close().await;
        self.ddl.close().await;
        assert!(Uuid::parse_str(self.name.strip_prefix("gateway_").unwrap()).is_ok());
        sqlx::query(&format!("DROP DATABASE {}", self.name))
            .execute(&self.admin)
            .await
            .unwrap();
        self.admin.close().await;
        for directory in ["managed", "playback"] {
            let directory = self.root.join(directory);
            if directory.exists() {
                for entry in std::fs::read_dir(&directory).unwrap() {
                    std::fs::remove_file(entry.unwrap().path()).unwrap();
                }
                std::fs::remove_dir(directory).unwrap();
            }
        }
        std::fs::remove_dir(self.root).unwrap();
    }
}

#[tokio::test]
#[ignore = "database tests explicitly UNRUN; requires separately approved disposable PostgreSQL"]
async fn five_rpc_receipts_ranges_and_transport_revocation_with_owned_fault_provider() {
    use playback_service_server::PlaybackService as _;
    use tower::ServiceExt;
    let f = Fixture::new().await;
    let request = f.create();
    let first = f
        .service
        .create_playback_session(f.request(request.clone()))
        .await
        .unwrap()
        .into_inner()
        .session
        .unwrap();
    assert_eq!(first.transport.as_ref().unwrap().start_position_ms, "9000");
    let get = GetPlaybackSessionRequest {
        session_id: first.session_id.clone(),
        control_capability: first.control_capability.clone(),
    };
    let current = f
        .service
        .get_playback_session(f.request(get.clone()))
        .await
        .unwrap()
        .into_inner()
        .session
        .unwrap();
    assert_eq!(current.transport_revision, 1);
    let update = UpdatePlaybackSessionRequest {
        session_id: first.session_id.clone(),
        control_capability: first.control_capability.clone(),
        request_id: Uuid::new_v4().to_string(),
        expected_transport_revision: 1,
        position_ms: "5000".into(),
        track_choices: None,
        compatible_transcode_fallback: Some(false),
    };
    let replaced = f
        .service
        .update_playback_session(f.request(update.clone()))
        .await
        .unwrap()
        .into_inner()
        .session
        .unwrap();
    let old_url = first
        .transport
        .as_ref()
        .unwrap()
        .media_url
        .strip_prefix("https://api.test")
        .unwrap();
    let new_url = replaced
        .transport
        .as_ref()
        .unwrap()
        .media_url
        .strip_prefix("https://api.test")
        .unwrap();
    assert_ne!(old_url, new_url);
    let app = crate::transport::router(f.service.clone(), vec![]);
    let control_as_media = new_url.replace(
        new_url.split("?cap=").nth(1).unwrap(),
        &first.control_capability,
    );
    assert_eq!(
        app.clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(control_as_media)
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap()
            .status(),
        404
    );
    let mut media_as_control = get.clone();
    media_as_control.control_capability = new_url.split("?cap=").nth(1).unwrap().into();
    assert!(
        f.service
            .get_playback_session(f.request(media_as_control))
            .await
            .is_err()
    );
    assert_eq!(
        app.clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri(old_url)
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap()
            .status(),
        404
    );
    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(new_url)
                .header(header::RANGE, "bytes=0-2")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 206);
    assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes 0-2/100");
    assert!(response.headers().get(header::SET_COOKIE).is_none());
    assert_eq!(
        axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap(),
        "abc"
    );
    let head = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("HEAD")
                .uri(new_url)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(head.status(), 200);
    assert_eq!(head.headers()[header::CONTENT_LENGTH], "11");
    assert!(
        axum::body::to_bytes(head.into_body(), 1024)
            .await
            .unwrap()
            .is_empty()
    );
    let unsatisfiable = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(new_url)
                .header(header::RANGE, "bytes=1000-")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unsatisfiable.status(), 416);
    assert_eq!(
        unsatisfiable.headers()[header::CONTENT_RANGE],
        "bytes */100"
    );
    let before = f.provider.requests.lock().await.len();
    let duplicate = f
        .service
        .create_playback_session(f.request(request))
        .await
        .unwrap()
        .into_inner()
        .session
        .unwrap();
    assert_eq!(duplicate, first);
    let duplicate = f
        .service
        .update_playback_session(f.request(update))
        .await
        .unwrap()
        .into_inner()
        .session
        .unwrap();
    assert_eq!(duplicate, replaced);
    assert_eq!(f.provider.requests.lock().await.len(), before);
    for (seq, position) in [(1, "5000"), (3, "100")] {
        let report = ReportPlaybackEventRequest {
            session_id: first.session_id.clone(),
            control_capability: first.control_capability.clone(),
            transport_revision: 2,
            sequence: seq,
            kind: if seq == 1 {
                PlaybackEventKind::Started as i32
            } else {
                PlaybackEventKind::Seeked as i32
            },
            position_ms: position.into(),
            paused: false,
            muted: false,
            volume: 0.5,
        };
        let response = f
            .service
            .report_playback_event(f.request(report.clone()))
            .await
            .unwrap()
            .into_inner();
        let before = f.provider.requests.lock().await.len();
        assert_eq!(
            f.service
                .report_playback_event(f.request(report))
                .await
                .unwrap()
                .into_inner(),
            response
        );
        assert_eq!(f.provider.requests.lock().await.len(), before);
    }
    let before = f.provider.requests.lock().await.len();
    let older = ReportPlaybackEventRequest {
        session_id: first.session_id.clone(),
        control_capability: first.control_capability.clone(),
        transport_revision: 2,
        sequence: 2,
        kind: PlaybackEventKind::Progress as i32,
        position_ms: "200".into(),
        paused: false,
        muted: false,
        volume: 0.5,
    };
    assert_eq!(
        f.service
            .report_playback_event(f.request(older))
            .await
            .unwrap()
            .into_inner()
            .accepted_sequence,
        3
    );
    let changed = ReportPlaybackEventRequest {
        session_id: first.session_id.clone(),
        control_capability: first.control_capability.clone(),
        transport_revision: 2,
        sequence: 3,
        kind: PlaybackEventKind::Seeked as i32,
        position_ms: "101".into(),
        paused: false,
        muted: false,
        volume: 0.5,
    };
    assert!(
        f.service
            .report_playback_event(f.request(changed))
            .await
            .is_err()
    );
    let heartbeat = ReportPlaybackEventRequest {
        session_id: first.session_id.clone(),
        control_capability: first.control_capability.clone(),
        transport_revision: 2,
        sequence: 4,
        kind: PlaybackEventKind::Heartbeat as i32,
        position_ms: "100".into(),
        paused: false,
        muted: false,
        volume: 0.5,
    };
    let response = f
        .service
        .report_playback_event(f.request(heartbeat.clone()))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        f.service
            .report_playback_event(f.request(heartbeat))
            .await
            .unwrap()
            .into_inner(),
        response
    );
    assert_eq!(f.provider.requests.lock().await.len(), before);
    let stop = StopPlaybackSessionRequest {
        session_id: first.session_id.clone(),
        control_capability: first.control_capability.clone(),
        request_id: Uuid::new_v4().to_string(),
        transport_revision: 2,
        final_sequence: 4,
        position_ms: "100".into(),
        stop_reason: "player-exit".into(),
    };
    let response = f
        .service
        .stop_playback_session(f.request(stop.clone()))
        .await
        .unwrap()
        .into_inner();
    let before = f.provider.requests.lock().await.len();
    assert_eq!(
        f.service
            .stop_playback_session(f.request(stop))
            .await
            .unwrap()
            .into_inner(),
        response
    );
    assert_eq!(f.provider.requests.lock().await.len(), before);
    assert_eq!(
        app.oneshot(
            axum::http::Request::builder()
                .uri(new_url)
                .body(Body::empty())
                .unwrap()
        )
        .await
        .unwrap()
        .status(),
        404
    );
    let public = serde_json::to_string(&first).unwrap();
    for secret in [
        "private-fixture-item",
        "private-fixture-source",
        "fixture-admin-token",
        "fixture-user-token",
        "fixture-play-",
    ] {
        assert!(!public.contains(secret));
    }
    f.close().await;
}

#[tokio::test]
#[ignore = "database tests explicitly UNRUN; requires separately approved disposable PostgreSQL"]
async fn ambiguous_start_is_never_retried_and_later_personal_reports_remain_blocked() {
    use playback_service_server::PlaybackService as _;
    let f = Fixture::new().await;
    let first = f
        .service
        .create_playback_session(f.request(f.create()))
        .await
        .unwrap()
        .into_inner()
        .session
        .unwrap();
    f.provider.lose_start.store(true, Ordering::SeqCst);
    let report = ReportPlaybackEventRequest {
        session_id: first.session_id,
        control_capability: first.control_capability,
        transport_revision: 1,
        sequence: 1,
        kind: PlaybackEventKind::Started as i32,
        position_ms: "9000".into(),
        paused: false,
        muted: false,
        volume: 1.0,
    };
    let response = f
        .service
        .report_playback_event(f.request(report.clone()))
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        response.report_outcome,
        PlaybackReportOutcome::Uncertain as i32
    );
    let before = f.provider.requests.lock().await.len();
    assert_eq!(
        f.service
            .report_playback_event(f.request(report.clone()))
            .await
            .unwrap()
            .into_inner(),
        response
    );
    let later = ReportPlaybackEventRequest {
        sequence: 2,
        kind: PlaybackEventKind::Progress as i32,
        ..report
    };
    assert!(
        f.service
            .report_playback_event(f.request(later))
            .await
            .is_err()
    );
    assert_eq!(f.provider.requests.lock().await.len(), before);
    let statuses: Vec<String> = sqlx::query_scalar("SELECT status FROM playback.effects")
        .fetch_all(&f.ddl)
        .await
        .unwrap();
    assert_eq!(statuses, vec!["unknown"]);
    f.close().await;
}

#[tokio::test]
#[ignore = "database tests explicitly UNRUN; requires separately approved disposable PostgreSQL"]
async fn losing_exclusive_runtime_ownership_revokes_dispatch_before_watchdog() {
    let f = Fixture::new().await;
    let mut connection = f.runtime.acquire().await.unwrap();
    let owned: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock(1935897197,1)")
        .fetch_one(&mut *connection)
        .await
        .unwrap();
    assert!(owned);
    let ownership = crate::runtime::RuntimeOwnership::new(connection, f.service.stopping.clone());
    use screenroom_catalog_adapter::jellyfin::playback::{DispatchDenial, DispatchPurpose};
    assert_eq!(ownership.check(DispatchPurpose::Active).await, Ok(()));
    ownership.unlock_for_test().await;
    assert_eq!(
        ownership.check(DispatchPurpose::Active).await,
        Err(DispatchDenial::Unavailable)
    );
    assert!(f.service.stopping.load(Ordering::Acquire));
    drop(ownership);
    f.close().await;
}

#[tokio::test]
#[ignore = "database tests explicitly UNRUN; requires separately approved disposable PostgreSQL"]
async fn lost_exchange_restart_cleans_exact_owned_device_after_source_disable_without_auth_replay()
{
    use playback_service_server::PlaybackService as _;
    let f = Fixture::new().await;
    f.provider.lose_exchange.store(true, Ordering::SeqCst);
    let request = f.create();
    let error = f
        .service
        .create_playback_session(f.request(request.clone()))
        .await
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::Unavailable);
    assert_eq!(error.message(), "upstream_unavailable");
    let before = f.provider.requests.lock().await.len();
    assert_eq!(
        f.service
            .create_playback_session(f.request(request))
            .await
            .unwrap_err()
            .message(),
        error.message()
    );
    assert_eq!(f.provider.requests.lock().await.len(), before);
    let (id, device, credential, replay_ref, quick_connect_ref): (Uuid, String, Uuid, Uuid, String) =
        sqlx::query_as("SELECT id,device_id,credential_ref,replay_ref,capabilities->>'_quick_connect_ref' FROM playback.sessions")
            .fetch_one(&f.ddl)
            .await
            .unwrap();
    assert!(f.service.secrets.read(credential).is_err());
    let quick_connect_ref = Uuid::parse_str(&quick_connect_ref).unwrap();
    assert_eq!(
        f.service.secrets.read(quick_connect_ref).unwrap(),
        "fixture-quick-connect-secret"
    );
    let replay = f.service.secrets.read(replay_ref).unwrap();
    assert!(!replay.contains("fixture-quick-connect-secret"));
    sqlx::query("UPDATE catalog.sources SET state='disabled'")
        .execute(&f.ddl)
        .await
        .unwrap();
    f.service.repository.recover_abandoned().await.unwrap();
    assert_eq!(f.provider.requests.lock().await.len(), before);
    f.service.cleanup_once().await;
    let requests = f.provider.requests.lock().await;
    let deletions: Vec<_> = requests
        .iter()
        .filter(|(method, _, _)| method == "DELETE")
        .collect();
    assert_eq!(deletions.len(), 1);
    assert_eq!(deletions[0].1, format!("/Devices?id={device}"));
    assert_eq!(
        requests
            .iter()
            .filter(|(_, uri, _)| uri == "/Users/AuthenticateWithQuickConnect")
            .count(),
        1
    );
    assert!(
        !requests
            .iter()
            .any(|(_, uri, _)| uri.starts_with("/Sessions/Playing"))
    );
    drop(requests);
    let state: String =
        sqlx::query_scalar("SELECT state FROM playback.cleanup WHERE revoke_device")
            .fetch_one(&f.ddl)
            .await
            .unwrap();
    assert_eq!(state, "complete");
    sqlx::query(
        "UPDATE playback.sessions SET updated_at=clock_timestamp()-interval '3 days' WHERE id=$1",
    )
    .bind(id)
    .execute(&f.ddl)
    .await
    .unwrap();
    f.service
        .repository
        .retain(chrono::Utc::now() - chrono::Duration::days(2))
        .await
        .unwrap();
    let proofs = f.service.repository.retired_credentials().await.unwrap();
    assert_eq!(proofs.len(), 3);
    for proof in proofs {
        f.service.secrets.remove_retired(&proof).unwrap();
        f.service
            .repository
            .finish_credential_gc(&proof)
            .await
            .unwrap();
    }
    assert!(f.service.secrets.read(quick_connect_ref).is_err());
    assert!(f.service.secrets.read(replay_ref).is_err());
    f.close().await;
}

#[tokio::test]
#[ignore = "database tests explicitly UNRUN; requires separately approved disposable PostgreSQL"]
async fn crash_before_first_publication_replays_stable_terminal_failure_without_new_authority() {
    use playback_service_server::PlaybackService as _;
    use screenroom_backend::playback::{Create, secrets::Capability};
    let f = Fixture::new().await;
    for recover_first in [false, true] {
        let request = f.create();
        let scope = crate::validation::scope(request.scope.clone()).unwrap();
        let body = json!({"start_policy":1,"position_ms":null});
        let caps = json!({"mp4":true,"h264":true,"aac":true,"native_hls":false,"mse_hls":false,"webvtt":false});
        let control = Capability::generate().unwrap();
        let created = f
            .service
            .repository
            .create(Create {
                user_id: f.user,
                public_item_id: &request.item_id,
                scope: &scope,
                request_id: Uuid::parse_str(&request.request_id).unwrap(),
                normalized_body: &body,
                capabilities: &caps,
                control_hash: &control.hash(),
            })
            .await
            .unwrap();
        // Process loss after committed intent but before the first private publication.
        assert!(f.service.secrets.read(created.session.replay_ref).is_err());
        if recover_first {
            f.service.repository.recover_abandoned().await.unwrap();
        }
        for _ in 0..2 {
            let error = f
                .service
                .create_playback_session(f.request(request.clone()))
                .await
                .unwrap_err();
            assert_eq!(error.code(), tonic::Code::Unavailable);
            assert_eq!(error.message(), "upstream_unavailable");
        }
        let op = f
            .service
            .repository
            .operation_receipt(&created.operation)
            .await
            .unwrap();
        assert_eq!(op.status, "failed");
        assert!(!op.dispatched);
        let (revision, hash): (i64, Vec<u8>) = sqlx::query_as(
            "SELECT transport_revision,control_hash FROM playback.sessions WHERE id=$1",
        )
        .bind(created.session.id)
        .fetch_one(&f.ddl)
        .await
        .unwrap();
        assert_eq!(revision, 0);
        assert_eq!(hash, control.hash());
        let mut altered = request;
        altered.start_policy = PlaybackStartPolicy::Beginning as i32;
        assert_eq!(
            f.service
                .create_playback_session(f.request(altered))
                .await
                .unwrap_err()
                .code(),
            tonic::Code::AlreadyExists
        );
        assert!(f.service.secrets.read(created.session.replay_ref).is_err());
        assert!(f.provider.requests.lock().await.is_empty());
    }
    f.close().await;
}
