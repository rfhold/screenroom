use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use screenroom_backend::{
    db::MIGRATOR,
    sqlx::{self, postgres::PgPoolOptions},
};
use screenroom_index_operator::{Operator, Origin};
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

const ORIGIN: &str = "http://127.0.0.1:50072";
fn app(pool: sqlx::PgPool) -> Router {
    Operator::new(pool, Origin::parse(ORIGIN).unwrap()).router()
}
fn lazy() -> sqlx::PgPool {
    PgPoolOptions::new()
        .connect_lazy("postgres://localhost/never_connect")
        .unwrap()
}
async fn call(app: &Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "127.0.0.1:50072")
        .header("origin", ORIGIN)
        .header("content-type", "application/json")
        .body(if method == "GET" {
            Body::empty()
        } else {
            Body::from(body.to_string())
        })
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
#[ignore = "requires task-owned SCREENROOM_OPERATOR_TEST_DATABASE_URL"]
async fn postgres_managed_connection_http_contract() {
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
    let app = Operator::new(runtime.clone(), Origin::parse(ORIGIN).unwrap())
        .with_connections(ConnectionRepository::new(
            runtime.clone(),
            Some(CredentialDirectory::new(directory.clone()).unwrap()),
        ))
        .router();
    assert_eq!(
        call(&app, "GET", "/api/health", json!({})).await.0,
        StatusCode::OK
    );
    let (status, empty) = call(&app, "GET", "/api/connections/jellyfin", json!({})).await;
    assert_eq!(status, StatusCode::OK);
    assert!(empty["items"].as_array().unwrap().is_empty());
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
    let (status, discovery) = call(
        &app,
        "POST",
        "/api/connections/jellyfin/discover",
        prospective.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
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
    assert_eq!(
        call(&app, "POST", "/api/connections/jellyfin", oversized).await,
        (
            StatusCode::BAD_REQUEST,
            json!({"error":{"code":"selection_capacity"}})
        )
    );
    assert!(!directory.exists());
    let (status, saved) = call(&app, "POST", "/api/connections/jellyfin", save.clone()).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert!(saved["run_id"].is_null());
    assert_eq!(
        call(&app, "POST", "/api/connections/jellyfin", save).await,
        (StatusCode::OK, saved.clone())
    );
    let (status, listed) = call(&app, "GET", "/api/connections/jellyfin", json!({})).await;
    assert_eq!(status, StatusCode::OK);
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
    let (_, policies) = call(
        &app,
        "GET",
        &format!("/api/indexing/sources/{source}/policies"),
        json!({}),
    )
    .await;
    assert_eq!(policies["items"].as_array().unwrap().len(), 1);
    assert!(
        policies["items"][0]["configuration"]
            .get("operator_profile_id")
            .is_none()
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("/api/sources/{source}/start"),
            json!({})
        )
        .await
        .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM queue.jobs")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
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
async fn deny_before_parsing_or_database() {
    let app = app(lazy());
    for (host, origin, method) in [
        (Some("evil.example"), Some(ORIGIN), "GET"),
        (None, Some(ORIGIN), "GET"),
        (Some("127.0.0.1:50072"), Some("null"), "GET"),
        (
            Some("127.0.0.1:50072"),
            Some("http://localhost:4173"),
            "GET",
        ),
        (Some("127.0.0.1:50072"), None, "POST"),
        (Some("127.0.0.1:50072"), Some("http://evil.example"), "POST"),
    ] {
        let mut req = Request::builder().method(method).uri("/api/health");
        if let Some(host) = host {
            req = req.header("host", host);
        }
        if let Some(origin) = origin {
            req = req.header("origin", origin);
        }
        assert_eq!(
            app.clone()
                .oneshot(req.body(Body::from("invalid-json")).unwrap())
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
    }
    for duplicate in ["host", "origin"] {
        let req = Request::builder()
            .uri("/api/health")
            .header("host", "127.0.0.1:50072")
            .header("origin", ORIGIN)
            .header(
                duplicate,
                if duplicate == "host" {
                    "127.0.0.1:50072"
                } else {
                    ORIGIN
                },
            )
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.clone().oneshot(req).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
    for site in ["cross-site", "same-site"] {
        let req = Request::builder()
            .uri("/api/health")
            .header("host", "127.0.0.1:50072")
            .header("sec-fetch-site", site)
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.clone().oneshot(req).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }
    for ct in [
        None,
        Some("text/plain"),
        Some("application/json; charset=utf-8"),
    ] {
        let mut req = Request::builder()
            .method("POST")
            .uri("/api/sources/bad/status")
            .header("host", "127.0.0.1:50072")
            .header("origin", ORIGIN);
        if let Some(ct) = ct {
            req = req.header("content-type", ct);
        }
        assert_eq!(
            app.clone()
                .oneshot(req.body(Body::from("{}")).unwrap())
                .await
                .unwrap()
                .status(),
            StatusCode::UNSUPPORTED_MEDIA_TYPE
        );
    }
    let req = Request::builder()
        .method("POST")
        .uri("/api/sources/bad/status")
        .header("host", "127.0.0.1:50072")
        .header("origin", ORIGIN)
        .header("content-type", "application/json")
        .body(Body::from(vec![b' '; 65537]))
        .unwrap();
    assert_eq!(
        app.clone().oneshot(req).await.unwrap().status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    for path in [
        "/assets/../secret.js",
        "/assets/%2e%2e/secret.js",
        "/assets/%252e%252e/secret.js",
        "/api/sources?limit=1001",
        "/api/sources?limit=0",
        "/api/sources?unknown=1",
    ] {
        assert_eq!(
            call(&app, "GET", path, Value::Null).await.0,
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        call(
            &app,
            "POST",
            "/api/indexing/sources/00000000-0000-0000-0000-000000000001/start",
            json!({"trigger":"scheduled"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
}
#[test]
fn exact_loopback_origin() {
    for origin in [
        "http://127.0.0.1:50072",
        "http://localhost:50072",
        "http://[::1]:50072",
    ] {
        assert!(Origin::parse(origin).is_ok(), "{origin}");
    }
    for origin in [
        "http://127.0.0.1:4173",
        "http://localhost:99999",
        "http://localhost:0",
        "http://evil.example",
        "https://localhost:50072",
        "http://localhost:50072/",
        "http://localhost:50072/path",
        "http://user@localhost:50072",
        "http://localhost:50072#x",
        "null",
    ] {
        assert!(Origin::parse(origin).is_err(), "{origin}");
    }
}
#[tokio::test]
async fn api_only_paths() {
    let app = app(lazy());
    for path in [
        "/",
        "/index.html",
        "/assets/index.js",
        "/assets/index.css",
        "/private.js",
        "/arbitrary",
    ] {
        for method in ["GET", "POST"] {
            let (status, body) = call(&app, method, path, json!({})).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{method} {path}");
            assert_eq!(body["error"]["code"], "not_found");
        }
        let request = Request::builder()
            .method("HEAD")
            .uri(path)
            .header("host", "127.0.0.1:50072")
            .header("origin", ORIGIN)
            .body(Body::empty())
            .unwrap();
        assert_eq!(
            app.clone().oneshot(request).await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
    }
}

#[tokio::test]
#[ignore = "requires task-owned disposable PostgreSQL 18 + pgvector SCREENROOM_OPERATOR_TEST_DATABASE_URL"]
async fn postgres_http_contract() {
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
    let app = app(pool.clone());
    let base = format!("/api/sources/{source}");
    for route in ["policies", "preview", "status", "start", "runs"] {
        for method in ["GET", "POST"] {
            assert_eq!(
                call(&app, method, &format!("{base}/{route}"), json!({}))
                    .await
                    .0,
                StatusCode::NOT_FOUND
            );
        }
    }
    let legacy_policy:Uuid=sqlx::query_scalar("INSERT INTO ingest.policies(source_id,selection_json,max_items,max_requests,incremental_seconds,reconcile_seconds,enrich_metadata,enrich_people,enrich_artwork) VALUES($1,'{\"profile_id\":\"secret-profile\"}',100,100,60,60,false,false,true) RETURNING id").bind(source).fetch_one(&pool).await.unwrap();
    let legacy:screenroom_backend::ingest::Run=sqlx::query_as("INSERT INTO ingest.runs(policy_id,policy_version,policy_snapshot,selection_snapshot,source_id) SELECT id,version,to_jsonb(p),selection_json,source_id FROM ingest.policies p WHERE id=$1 RETURNING *")
        .bind(legacy_policy).fetch_one(&pool).await.unwrap();
    let legacy_status = call(
        &app,
        "GET",
        &format!("/api/indexing/sources/{source}/runs/{}", legacy.id),
        Value::Null,
    )
    .await;
    assert_eq!(legacy_status.0, StatusCode::NOT_FOUND);
    assert!(!legacy_status.1.to_string().contains("secret-profile"));
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("{base}/runs/{}/cancel", legacy.id),
            json!({})
        )
        .await
        .1["error"]["code"],
        "not_found"
    );
    assert!(
        call(
            &app,
            "GET",
            &format!("/api/indexing/sources/{source}/runs"),
            Value::Null
        )
        .await
        .1["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let canonical_source:Uuid=sqlx::query_scalar("INSERT INTO catalog.sources(source_key,provider,configuration) VALUES('canonical-http','jellyfin','{\"adapter\":\"jellyfin\"}') RETURNING id").fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO catalog.libraries(source_id,public_id,external_id,title) VALUES($1,'a','private-a','A')").bind(canonical_source).execute(&pool).await.unwrap();
    let generic = format!("/api/indexing/sources/{canonical_source}");
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
    let (code, policy) = call(
        &app,
        "POST",
        &format!("{generic}/policies"),
        configure.clone(),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(policy["automatic_runs"], false);
    assert!(!policy.to_string().contains("private-profile-sentinel"));
    assert!(policy.get("selection_json").is_none());
    let mut private_input = configure.clone();
    private_input["configuration"]["operator_profile_id"] = json!("private-profile-sentinel");
    let (rejected, error) = call(&app, "POST", &format!("{generic}/policies"), private_input).await;
    assert_eq!(rejected, StatusCode::BAD_REQUEST);
    assert!(!error.to_string().contains("private-profile-sentinel"));
    let listed = call(&app, "GET", &format!("{generic}/policies"), Value::Null)
        .await
        .1;
    assert!(!listed.to_string().contains("private-profile-sentinel"));
    assert!(!listed.to_string().contains("selection_json"));
    assert_eq!(
        call(&app, "POST", &format!("{generic}/policies"), configure)
            .await
            .1,
        policy
    );
    let (code,run)=call(&app,"POST",&format!("{generic}/start"),json!({"request_id":Uuid::new_v4(),"policy_id":policy["id"],"policy_version":policy["version"],"scope":null})).await;
    assert_eq!(code, StatusCode::OK);
    let run_id = run["id"].as_str().unwrap();
    assert!(!run.to_string().contains("private-profile-sentinel"));
    assert!(run.get("policy_snapshot").is_none());
    assert!(run.get("trigger").is_none());
    let history = call(&app, "GET", &format!("{generic}/runs?limit=1"), Value::Null)
        .await
        .1;
    assert_eq!(history["items"][0]["id"], run_id);
    assert!(history["items"][0].get("trigger").is_none());
    let detail = call(
        &app,
        "GET",
        &format!("{generic}/runs/{run_id}"),
        Value::Null,
    )
    .await
    .1;
    assert!(!detail.to_string().contains("private-profile-sentinel"));
    let jobs = call(
        &app,
        "GET",
        &format!("{generic}/runs/{run_id}/jobs"),
        Value::Null,
    )
    .await
    .1;
    assert_eq!(jobs["items"].as_array().unwrap().len(), 1);
    assert!(jobs["items"][0].get("input").is_none());
    let diagnostics = call(
        &app,
        "POST",
        "/api/indexing/queue",
        json!({"run_id":run_id,"capabilities":null}),
    )
    .await
    .1;
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
    let redrive = json!({"request_id":Uuid::new_v4()});
    let route = format!("{generic}/runs/{run_id}/jobs/{}/redrive", claim.id);
    let (code, receipt) = call(&app, "POST", &route, redrive.clone()).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(receipt["generation"], 1);
    assert_eq!(call(&app, "POST", &route, redrive).await.1, receipt);
    let history = call(
        &app,
        "GET",
        &format!("{generic}/runs/{run_id}/jobs/{}/attempts", claim.id),
        Value::Null,
    )
    .await
    .1;
    assert_eq!(history["items"][0]["outcome"], "permanent");
    assert!(history["items"][0].get("owner").is_none());
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("{generic}/runs/{run_id}/pause"),
            json!({})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("{generic}/runs/{run_id}/resume"),
            json!({})
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        call(
            &app,
            "POST",
            &format!("{generic}/runs/{run_id}/cancel"),
            json!({})
        )
        .await
        .0,
        StatusCode::OK
    );
    drop(app);
    pool.close().await;
    sqlx::query(&format!("DROP DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}
