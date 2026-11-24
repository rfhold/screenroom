#![cfg(not(target_arch = "wasm32"))]
use screenroom_client_core::*;
use screenroom_proto::operator::v1::{
    OperatorRequest, OperatorResponse,
    operator_service_server::{OperatorService, OperatorServiceServer},
};
use std::{
    net::TcpListener,
    sync::{Arc, Mutex},
    time::Duration,
};
use tonic::{Request, Response, Status};

const ID: &str = "01234567-89ab-cdef-0123-456789abcdef";

struct Server {
    endpoint: String,
    requests: Arc<Mutex<Vec<Call>>>,
    _stop: tokio::sync::oneshot::Sender<()>,
}

#[derive(Clone)]
struct Call {
    rpc: &'static str,
    request: OperatorRequest,
}

impl Call {
    fn body(&self) -> &str {
        std::str::from_utf8(&self.request.payload_json).unwrap()
    }
}

struct Fixture {
    requests: Arc<Mutex<Vec<Call>>>,
    status: u16,
    body: String,
    delay: u64,
    disconnect: bool,
}
impl Fixture {
    async fn respond(
        &self,
        rpc: &'static str,
        request: OperatorRequest,
    ) -> Result<Response<OperatorResponse>, Status> {
        self.requests.lock().unwrap().push(Call { rpc, request });
        tokio::time::sleep(Duration::from_millis(self.delay)).await;
        if self.disconnect {
            return Err(Status::unavailable("unavailable"));
        }
        if self.status == 200 {
            return Ok(Response::new(OperatorResponse {
                payload_json: self.body.as_bytes().to_vec(),
            }));
        }
        let value: serde_json::Value = serde_json::from_str(&self.body).unwrap_or_default();
        let message = value["error"]["code"].as_str().unwrap_or("fixture_invalid");
        let code = match (self.status, message) {
            (403, _) => tonic::Code::PermissionDenied,
            (400, _) => tonic::Code::InvalidArgument,
            (404, _) => tonic::Code::NotFound,
            (409, "source_busy") => tonic::Code::Aborted,
            (409, "stale") => tonic::Code::FailedPrecondition,
            (409, _) => tonic::Code::AlreadyExists,
            (503, _) => tonic::Code::Unavailable,
            _ => tonic::Code::Internal,
        };
        let mut status = Status::new(code, message);
        if let Some(id) = value["error"]["run_id"].as_str() {
            status
                .metadata_mut()
                .insert("x-screenroom-run-id", id.parse().unwrap());
        }
        Err(status)
    }
}

macro_rules! fixture_methods {
    ($($name:ident => $rpc:literal;)+) => {
        #[tonic::async_trait]
        impl OperatorService for Fixture {
            $(async fn $name(&self, request: Request<OperatorRequest>) -> Result<Response<OperatorResponse>, Status> {
                let input = request.into_inner();
                self.respond($rpc, input).await
            })+
        }
    };
}
fixture_methods! {
    health => "Health";
    discover_connection => "DiscoverConnection";
    save_connection => "SaveConnection";
    list_connections => "ListConnections";
    list_sources => "ListSources";
    list_libraries => "ListLibraries";
    list_runs => "ListRuns";
    list_candidates => "ListCandidates";
    list_policies => "ListPolicies";
    configure_policy => "ConfigurePolicy";
    get_progress => "GetProgress";
    start_run => "StartRun";
    get_run_status => "GetRunStatus";
    get_execution => "GetExecution";
    pause_run => "PauseRun";
    resume_run => "ResumeRun";
    cancel_run => "CancelRun";
    list_jobs => "ListJobs";
    list_attempts => "ListAttempts";
    list_redrives => "ListRedrives";
    redrive_job => "RedriveJob";
    observe_queue => "ObserveQueue";
}

impl Server {
    fn new(status: u16, body: String, delay: u64, disconnect: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (stop, done) = tokio::sync::oneshot::channel();
        let fixture = Fixture {
            requests: requests.clone(),
            status,
            body,
            delay,
            disconnect,
        };
        let listener = tokio::net::TcpListener::from_std(listener).unwrap();
        tokio::spawn(async move {
            let incoming = futures::stream::unfold(listener, |listener| async {
                Some((listener.accept().await.map(|(stream, _)| stream), listener))
            });
            tonic::transport::Server::builder()
                .add_service(OperatorServiceServer::new(fixture))
                .serve_with_incoming_shutdown(incoming, async {
                    let _ = done.await;
                })
                .await
                .unwrap();
        });
        Self {
            endpoint,
            requests,
            _stop: stop,
        }
    }
    fn client(&self, timeout_ms: u32) -> Arc<OperatorClient> {
        OperatorClient::new(OperatorClientConfig {
            endpoint: self.endpoint.clone(),
            timeout_ms,
        })
        .unwrap()
    }
}
fn page() -> OperatorPage {
    OperatorPage {
        after: None,
        limit: 2,
    }
}
fn selected() -> OperatorScope {
    OperatorScope {
        library_ids: vec!["library:a".into()],
        item_kinds: vec![OperatorItemKind::Series, OperatorItemKind::Movie],
        item_ids: vec!["item:2".into(), "item:1".into()],
    }
}

fn discovery_fixture() -> serde_json::Value {
    serde_json::json!({"server":{"id":"server","name":"Synthetic","version":"10.10.7"},"libraries":[
        {"id":"movies","name":"Movies","kind":"movies","supported":true},
        {"id":"shows","name":"Shows","kind":"television","supported":true},
        {"id":"mixed","name":"Mixed","kind":"collections","supported":true},
        {"id":"music","name":"Music","kind":"unsupported","supported":false}],
        "profiles":[{"id":"profile","name":"Hidden profile","hidden":true}],"api_key":"not-public","credential_ref":"not-public"})
}
fn saved_fixture(refresh: bool) -> serde_json::Value {
    serde_json::json!({"connection":{"id":ID,"source_id":ID,"source_key":"jellyfin_synthetic","display_name":"Synthetic","server_version":"10.10.7","state":"enabled","credential_ref":"not-public","revision":"not-public"},"policy_id":ID,"policy_version":9007199254740993_i64,"run_id":refresh.then_some(ID),"api_key":"not-public","base_url":"not-public"})
}
fn save_request(refresh: bool) -> OperatorSaveConnectionRequest {
    OperatorSaveConnectionRequest {
        request_id: ID.into(),
        base_url: "http://provider.test/jellyfin".into(),
        api_key: "synthetic-secret".into(),
        server_id: "server".into(),
        profile_id: "profile".into(),
        library_ids: vec!["shows".into(), "movies".into()],
        local_user_id: ID.into(),
        refresh,
    }
}

#[tokio::test]
async fn connection_records_are_redacted_and_transported_only_to_typed_operator_routes() {
    let input = save_request(false);
    let discovery = OperatorDiscoverConnectionRequest {
        base_url: input.base_url.clone(),
        api_key: input.api_key.clone(),
    };
    for debug in [format!("{input:?}"), format!("{discovery:?}")] {
        assert!(debug.contains("REDACTED"));
        assert!(!debug.contains(&input.api_key));
        assert!(!debug.contains(&input.base_url));
    }
    let server = Server::new(200, discovery_fixture().to_string(), 0, false);
    let result = server
        .client(1000)
        .discover_connection(discovery)
        .await
        .unwrap();
    assert_eq!(result.libraries.len(), 4);
    assert!(result.profiles[0].hidden);
    assert!(
        !serde_json::to_string(&result)
            .unwrap()
            .contains("not-public")
    );
    {
        let call = &server.requests.lock().unwrap()[0];
        assert_eq!(call.rpc, "DiscoverConnection");
    }
    for refresh in [false, true] {
        let server = Server::new(200, saved_fixture(refresh).to_string(), 0, false);
        let client = server.client(1000);
        let result = client.save_connection(save_request(refresh)).await.unwrap();
        assert_eq!(result.policy_version, 9007199254740993);
        assert_eq!(result.run_id.is_some(), refresh);
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("not-public")
        );
        client.save_connection(save_request(refresh)).await.unwrap();
        let requests = server.requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        let bodies: Vec<_> = requests.iter().map(|r| r.body()).collect();
        assert_eq!(bodies[0], bodies[1]);
        {
            let call = &requests[0];
            assert_eq!(call.rpc, "SaveConnection");
        }

        let wire: serde_json::Value = serde_json::from_str(bodies[0]).unwrap();
        assert_eq!(
            wire,
            serde_json::json!({"request_id":ID,"base_url":"http://provider.test/jellyfin","api_key":"synthetic-secret","server_id":"server","profile_id":"profile","library_ids":["shows","movies"],"local_user_id":ID,"refresh":refresh})
        );
    }
    let server = Server::new(
        200,
        serde_json::json!({"items":[saved_fixture(false)["connection"]],"next_after":null})
            .to_string(),
        0,
        false,
    );
    let result = server
        .client(1000)
        .connections(OperatorPage {
            after: Some(ID.into()),
            limit: 200,
        })
        .await
        .unwrap();
    assert_eq!(result.items.len(), 1);
    assert!(
        !serde_json::to_string(&result)
            .unwrap()
            .contains("not-public")
    );
    {
        let call = &server.requests.lock().unwrap()[0];
        assert_eq!(call.rpc, "ListConnections");
        let page = call.request.page.as_ref().unwrap();
        assert_eq!(page.limit, 200);
        assert_eq!(page.after.as_deref(), Some(ID));
    }
}

#[test]
fn connection_json_conversion_rejects_extra_fields() {
    assert!(serde_json::from_value::<OperatorDiscoverConnectionRequest>(serde_json::json!({
        "base_url":"http://provider.test/jellyfin", "api_key":"synthetic-secret", "request_id":ID
    })).is_err());
    assert!(
        serde_json::from_value::<OperatorSaveConnectionRequest>(serde_json::json!({
            "request_id":ID,"base_url":"http://provider.test/jellyfin","api_key":"synthetic-secret",
            "server_id":"server","profile_id":"profile","library_ids":["movies"],"local_user_id":ID,
            "refresh":false,"unexpected":true
        }))
        .is_err()
    );
}

#[tokio::test]
async fn connection_validation_and_closed_errors_do_not_expose_credentials() {
    let server = Server::new(200, "{}".into(), 0, false);
    let client = server.client(1000);
    for base_url in [
        "http://user:password@provider.test",
        "http://provider.test?api_key=synthetic-secret",
        "http://provider.test#fragment",
        "file:///private/secret",
        "http://provider.test/synthetic-secret",
    ] {
        let error = client
            .discover_connection(OperatorDiscoverConnectionRequest {
                base_url: base_url.into(),
                api_key: "synthetic-secret".into(),
            })
            .await
            .unwrap_err();
        assert!(!format!("{error:?} {error}").contains("synthetic-secret"));
        assert!(matches!(
            error,
            OperatorError::Connection {
                code: OperatorConnectionErrorCode::InvalidConnection,
                ambiguous: false
            }
        ));
    }
    for key in [
        "".to_owned(),
        "x".repeat(4097),
        "bad\nkey".into(),
        "bad\"key".into(),
    ] {
        assert!(matches!(
            client
                .discover_connection(OperatorDiscoverConnectionRequest {
                    base_url: "http://provider.test".into(),
                    api_key: key
                })
                .await,
            Err(OperatorError::Connection {
                code: OperatorConnectionErrorCode::InvalidCredentials,
                ambiguous: false
            })
        ));
    }
    for count in [0, 33] {
        let mut input = save_request(false);
        input.library_ids = (0..count).map(|i| format!("library{i}")).collect();
        assert!(matches!(
            client.save_connection(input).await,
            Err(OperatorError::Connection {
                ambiguous: false,
                ..
            })
        ));
    }
    let mut input = save_request(false);
    input.library_ids.push(input.library_ids[0].clone());
    assert!(client.save_connection(input).await.is_err());
    assert!(
        client
            .connections(OperatorPage {
                after: None,
                limit: 201
            })
            .await
            .is_err()
    );
    assert!(server.requests.lock().unwrap().is_empty());
    for (status, codes) in [
        (
            400,
            vec![
                "invalid_connection",
                "invalid_discovery",
                "invalid_credentials",
                "credentials_rejected",
                "invalid_server_identity",
                "invalid_selection",
                "profile_unavailable",
                "library_unavailable",
                "unsupported_library",
                "unsupported_library_scope",
                "selection_capacity",
                "local_user_unavailable",
            ],
        ),
        (
            409,
            vec![
                "request_conflict",
                "request_expired",
                "server_identity_mismatch",
                "server_already_connected",
            ],
        ),
        (
            503,
            vec![
                "connection_unavailable",
                "credential_store_unavailable",
                "discovery_unavailable",
            ],
        ),
    ] {
        for code in codes {
            let server = Server::new(status, serde_json::json!({"error":{"code":code,"message":"synthetic-secret http://private/locator"}}).to_string(), 0, false);
            let error = server
                .client(1000)
                .save_connection(save_request(false))
                .await
                .unwrap_err();
            assert!(!format!("{error:?} {error}").contains("synthetic-secret"));
            match error {
                OperatorError::Connection {
                    code: typed,
                    ambiguous,
                } => {
                    assert_eq!(serde_json::to_value(typed).unwrap(), code);
                    assert_eq!(ambiguous, status == 503);
                }
                other => panic!("unexpected sanitized error: {other:?}"),
            }
        }
    }
    for body in [
        r#"{"error":{"code":"synthetic-secret"}}"#,
        r#"{"error":{"code":{"toString":null}}}"#,
    ] {
        let server = Server::new(400, body.into(), 0, false);
        assert!(matches!(
            server
                .client(1000)
                .save_connection(save_request(false))
                .await,
            Err(OperatorError::InvalidResponse { ambiguous: true })
        ));
    }
}

#[tokio::test]
async fn connection_ambiguous_faults_are_one_attempt_and_reflection_is_rejected() {
    let server = Server::new(200, String::new(), 0, true);
    assert!(matches!(
        server
            .client(1000)
            .save_connection(save_request(false))
            .await,
        Err(OperatorError::Unavailable { ambiguous: true })
    ));
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(server.requests.lock().unwrap().len(), 1);
    let server = Server::new(200, saved_fixture(false).to_string(), 100, false);
    assert!(matches!(
        server.client(10).save_connection(save_request(false)).await,
        Err(OperatorError::Deadline { ambiguous: true })
    ));
    let mut reflected = discovery_fixture();
    reflected["server"]["name"] = serde_json::json!("synthetic-secret");
    let server = Server::new(200, reflected.to_string(), 0, false);
    assert!(matches!(
        server
            .client(1000)
            .discover_connection(OperatorDiscoverConnectionRequest {
                base_url: "http://provider.test".into(),
                api_key: "synthetic-secret".into()
            })
            .await,
        Err(OperatorError::InvalidResponse { ambiguous: false })
    ));
    let mut reflected = saved_fixture(false);
    reflected["connection"]["display_name"] = serde_json::json!("synthetic-secret");
    let server = Server::new(200, reflected.to_string(), 0, false);
    assert!(matches!(
        server
            .client(1000)
            .save_connection(save_request(false))
            .await,
        Err(OperatorError::InvalidResponse { ambiguous: true })
    ));
}

fn indexing_configuration_fixture() -> serde_json::Value {
    serde_json::json!({"indexing":1,"enabled":true,"cadence_seconds":60,"mode":"full","components":{"metadata":true,"artwork":true,"embeddings":false},"strategy":"all","scope":{"library_ids":["library:b","library:a"],"item_kinds":["series","movie"],"item_ids":[]},"limits":{"work_requests":10,"work_records":20,"lifetime_requests":1000,"lifetime_records":1000,"deadline_seconds":600,"work_timeout_seconds":30}})
}

fn indexing_policy_fixture() -> serde_json::Value {
    serde_json::json!({"id":ID,"source_id":ID,"version":9007199254740993_i64,"enabled":true,"automatic_runs":false,"next_due_at":"2026-11-12T00:00:00Z","configuration":indexing_configuration_fixture()})
}
fn diagnostics_fixture(capabilities: Option<Vec<String>>) -> serde_json::Value {
    let relative = capabilities.as_ref().map(|_| 9007199254740993_i64);
    serde_json::json!({"atomic":false,"queue_observation":{"started_at":"2026-11-12T00:00:00Z","finished_at":"2026-11-12T00:00:01Z"},"blockers_as_of":"2026-11-12T00:00:02Z","effective_capabilities":capabilities,"worker_liveness":"unknown","queue":{"ready":9007199254740993_i64,"delayed":0,"leased":0,"failed":1,"cancelled":0,"complete":0,"claimable":relative,"reclaimable":relative,"blocked":relative,"exhausted":1,"oldest_ready_seconds":1.25},"blockers":{"deadline_expired":0,"policy_disabled":0,"policy_version":0,"source_disabled":0,"source_changed":0,"source_concurrency":0,"run_state":0,"cooldown":0,"capability":relative,"attempt_limit":1,"next_eligible_at":null,"as_of":"2026-11-12T00:00:02Z"},"count_semantics":{"blocked":"overlaps exhausted when another blocker exists","exhausted":"includes delayed ready jobs at attempt limit"}})
}

#[tokio::test]
async fn indexing_policy_configure_progress_and_execution_use_verified_routes() {
    let server = Server::new(200, indexing_policy_fixture().to_string(), 0, false);
    let configuration: OperatorIndexingConfiguration =
        serde_json::from_value(indexing_configuration_fixture()).unwrap();
    assert!(!configuration.automatic_runs);
    assert!(configuration.artwork.posters.skip_existing);
    assert_eq!(
        (
            configuration.reconcile_seconds,
            configuration.overlap_seconds,
            configuration.source_concurrency
        ),
        (604800, 300, 1)
    );
    let input = OperatorIndexingConfigureRequest {
        request_id: ID.into(),
        existing: Some(OperatorExistingPolicy {
            policy_id: ID.into(),
            expected_version: 9007199254740993,
        }),
        configuration,
    };
    let policy = server
        .client(1000)
        .indexing_configure(ID.into(), input.clone())
        .await
        .unwrap();
    assert_eq!(policy.version, 9007199254740993);
    let requests = server.requests.lock().unwrap().clone();
    {
        let call = &requests[0];
        assert_eq!(call.rpc, "ConfigurePolicy");
        assert_eq!(call.request.source_id, ID);
    }

    assert!(
        requests[0]
            .body()
            .contains("\"expected_version\":9007199254740993")
    );
    assert!(!requests[0].body().contains("operator_profile_id"));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(requests[0].body()).unwrap(),
        serde_json::to_value(input).unwrap()
    );
    let server = Server::new(
        200,
        serde_json::json!({"items":[indexing_policy_fixture()],"next_after":null}).to_string(),
        0,
        false,
    );
    assert_eq!(
        server
            .client(1000)
            .indexing_policies(ID.into(), page())
            .await
            .unwrap()
            .items[0]
            .version,
        9007199254740993
    );
    let server = Server::new(200, r#"{"policy_version":9007199254740993,"watermark":null,"next_reconcile_at":"2026-11-12T00:00:00Z"}"#.into(), 0, false);
    assert_eq!(
        server
            .client(1000)
            .indexing_progress(ID.into(), ID.into())
            .await
            .unwrap()
            .unwrap()
            .policy_version,
        9007199254740993
    );
    {
        let call = &server.requests.lock().unwrap()[0];
        assert_eq!(call.rpc, "GetProgress");
        assert_eq!(call.request.source_id, ID);
        assert_eq!(call.request.policy_id, ID);
    }
    let server = Server::new(200, "null".into(), 0, false);
    assert!(
        server
            .client(1000)
            .indexing_progress(ID.into(), ID.into())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        server
            .client(1000)
            .indexing_execution(ID.into(), ID.into())
            .await
            .unwrap()
            .is_none()
    );
    let server = Server::new(200, r#"{"mode":"incremental","lower_watermark":null,"candidate_watermark":"2026-11-12T00:00:00Z","settled_at":null,"fallback_reason":"unsupported_cursor"}"#.into(), 0, false);
    assert_eq!(
        server
            .client(1000)
            .indexing_execution(ID.into(), ID.into())
            .await
            .unwrap()
            .unwrap()
            .mode,
        OperatorIndexingMode::Incremental
    );
}

#[tokio::test]
async fn indexing_configuration_rejects_unsupported_matrix_and_private_fields_before_dispatch() {
    let server = Server::new(200, "{}".into(), 0, false);
    let client = server.client(1000);
    let mut invalids = Vec::new();
    for (key, value) in [
        ("source_concurrency", serde_json::json!(2)),
        ("strategy", serde_json::json!("missing")),
        ("stale_after_seconds", serde_json::json!(60)),
        ("overlap_seconds", serde_json::json!(0)),
    ] {
        let mut config = indexing_configuration_fixture();
        config[key] = value;
        invalids.push(config);
    }
    for (key, value) in [
        ("work_records", serde_json::json!(1001)),
        ("work_requests", serde_json::json!(6)),
        ("deadline_seconds", serde_json::json!(59)),
        ("work_timeout_seconds", serde_json::json!(241)),
    ] {
        let mut config = indexing_configuration_fixture();
        config["limits"][key] = value;
        invalids.push(config);
    }
    let mut config = indexing_configuration_fixture();
    config["limits"]["retry"] = serde_json::json!({"max_attempts":21,"base_seconds":5,"cap_seconds":3600,"jitter_percent":25});
    invalids.push(config);
    let mut config = indexing_configuration_fixture();
    config["scope"]["item_kinds"] = serde_json::json!(["movie"]);
    invalids.push(config);
    let mut config = indexing_configuration_fixture();
    config["scope"]["item_ids"] = serde_json::json!(["one"]);
    invalids.push(config);
    for config in invalids {
        let input = OperatorIndexingConfigureRequest {
            request_id: ID.into(),
            existing: None,
            configuration: serde_json::from_value(config).unwrap(),
        };
        assert!(matches!(
            client.indexing_configure(ID.into(), input).await,
            Err(OperatorError::Invalid)
        ));
    }
    assert!(server.requests.lock().unwrap().is_empty());
    let mut private = indexing_configuration_fixture();
    private["operator_profile_id"] = serde_json::json!("forbidden");
    assert!(serde_json::from_value::<OperatorIndexingConfiguration>(private).is_err());
    let mut repair = indexing_configuration_fixture();
    repair["mode"] = serde_json::json!("repair");
    repair["components"] = serde_json::json!({"metadata":false,"artwork":true,"embeddings":false});
    for strategy in ["missing", "changed", "stale", "failed", "all"] {
        repair["strategy"] = serde_json::json!(strategy);
        repair["artwork"] = serde_json::json!({"posters":{"skip_existing":strategy=="missing"}});
        repair["stale_after_seconds"] = if strategy == "stale" {
            serde_json::json!(60)
        } else {
            serde_json::Value::Null
        };
        let input = OperatorIndexingConfigureRequest {
            request_id: ID.into(),
            existing: None,
            configuration: serde_json::from_value(repair.clone()).unwrap(),
        };
        // The synthetic malformed success proves admission dispatched, without claiming server acceptance.
        assert!(matches!(
            client.indexing_configure(ID.into(), input).await,
            Err(OperatorError::InvalidResponse { ambiguous: true })
        ));
    }
    assert_eq!(server.requests.lock().unwrap().len(), 5);
}

#[tokio::test]
async fn indexing_jobs_and_numeric_history_are_sanitized_and_lossless() {
    let job = serde_json::json!({"id":ID,"run_id":ID,"kind":"ingest","state":"failed","available_at":"2026-11-12T00:00:00Z","lease_until":null,"attempts":5,"max_attempts":5,"generation":0,"redrive_allowed":true,"redrive_reason":null,"input":{"credential":"must_not_escape"},"lease_owner":"must_not_escape"});
    let server = Server::new(
        200,
        serde_json::json!({"items":[job],"next_after":ID}).to_string(),
        0,
        false,
    );
    let jobs = server
        .client(1000)
        .indexing_jobs(
            ID.into(),
            ID.into(),
            OperatorPage {
                after: None,
                limit: 1,
            },
        )
        .await
        .unwrap();
    assert!(jobs.items[0].redrive_allowed);
    assert!(
        !serde_json::to_string(&jobs)
            .unwrap()
            .contains("must_not_escape")
    );
    let attempt = serde_json::json!({"job_id":ID,"fence":9007199254740993_i64,"generation":2,"started_at":"2026-11-12T00:00:00Z","finished_at":null,"outcome":"running","error_code":null,"owner":"must_not_escape"});
    let server = Server::new(
        200,
        serde_json::json!({"items":[attempt],"next_after":"9007199254740993"}).to_string(),
        0,
        false,
    );
    let history = server
        .client(1000)
        .indexing_attempts(
            ID.into(),
            ID.into(),
            ID.into(),
            OperatorPage {
                after: Some("9007199254740992".into()),
                limit: 1,
            },
        )
        .await
        .unwrap();
    assert_eq!(history.items[0].fence, 9007199254740993);
    assert!(!serde_json::to_string(&history).unwrap().contains("owner"));
    {
        let call = &server.requests.lock().unwrap()[0];
        assert_eq!(call.rpc, "ListAttempts");
        assert_eq!(call.request.source_id, ID);
        assert_eq!(call.request.run_id, ID);
        assert_eq!(call.request.job_id, ID);
        let page = call.request.page.as_ref().unwrap();
        assert_eq!(page.limit, 1);
        assert_eq!(page.after.as_deref(), Some("9007199254740992"));
    }
    let server=Server::new(200,serde_json::json!({"items":[{"request_id":ID,"job_id":ID,"generation":2}],"next_after":"2"}).to_string(),0,false);
    assert_eq!(
        server
            .client(1000)
            .indexing_redrives(
                ID.into(),
                ID.into(),
                ID.into(),
                OperatorPage {
                    after: Some("1".into()),
                    limit: 1
                }
            )
            .await
            .unwrap()
            .items[0]
            .generation,
        2
    );
    let client = server.client(1000);
    for after in [
        "-1",
        "01",
        "9223372036854775808",
        "1&limit=1",
        "1/../../",
        "",
    ] {
        assert!(matches!(
            client
                .indexing_attempts(
                    ID.into(),
                    ID.into(),
                    ID.into(),
                    OperatorPage {
                        after: Some(after.into()),
                        limit: 1
                    }
                )
                .await,
            Err(OperatorError::Invalid)
        ));
    }
    assert!(matches!(
        client
            .indexing_redrives(
                ID.into(),
                ID.into(),
                ID.into(),
                OperatorPage {
                    after: Some("2147483648".into()),
                    limit: 1
                }
            )
            .await,
        Err(OperatorError::Invalid)
    ));
    assert!(matches!(
        client
            .indexing_jobs(
                ID.into(),
                ID.into(),
                OperatorPage {
                    after: None,
                    limit: 201
                }
            )
            .await,
        Err(OperatorError::Invalid)
    ));
    assert_eq!(server.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn indexing_diagnostics_preserve_unknown_capability_context_and_observation_intervals() {
    for capabilities in [None, Some(vec![]), Some(vec!["jellyfin:locator/v1".into()])] {
        let server = Server::new(
            200,
            diagnostics_fixture(capabilities.clone()).to_string(),
            0,
            false,
        );
        let client = server.client(1000);
        let input = OperatorDiagnosticsRequest {
            run_id: Some(ID.into()),
            capabilities: capabilities.clone(),
        };
        let result = client.indexing_queue(input.clone()).await.unwrap();
        assert_eq!(result.queue.ready, 9007199254740993);
        assert_eq!(result.queue.claimable.is_some(), capabilities.is_some());
        assert!(!result.atomic);
        assert_ne!(result.queue_observation.finished_at, result.blockers_as_of);
        assert!(
            serde_json::to_value(&result)
                .unwrap()
                .get("as_of")
                .is_none()
        );
        {
            let call = &server.requests.lock().unwrap()[0];
            assert_eq!(call.rpc, "ObserveQueue");
        }
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(server.requests.lock().unwrap()[0].body())
                .unwrap(),
            serde_json::to_value(input).unwrap()
        );
    }
    let mut invalid = diagnostics_fixture(None);
    invalid["queue"]["claimable"] = serde_json::json!(0);
    let server = Server::new(200, invalid.to_string(), 0, false);
    assert!(matches!(
        server
            .client(1000)
            .indexing_queue(OperatorDiagnosticsRequest {
                run_id: None,
                capabilities: None
            })
            .await,
        Err(OperatorError::InvalidResponse { ambiguous: false })
    ));
    let client = server.client(1000);
    assert!(matches!(
        client
            .indexing_queue(OperatorDiagnosticsRequest {
                run_id: None,
                capabilities: Some(vec!["secret?url".into()])
            })
            .await,
        Err(OperatorError::Invalid)
    ));
    assert_eq!(server.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn indexing_redrive_and_start_never_retry_ambiguous_mutations() {
    let server = Server::new(
        200,
        serde_json::json!({"request_id":ID,"job_id":ID,"generation":3}).to_string(),
        0,
        false,
    );
    assert_eq!(
        server
            .client(1000)
            .indexing_redrive(
                ID.into(),
                ID.into(),
                ID.into(),
                OperatorRedriveRequest {
                    request_id: ID.into()
                }
            )
            .await
            .unwrap()
            .generation,
        3
    );
    {
        let call = &server.requests.lock().unwrap()[0];
        assert_eq!(call.rpc, "RedriveJob");
        assert_eq!(call.request.source_id, ID);
        assert_eq!(call.request.run_id, ID);
        assert_eq!(call.request.job_id, ID);
    }
    for operation in ["start", "configure", "redrive"] {
        let server = Server::new(200, "{}".into(), 0, true);
        let client = server.client(1000);
        let error = match operation {
            "start" => client.indexing_start(ID.into(), start()).await.unwrap_err(),
            "configure" => client
                .indexing_configure(
                    ID.into(),
                    OperatorIndexingConfigureRequest {
                        request_id: ID.into(),
                        existing: None,
                        configuration: serde_json::from_value(indexing_configuration_fixture())
                            .unwrap(),
                    },
                )
                .await
                .unwrap_err(),
            _ => client
                .indexing_redrive(
                    ID.into(),
                    ID.into(),
                    ID.into(),
                    OperatorRedriveRequest {
                        request_id: ID.into(),
                    },
                )
                .await
                .unwrap_err(),
        };
        assert!(matches!(
            error,
            OperatorError::Unavailable { ambiguous: true }
        ));
        assert_eq!(server.requests.lock().unwrap().len(), 1);
    }
    let server = Server::new(200, "{}".into(), 100, false);
    assert!(matches!(
        server
            .client(10)
            .indexing_redrive(
                ID.into(),
                ID.into(),
                ID.into(),
                OperatorRedriveRequest {
                    request_id: ID.into()
                }
            )
            .await,
        Err(OperatorError::Deadline { ambiguous: true })
    ));
}

#[tokio::test]
async fn indexing_start_status_and_controls_preserve_canonical_outcomes() {
    let mut status: serde_json::Value = serde_json::from_str(&run_status_body()).unwrap();
    status["observed_records"] = serde_json::json!(9007199254740993_i64);
    status["terminal_reason"] = serde_json::json!("deadline_expired");
    status["coverage"]["unresolved_attempts"] = serde_json::json!(2);
    status["enumeration_complete"] = serde_json::json!(true);
    status["work_settled"] = serde_json::json!(true);
    status["work_succeeded"] = serde_json::json!(false);
    status["run"]["policy_version"] = serde_json::json!(9007199254740993_i64);
    let server = Server::new(200, status["run"].to_string(), 0, false);
    let mut input = start();
    input.policy_version = 9007199254740993;
    assert_eq!(
        server
            .client(1000)
            .indexing_start(ID.into(), input)
            .await
            .unwrap()
            .policy_version,
        9007199254740993
    );
    assert!(
        server.requests.lock().unwrap()[0]
            .body()
            .contains("\"policy_version\":9007199254740993")
    );
    let server = Server::new(200, status.to_string(), 0, false);
    let client = server.client(1000);
    let result = client
        .indexing_run_status(ID.into(), ID.into())
        .await
        .unwrap();
    assert_eq!(result.observed_records, Some(9007199254740993));
    assert_eq!(result.coverage.unresolved_attempts, 2);
    assert_eq!(result.terminal_reason.as_deref(), Some("deadline_expired"));
    assert!(result.enumeration_complete && result.work_settled && !result.work_succeeded);
    client.indexing_pause(ID.into(), ID.into()).await.unwrap();
    client.indexing_resume(ID.into(), ID.into()).await.unwrap();
    client.indexing_cancel(ID.into(), ID.into()).await.unwrap();
    let requests = server.requests.lock().unwrap();
    let actions = &requests[1..];
    assert_eq!(actions.len(), 3);
    for (call, rpc) in actions.iter().zip(["PauseRun", "ResumeRun", "CancelRun"]) {
        assert_eq!(call.rpc, rpc);
        assert_eq!(call.request.source_id, ID);
        assert_eq!(call.request.run_id, ID);
        assert_eq!(call.body(), "{}");
    }
}
fn start() -> OperatorStartRequest {
    OperatorStartRequest {
        request_id: ID.into(),
        policy_id: ID.into(),
        policy_version: 9_007_199_254_740_993,
        scope: Some(selected()),
    }
}

fn run_status_body() -> String {
    serde_json::json!({
        "run": {"id":ID,"source_id":ID,"policy_id":ID,"policy_version":9_007_199_254_740_993_i64,"state":"running","items":0,"requests":0,"started_at":"2026-11-11T00:00:00Z","finished_at":null},
        "coverage":{"unresolved_attempts":0,"primary_unique":0,"followup_ready":0,"followup_delayed":0,"followup_leased":0,"followup_failed":0,"unknown_attempts":0,"retryable_attempts":0,"permanent_attempts":0,"omissions":0,"unresolved_operations":0,"pending_artwork_publications":0},
        "queue":{"ready":0,"delayed":0,"leased":0,"failed":0},
        "candidates":{"selected":0,"checked":0,"skipped_ready":0,"staged":0,"unavailable":0,"failed":0},
        "allowed_actions":["pause","cancel"],"all_complete":false,"enumeration_complete":false,"work_settled":false,"work_succeeded":false,"observed_records":null,"terminal_reason":null,"as_of":"2026-11-11T00:00:00Z"
    }).to_string()
}

#[tokio::test]
async fn canonical_history_and_safe_candidates() {
    let run: serde_json::Value = serde_json::from_str(&run_status_body()).unwrap();
    let server = Server::new(
        200,
        serde_json::json!({"items":[run["run"]],"next_after":null}).to_string(),
        0,
        false,
    );
    let runs = server
        .client(1000)
        .indexing_runs(ID.into(), page())
        .await
        .unwrap();
    assert_eq!(runs.items[0].policy_version, 9007199254740993);
    {
        let call = &server.requests.lock().unwrap()[0];
        assert_eq!(call.rpc, "ListRuns");
        assert_eq!(call.request.source_id, ID);
    }
    let server = Server::new(200, serde_json::json!({"items":[{
        "source_item_id":ID,"public_id":"item:1","kind":"movie","state":"selected","code":"ready","selected_at":"2026-11-11T00:00:00Z","updated_at":"2026-11-11T00:00:00Z",
        "external_id":"not-public","source_artwork_id":"not-public","job_payload":{"secret":"not-public"}
    }],"next_after":null}).to_string(), 0, false);
    let candidates = server
        .client(1000)
        .indexing_candidates(ID.into(), ID.into(), page(), true)
        .await
        .unwrap();
    assert!(
        !serde_json::to_string(&candidates)
            .unwrap()
            .contains("not-public")
    );
    {
        let call = &server.requests.lock().unwrap()[0];
        assert_eq!(call.rpc, "ListCandidates");
        assert_eq!(call.request.source_id, ID);
        assert_eq!(call.request.run_id, ID);
        let page = call.request.page.as_ref().unwrap();
        assert_eq!(page.limit, 2);
        assert_eq!(page.pending_only, Some(true));
    }
}

#[test]
fn endpoint_and_input_boundary() {
    for endpoint in [
        "https://localhost/api",
        "http://example.com/api",
        "http://127.0.0.1/api/",
        "http://localhost/__operator/api",
        "http://localhost/api?x",
        "http://user@localhost/api",
        "http://localhost/x/../api",
        "http://localhost:0/api",
    ] {
        assert!(
            matches!(
                OperatorClient::new(OperatorClientConfig {
                    endpoint: endpoint.into(),
                    timeout_ms: 100
                }),
                Err(OperatorError::Invalid)
            ),
            "{endpoint}"
        );
    }
}

#[tokio::test]
async fn fresh_gets_pagination_and_encoded_library_cursor() {
    let server = Server::new(200, r#"{"items":[],"next_after":null}"#.into(), 0, false);
    let client = server.client(1000);
    client.sources(page()).await.unwrap();
    client.sources(page()).await.unwrap();
    client
        .libraries(
            ID.into(),
            OperatorPage {
                after: Some("a&limit=999/x?#".into()),
                limit: 2,
            },
        )
        .await
        .unwrap();
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    {
        let call = &requests[0];
        assert_eq!(call.rpc, "ListSources");
        let page = call.request.page.as_ref().unwrap();
        assert_eq!(page.limit, 2);
    }
    assert_eq!(
        requests[2].request.page.as_ref().unwrap().after.as_deref(),
        Some("a&limit=999/x?#")
    );
}

#[tokio::test]
async fn exact_start_body_and_one_ambiguous_attempt() {
    let server = Server::new(200, String::new(), 0, true);
    let client = server.client(500);
    assert!(matches!(
        client.indexing_start(ID.into(), start()).await,
        Err(OperatorError::Unavailable { ambiguous: true })
    ));
    tokio::time::sleep(Duration::from_millis(30)).await;
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.rpc, "StartRun");
    assert_eq!(request.request.source_id, ID);
    let body: serde_json::Value = serde_json::from_str(request.body()).unwrap();
    assert_eq!(body, serde_json::to_value(start()).unwrap());
    assert_eq!(body["policy_version"].as_i64(), Some(9_007_199_254_740_993));
}

#[tokio::test]
async fn typed_errors_and_malformed_responses() {
    for (status, code) in [
        (400, "invalid"),
        (404, "not_found"),
        (409, "stale"),
        (409, "source_busy"),
        (503, "unavailable"),
        (403, "invalid"),
    ] {
        let server = Server::new(
            status,
            format!(r#"{{"error":{{"code":"{code}","run_id":"{ID}"}}}}"#),
            0,
            false,
        );
        let error = server
            .client(1000)
            .indexing_start(ID.into(), start())
            .await
            .unwrap_err();
        match code {
            _ if status == 403 => assert!(matches!(error, OperatorError::Forbidden)),
            "invalid" => assert!(matches!(error, OperatorError::Invalid)),
            "not_found" => assert!(matches!(error, OperatorError::NotFound)),
            "stale" => assert!(matches!(error, OperatorError::Stale)),
            "source_busy" => {
                assert!(matches!(error, OperatorError::SourceBusy { run_id } if run_id == ID))
            }
            _ => assert!(matches!(
                error,
                OperatorError::Unavailable { ambiguous: true }
            )),
        }
        assert_eq!(server.requests.lock().unwrap().len(), 1);
    }
    for body in [
        "{bad",
        r#"{"ready":"yes"}"#,
        r#"{"error":{"code":"source_busy","run_id":"bad"}}"#,
    ] {
        let server = Server::new(
            if body.contains("error") { 409 } else { 200 },
            body.into(),
            0,
            false,
        );
        assert!(matches!(
            server.client(1000).health().await,
            Err(OperatorError::InvalidResponse { ambiguous: false })
        ));
    }
    let server = Server::new(200, "{}".into(), 0, false);
    assert!(matches!(
        server.client(1000).indexing_start(ID.into(), start()).await,
        Err(OperatorError::InvalidResponse { ambiguous: true })
    ));
}

#[tokio::test]
async fn response_and_input_bounds() {
    let server = Server::new(200, "x".repeat(2 * 1024 * 1024 + 1), 0, false);
    let client = server.client(1000);
    assert!(matches!(
        client.health().await,
        Err(OperatorError::InvalidResponse { ambiguous: false })
    ));
    assert!(matches!(
        client
            .sources(OperatorPage {
                limit: 1001,
                after: None
            })
            .await,
        Err(OperatorError::Invalid)
    ));
    assert!(matches!(
        client.indexing_run_status("../bad".into(), ID.into()).await,
        Err(OperatorError::Invalid)
    ));
    let mut invalid = selected();
    invalid.library_ids = vec!["x".repeat(201)];
    assert!(matches!(
        client
            .indexing_start(
                ID.into(),
                OperatorStartRequest {
                    scope: Some(invalid),
                    ..start()
                }
            )
            .await,
        Err(OperatorError::Invalid)
    ));
    assert_eq!(server.requests.lock().unwrap().len(), 1);
    let mut large = selected();
    large.item_ids = (0..1000)
        .map(|n| format!("{n:04}{}", "x".repeat(196)))
        .collect();
    assert!(matches!(
        client
            .indexing_start(
                ID.into(),
                OperatorStartRequest {
                    scope: Some(large),
                    ..start()
                }
            )
            .await,
        Err(OperatorError::Invalid)
    ));
    assert_eq!(server.requests.lock().unwrap().len(), 1);
    let server = Server::new(
        200,
        format!(r#"{{"items":[],"next_after":"{ID}"}}"#),
        0,
        false,
    );
    assert!(matches!(
        server
            .client(1000)
            .sources(OperatorPage {
                after: Some(ID.into()),
                limit: 1
            })
            .await,
        Err(OperatorError::InvalidResponse { ambiguous: false })
    ));
}

#[tokio::test]
async fn discovery_read_post_does_not_claim_ambiguous_mutation() {
    let server = Server::new(200, discovery_fixture().to_string(), 0, false);
    let client = server.client(1000);
    let discovery = OperatorDiscoverConnectionRequest {
        base_url: "http://provider.test/jellyfin".into(),
        api_key: "synthetic-secret".into(),
    };
    assert_eq!(
        client
            .discover_connection(discovery)
            .await
            .unwrap()
            .server
            .name,
        "Synthetic"
    );
    {
        let requests = server.requests.lock().unwrap();
        {
            let call = &requests[0];
            assert_eq!(call.rpc, "DiscoverConnection");
        }
        let bytes = requests[0].body();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(bytes).unwrap(),
            serde_json::json!({"base_url":"http://provider.test/jellyfin","api_key":"synthetic-secret"})
        );
    }
    let server = Server::new(503, r#"{"error":{"code":"unavailable"}}"#.into(), 0, false);
    assert!(matches!(
        server
            .client(1000)
            .discover_connection(OperatorDiscoverConnectionRequest {
                base_url: "http://provider.test".into(),
                api_key: "synthetic-secret".into()
            })
            .await,
        Err(OperatorError::Unavailable { ambiguous: false })
    ));
}

#[tokio::test]
async fn deadlines_and_cancel_all_cover_mutations_and_new_generation() {
    let server = Server::new(200, r#"{"ready":true}"#.into(), 120, false);
    assert!(matches!(
        server.client(20).health().await,
        Err(OperatorError::Deadline { ambiguous: false })
    ));
    assert!(matches!(
        server.client(20).indexing_start(ID.into(), start()).await,
        Err(OperatorError::Deadline { ambiguous: true })
    ));
    let server = Server::new(200, r#"{"ready":true}"#.into(), 80, false);
    let client = server.client(2000);
    let a = client.clone();
    let read = tokio::spawn(async move { a.health().await });
    let a = client.clone();
    let mutation = tokio::spawn(async move { a.indexing_start(ID.into(), start()).await });
    tokio::time::sleep(Duration::from_millis(15)).await;
    client.cancel_all();
    assert!(matches!(
        read.await.unwrap(),
        Err(OperatorError::Cancelled { ambiguous: false })
    ));
    assert!(matches!(
        mutation.await.unwrap(),
        Err(OperatorError::Cancelled { ambiguous: true })
    ));
    assert!(client.health().await.unwrap().ready);
}
