use screenroom_backend::{
    db::MIGRATOR,
    sqlx::{self, postgres::PgPoolOptions},
};
use screenroom_index_operator::Operator;
use screenroom_proto::operator::v1::{
    OperatorPage, OperatorRequest, OperatorResponse, operator_service_client::OperatorServiceClient,
};
use serde_json::{Value, json};
use uuid::Uuid;

use tonic::{
    Code, Response, Status,
    transport::{Channel, Endpoint},
};

async fn start_operator(
    operator: Operator,
) -> (
    OperatorServiceClient<Channel>,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<Result<(), tonic::transport::Error>>,
) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint =
        Endpoint::from_shared(format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let (stop, shutdown) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(screenroom_index_operator::serve(
        operator,
        listener,
        async {
            let _ = shutdown.await;
        },
        vec![],
    ));
    (
        OperatorServiceClient::new(endpoint.connect().await.unwrap()),
        stop,
        server,
    )
}

fn body(json: &Value) -> OperatorRequest {
    OperatorRequest {
        payload_json: serde_json::to_vec(json).unwrap(),
        ..Default::default()
    }
}

fn page(limit: u32) -> OperatorPage {
    OperatorPage {
        limit,
        ..Default::default()
    }
}

fn payload(result: Result<Response<OperatorResponse>, Status>) -> Value {
    serde_json::from_slice(&result.unwrap().into_inner().payload_json).unwrap()
}

#[tokio::test]
#[ignore = "requires task-owned SCREENROOM_OPERATOR_TEST_DATABASE_URL"]
async fn postgres_managed_connection_rpc_contract() {
    use screenroom_backend::connections::{ConnectionRepository, CredentialDirectory};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let admin =
        sqlx::PgPool::connect(&std::env::var("SCREENROOM_OPERATOR_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
    let name = format!("operator_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect_with(admin.connect_options().as_ref().clone().database(&name))
        .await
        .unwrap();
    MIGRATOR.run(&pool).await.unwrap();
    let runtime = PgPoolOptions::new()
        .max_connections(5)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE screenroom_operator")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap();
    let directory = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("operator-keys-{}", Uuid::new_v4()));
    let operator = Operator::new_rpc(runtime.clone()).with_connections(ConnectionRepository::new(
        runtime.clone(),
        Some(CredentialDirectory::new(directory.clone()).unwrap()),
    ));
    let (mut rpc, stop_rpc, rpc_server) = start_operator(operator).await;
    assert!(rpc.health(OperatorRequest::default()).await.is_ok());
    let list = OperatorRequest {
        page: Some(page(100)),
        ..Default::default()
    };
    assert!(
        payload(rpc.list_connections(list.clone()).await)["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut bytes = Vec::new();
                loop {
                    let mut chunk = [0; 4096];
                    let n = socket.read(&mut chunk).await.unwrap();
                    if n == 0 {
                        return;
                    }
                    bytes.extend_from_slice(&chunk[..n]);
                    if bytes.windows(4).any(|s| s == b"\r\n\r\n") {
                        break;
                    }
                }
                let headers = String::from_utf8(bytes).unwrap();
                let path = headers
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap();
                assert!(headers.contains("operator-owned-key"));
                let body = match path {
                    "/System/Info" => {
                        json!({"Id":"operator-server","ServerName":"Owned HTTP server","Version":"10.11.0"})
                    }
                    "/Library/VirtualFolders" => {
                        json!([{"ItemId":"movies","Name":"Movies","CollectionType":"movies"}])
                    }
                    "/Users" => {
                        json!([{"Id":"selected","Name":"Selected","Policy":{"IsDisabled":false,"IsHidden":false}}])
                    }
                    _ => panic!("unexpected path"),
                };
                let body = body.to_string();
                let _=socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await;
            });
        }
    });
    let prospective = json!({"base_url":url,"api_key":"operator-owned-key"});
    let discovery = payload(rpc.discover_connection(body(&prospective)).await);
    assert_eq!(discovery["server"]["id"], "operator-server");
    assert_eq!(discovery["profiles"][0]["id"], "selected");
    assert!(!directory.exists());
    let local = Uuid::new_v4();
    sqlx::query("INSERT INTO identity.users(id,public_id,display_name) VALUES($1,$2,'Local user')")
        .bind(local)
        .bind(local.to_string())
        .execute(&pool)
        .await
        .unwrap();
    let save = json!({"request_id":Uuid::new_v4(),"base_url":url,"api_key":"operator-owned-key","server_id":"operator-server","profile_id":"selected","library_ids":["movies"],"local_user_id":local,"refresh":false});
    let mut oversized = save.clone();
    oversized["library_ids"] = json!((0..33).map(|i| format!("library-{i}")).collect::<Vec<_>>());
    let rejected = rpc.save_connection(body(&oversized)).await.unwrap_err();
    assert_eq!(rejected.code(), Code::InvalidArgument);
    assert_eq!(rejected.message(), "selection_capacity");
    assert!(!directory.exists());
    let saved = payload(rpc.save_connection(body(&save)).await);
    assert!(saved["run_id"].is_null());
    assert_eq!(payload(rpc.save_connection(body(&save)).await), saved);
    let listed = payload(rpc.list_connections(list).await);
    assert_eq!(listed["items"].as_array().unwrap().len(), 1);
    for dto in [&saved, &listed] {
        let text = dto.to_string();
        for secret in [
            "operator-owned-key",
            "selected",
            url.as_str(),
            "credential_ref",
            "locator_revision",
            "indexing_profile_id",
        ] {
            assert!(!text.contains(secret));
        }
    }
    let source = saved["connection"]["source_id"].as_str().unwrap();
    let policies = payload(
        rpc.list_policies(OperatorRequest {
            source_id: source.into(),
            page: Some(page(100)),
            ..Default::default()
        })
        .await,
    );
    assert_eq!(policies["items"].as_array().unwrap().len(), 1);
    assert!(
        policies["items"][0]["configuration"]
            .get("operator_profile_id")
            .is_none()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM queue.jobs")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    drop(rpc);
    stop_rpc.send(()).unwrap();
    rpc_server.await.unwrap().unwrap();
    runtime.close().await;
    pool.close().await;
    server.abort();
    assert!(Uuid::parse_str(name.strip_prefix("operator_").unwrap()).is_ok());
    sqlx::query(&format!("DROP DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
    for entry in std::fs::read_dir(&directory).unwrap() {
        let path = entry.unwrap().path();
        assert!(Uuid::parse_str(path.file_stem().unwrap().to_str().unwrap()).is_ok());
        std::fs::remove_file(path).unwrap();
    }
    std::fs::remove_dir(directory).unwrap();
}
#[tokio::test]
#[ignore = "requires task-owned disposable PostgreSQL 18 + pgvector SCREENROOM_OPERATOR_TEST_DATABASE_URL"]
async fn postgres_indexing_rpc_contract() {
    let admin = PgPoolOptions::new()
        .max_connections(4)
        .connect(&std::env::var("SCREENROOM_OPERATOR_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let name = format!("operator_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect_with(admin.connect_options().as_ref().clone().database(&name))
        .await
        .unwrap();
    MIGRATOR.run(&pool).await.unwrap();
    let source:Uuid=sqlx::query_scalar("INSERT INTO catalog.sources(source_key,provider) VALUES('operator-fixture','synthetic') RETURNING id").fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO catalog.libraries(source_id,public_id,external_id,title) VALUES($1,'library','private-library','Library')").bind(source).execute(&pool).await.unwrap();
    let item: Uuid = sqlx::query_scalar(
        "INSERT INTO catalog.items(public_id,kind) VALUES('item','movie') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let si:Uuid=sqlx::query_scalar("INSERT INTO catalog.source_items(source_id,item_id,external_id,metadata,content_hash) VALUES($1,$2,'private-provider-id','{}',decode(repeat('00',32),'hex')) RETURNING id").bind(source).bind(item).fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO catalog.library_items(library_id,source_item_id,source_id,last_seen_run_id) SELECT id,$2,$1,$3 FROM catalog.libraries WHERE source_id=$1").bind(source).bind(si).bind(Uuid::new_v4()).execute(&pool).await.unwrap();
    let (mut rpc, stop_rpc, rpc_server) = start_operator(Operator::new_rpc(pool.clone())).await;
    let legacy_policy:Uuid=sqlx::query_scalar("INSERT INTO ingest.policies(source_id,selection_json,max_items,max_requests,incremental_seconds,reconcile_seconds,enrich_metadata,enrich_people,enrich_artwork) VALUES($1,'{\"profile_id\":\"secret-profile\"}',100,100,60,60,false,false,true) RETURNING id").bind(source).fetch_one(&pool).await.unwrap();
    let legacy:screenroom_backend::ingest::Run=sqlx::query_as("INSERT INTO ingest.runs(policy_id,policy_version,policy_snapshot,selection_snapshot,source_id) SELECT id,version,to_jsonb(p),selection_json,source_id FROM ingest.policies p WHERE id=$1 RETURNING *")
        .bind(legacy_policy).fetch_one(&pool).await.unwrap();
    let legacy_request = OperatorRequest {
        source_id: source.to_string(),
        run_id: legacy.id.to_string(),
        ..Default::default()
    };
    let legacy_error = rpc
        .get_run_status(legacy_request.clone())
        .await
        .unwrap_err();
    assert_eq!(legacy_error.code(), Code::NotFound);
    assert!(!legacy_error.message().contains("secret-profile"));
    let mut cancel = legacy_request;
    cancel.payload_json = b"{}".to_vec();
    assert_eq!(
        rpc.cancel_run(cancel).await.unwrap_err().code(),
        Code::NotFound
    );
    assert!(
        payload(
            rpc.list_runs(OperatorRequest {
                source_id: source.to_string(),
                page: Some(page(100)),
                ..Default::default()
            })
            .await
        )["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let canonical_source:Uuid=sqlx::query_scalar("INSERT INTO catalog.sources(source_key,provider,configuration) VALUES('canonical-http','jellyfin','{\"adapter\":\"jellyfin\"}') RETURNING id").fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO catalog.libraries(source_id,public_id,external_id,title) VALUES($1,'a','private-a','A')").bind(canonical_source).execute(&pool).await.unwrap();
    screenroom_backend::indexing::IndexingRepository::new(pool.clone())
        .enroll_locator(canonical_source, &"0".repeat(64))
        .await
        .unwrap();
    let connection = Uuid::new_v4();
    sqlx::query("INSERT INTO connections.jellyfin(id,source_id,server_id,server_name,server_version,base_url,credential_ref,revision,indexing_profile_id) VALUES($1,$2,'operator-test','Owned HTTP test','10.11.0','http://127.0.0.1:1',$3,$4,'private-profile-sentinel')").bind(connection).bind(canonical_source).bind(Uuid::new_v4()).bind("0".repeat(64)).execute(&pool).await.unwrap();
    sqlx::query("UPDATE catalog.sources SET configuration=configuration||$2 WHERE id=$1")
        .bind(canonical_source)
        .bind(json!({"connection_id":connection,"connection_generation":1}))
        .execute(&pool)
        .await
        .unwrap();
    let configuration = json!({"indexing":1,"enabled":true,"cadence_seconds":3600,"mode":"full","components":{"metadata":true,"artwork":false,"embeddings":false},"strategy":"all","scope":{"library_ids":["a"],"item_kinds":["movie","series"]},"limits":{"work_requests":10,"work_records":8,"lifetime_requests":100,"lifetime_records":100,"deadline_seconds":3600,"work_timeout_seconds":30}});
    let configure =
        json!({"request_id":Uuid::new_v4(),"existing":null,"configuration":configuration});
    let scoped = || OperatorRequest {
        source_id: canonical_source.to_string(),
        ..Default::default()
    };
    let write = |json: &Value| OperatorRequest {
        payload_json: body(json).payload_json,
        ..scoped()
    };
    let policy = payload(rpc.configure_policy(write(&configure)).await);
    assert_eq!(policy["automatic_runs"], false);
    assert!(!policy.to_string().contains("private-profile-sentinel"));
    assert!(policy.get("selection_json").is_none());
    let mut private_input = configure.clone();
    private_input["configuration"]["operator_profile_id"] = json!("private-profile-sentinel");
    let rejected = rpc
        .configure_policy(write(&private_input))
        .await
        .unwrap_err();
    assert_eq!(rejected.code(), Code::InvalidArgument);
    assert!(!rejected.message().contains("private-profile-sentinel"));
    let listed = payload(
        rpc.list_policies(OperatorRequest {
            page: Some(page(100)),
            ..scoped()
        })
        .await,
    );
    assert!(!listed.to_string().contains("private-profile-sentinel"));
    assert!(!listed.to_string().contains("selection_json"));
    assert_eq!(
        payload(rpc.configure_policy(write(&configure)).await),
        policy
    );
    let run = payload(rpc.start_run(write(&json!({"request_id":Uuid::new_v4(),"policy_id":policy["id"],"policy_version":policy["version"],"scope":null}))).await);
    let run_id = run["id"].as_str().unwrap();
    let for_run = || OperatorRequest {
        run_id: run_id.into(),
        ..scoped()
    };
    assert!(!run.to_string().contains("private-profile-sentinel"));
    assert!(run.get("policy_snapshot").is_none());
    assert!(run.get("trigger").is_none());
    let history = payload(
        rpc.list_runs(OperatorRequest {
            page: Some(page(1)),
            ..scoped()
        })
        .await,
    );
    assert_eq!(history["items"][0]["id"], run_id);
    assert!(history["items"][0].get("trigger").is_none());
    let detail = payload(rpc.get_run_status(for_run()).await);
    assert!(!detail.to_string().contains("private-profile-sentinel"));
    let jobs = payload(
        rpc.list_jobs(OperatorRequest {
            page: Some(page(100)),
            ..for_run()
        })
        .await,
    );
    assert_eq!(jobs["items"].as_array().unwrap().len(), 1);
    assert!(jobs["items"][0].get("input").is_none());
    let diagnostics = payload(
        rpc.observe_queue(body(&json!({"run_id":run_id,"capabilities":null})))
            .await,
    );
    assert!(diagnostics["queue"]["claimable"].is_null());
    assert!(diagnostics["blockers"]["capability"].is_null());
    assert_eq!(diagnostics["worker_liveness"], "unknown");
    let ingest = screenroom_backend::ingest::IngestRepository::new(pool.clone());
    let claim = ingest
        .claim_canonical(
            "operator-test",
            60,
            &[
                "adapter:jellyfin".into(),
                "source:canonical-http".into(),
                format!("locator:{}", "0".repeat(64)),
            ],
        )
        .await
        .unwrap()
        .unwrap();
    ingest
        .fail(
            &claim,
            screenroom_backend::queue::FailureClass::Permanent,
            "invalid_input",
            None,
        )
        .await
        .unwrap();
    let for_job = || OperatorRequest {
        job_id: claim.id.to_string(),
        ..for_run()
    };
    let redrive = OperatorRequest {
        payload_json: body(&json!({"request_id":Uuid::new_v4()})).payload_json,
        ..for_job()
    };
    let receipt = payload(rpc.redrive_job(redrive.clone()).await);
    assert_eq!(receipt["generation"], 1);
    assert_eq!(payload(rpc.redrive_job(redrive).await), receipt);
    let history = payload(
        rpc.list_attempts(OperatorRequest {
            page: Some(page(100)),
            ..for_job()
        })
        .await,
    );
    assert_eq!(history["items"][0]["outcome"], "permanent");
    assert!(history["items"][0].get("owner").is_none());
    let action = OperatorRequest {
        payload_json: b"{}".to_vec(),
        ..for_run()
    };
    rpc.pause_run(action.clone()).await.unwrap();
    rpc.resume_run(action.clone()).await.unwrap();
    rpc.cancel_run(action).await.unwrap();
    drop(rpc);
    stop_rpc.send(()).unwrap();
    rpc_server.await.unwrap().unwrap();
    pool.close().await;
    sqlx::query(&format!("DROP DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}
