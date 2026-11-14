use screenroom_backend::{
    Uuid,
    connections::*,
    db::MIGRATOR,
    sqlx::{self, PgPool},
};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct Http {
    url: String,
    fault: Arc<AtomicUsize>,
    calls: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}
impl Http {
    async fn new() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let fault = Arc::new(AtomicUsize::new(0));
        let f = fault.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let f = f.clone();
                let c = c.clone();
                tokio::spawn(async move {
                    let mut input = Vec::new();
                    loop {
                        let mut chunk = [0; 4096];
                        let n = socket.read(&mut chunk).await.unwrap();
                        if n == 0 {
                            return;
                        }
                        input.extend_from_slice(&chunk[..n]);
                        if input.windows(4).any(|s| s == b"\r\n\r\n") {
                            break;
                        }
                    }
                    c.fetch_add(1, Ordering::SeqCst);
                    let request = String::from_utf8(input).unwrap();
                    let path = request
                        .lines()
                        .next()
                        .unwrap()
                        .split_whitespace()
                        .nth(1)
                        .unwrap();
                    let mut status = 200;
                    let body = if !request.contains("test-administrator-key")
                        || f.load(Ordering::SeqCst) == 1
                    {
                        status = 401;
                        json!({"private":"must-never-escape"})
                    } else if path == "/System/Info" {
                        json!({"Id":if f.load(Ordering::SeqCst)==2{"replacement"}else{"server"},"ServerName":if f.load(Ordering::SeqCst)==3{"echo-test-administrator-key"}else{"Owned server"},"Version":"10.11.0"})
                    } else if path == "/Library/VirtualFolders" {
                        let mut libraries = json!([{"ItemId":"movies","Name":"Movies","CollectionType":"movies"},{"ItemId":"shows","Name":"Shows","CollectionType":"tvshows"},{"ItemId":"collections","Name":"Collections","CollectionType":"boxsets"},{"ItemId":"music","Name":"Music","CollectionType":"music"}]);
                        if f.load(Ordering::SeqCst) == 4 {
                            libraries.as_array_mut().unwrap().push(json!({"ItemId":"collections2","Name":"More collections","CollectionType":"boxsets"}));
                        }
                        libraries
                    } else if path == "/Users" {
                        json!([{"Id":"admin","Name":"Administrator","Policy":{"IsDisabled":false,"IsHidden":false}},{"Id":"disabled","Name":"Disabled","Policy":{"IsDisabled":true,"IsHidden":false}}])
                    } else {
                        status = 404;
                        json!({})
                    };
                    let body = serde_json::to_vec(&body).unwrap();
                    let _=socket.write_all(format!("HTTP/1.1 {status} Result\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).as_bytes()).await;
                    let _ = socket.write_all(&body).await;
                });
            }
        });
        Self {
            url,
            fault,
            calls,
            task,
        }
    }
}
impl Drop for Http {
    fn drop(&mut self) {
        self.task.abort();
    }
}
struct Fixture {
    pool: PgPool,
    admin: PgPool,
    name: String,
    path: std::path::PathBuf,
}
impl Fixture {
    async fn new() -> Self {
        let admin = PgPool::connect(&std::env::var("SCREENROOM_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
        let name = format!("connections_{}", Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE DATABASE {name}"))
            .execute(&admin)
            .await
            .unwrap();
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .connect_with(admin.connect_options().as_ref().clone().database(&name))
            .await
            .unwrap();
        MIGRATOR.run(&pool).await.unwrap();
        let path = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("managed-keys-{}", Uuid::new_v4()));
        Self {
            pool,
            admin,
            name,
            path,
        }
    }
    fn repo(&self) -> ConnectionRepository {
        ConnectionRepository::new(
            self.pool.clone(),
            Some(CredentialDirectory::new(self.path.clone()).unwrap()),
        )
    }
    async fn cleanup(self) {
        self.pool.close().await;
        assert!(Uuid::parse_str(self.name.strip_prefix("connections_").unwrap()).is_ok());
        sqlx::query(&format!("DROP DATABASE {}", self.name))
            .execute(&self.admin)
            .await
            .unwrap();
        self.admin.close().await;
        if self.path.exists() {
            for entry in std::fs::read_dir(&self.path).unwrap() {
                let path = entry.unwrap().path();
                assert!(Uuid::parse_str(path.file_stem().unwrap().to_str().unwrap()).is_ok());
                std::fs::remove_file(path).unwrap();
            }
            std::fs::remove_dir(self.path).unwrap();
        }
    }
}
fn request(http: &Http, user: Uuid, refresh: bool) -> SaveRequest {
    SaveRequest {
        request_id: Uuid::new_v4(),
        base_url: http.url.clone(),
        api_key: "test-administrator-key".into(),
        server_id: "server".into(),
        profile_id: "admin".into(),
        library_ids: vec!["movies".into(), "shows".into(), "collections".into()],
        local_user_id: user,
        refresh,
    }
}

#[tokio::test]
#[ignore = "requires task-owned SCREENROOM_TEST_DATABASE_URL"]
async fn concurrent_distinct_requests_cannot_duplicate_remote_identity() {
    let f = Fixture::new().await;
    let http = Http::new().await;
    let repo = f.repo();
    let local = user(&f.pool).await;
    let a = request(&http, local, false);
    let b = request(&http, local, true);
    let (a, b) = tokio::join!(repo.save(&a), repo.save(&b));
    assert_ne!(a.is_ok(), b.is_ok());
    error_code(if a.is_err() { a } else { b }, "server_already_connected");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM connections.jellyfin")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM catalog.sources")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(std::fs::read_dir(&f.path).unwrap().count(), 1);
    f.cleanup().await;
}
fn error_code<T: std::fmt::Debug>(result: screenroom_backend::Result<T>, code: &str) {
    assert!(
        matches!(result,Err(screenroom_backend::Error::Connection(c)) if c==code),
        "expected {code}, got {result:?}"
    );
}
async fn user(pool: &PgPool) -> Uuid {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO identity.users(id,public_id,display_name) VALUES($1,$2,'Real local user')",
    )
    .bind(id)
    .bind(id.to_string())
    .execute(pool)
    .await
    .unwrap();
    id
}

#[tokio::test]
#[ignore = "requires task-owned SCREENROOM_TEST_DATABASE_URL"]
async fn prospective_discovery_atomic_save_refresh_and_private_grants() {
    let f = Fixture::new().await;
    let http = Http::new().await;
    let repo = f.repo();
    let local = user(&f.pool).await;
    let other = user(&f.pool).await;
    let before: String =
        sqlx::query_scalar("SELECT updated_at::text FROM identity.users WHERE id=$1")
            .bind(other)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert!(repo.list(None, 100).await.unwrap().is_empty());
    assert!(repo.capabilities().await.unwrap().is_empty());
    let discovered = repo
        .discover(&DiscoverRequest {
            base_url: http.url.clone(),
            api_key: "test-administrator-key".into(),
        })
        .await
        .unwrap();
    assert_eq!(http.calls.load(Ordering::SeqCst), 3);
    assert_eq!(discovered.profiles.len(), 1);
    assert_eq!(discovered.libraries.len(), 4);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM catalog.sources")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        0
    );
    assert!(!f.path.exists());
    let mut bad = request(&http, local, false);
    http.fault.store(3, Ordering::SeqCst);
    error_code(
        repo.discover(&DiscoverRequest {
            base_url: http.url.clone(),
            api_key: "test-administrator-key".into(),
        })
        .await,
        "invalid_discovery",
    );
    error_code(repo.save(&bad).await, "invalid_discovery");
    http.fault.store(4, Ordering::SeqCst);
    bad.library_ids = vec!["collections".into(), "collections2".into()];
    error_code(repo.save(&bad).await, "unsupported_library_scope");
    http.fault.store(0, Ordering::SeqCst);
    bad = request(&http, local, false);
    bad.api_key = "bad-key".into();
    error_code(repo.save(&bad).await, "credentials_rejected");
    bad = request(&http, local, false);
    bad.server_id = "different".into();
    error_code(repo.save(&bad).await, "server_identity_mismatch");
    bad = request(&http, local, false);
    bad.profile_id = "disabled".into();
    error_code(repo.save(&bad).await, "profile_unavailable");
    for selected in [
        vec![],
        vec!["movies".into(), "movies".into()],
        vec!["unknown".into()],
        vec!["music".into()],
    ] {
        bad = request(&http, local, false);
        bad.library_ids = selected;
        assert!(repo.save(&bad).await.is_err());
    }
    bad = request(&http, Uuid::new_v4(), false);
    error_code(repo.save(&bad).await, "local_user_unavailable");
    let saved_request = request(&http, local, false);
    let (one, two) = tokio::join!(repo.save(&saved_request), repo.save(&saved_request));
    let saved = one.unwrap();
    assert_eq!(two.unwrap().connection.id, saved.connection.id);
    assert!(saved.run_id.is_none());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM queue.jobs")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM ingest.policies")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM catalog.libraries")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        3
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT jellyfin_user_id FROM identity.jellyfin_user_links WHERE user_id=$1"
        )
        .bind(local)
        .fetch_one(&f.pool)
        .await
        .unwrap(),
        "admin"
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT updated_at::text FROM identity.users WHERE id=$1")
            .bind(other)
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        before
    );
    let calls = http.calls.load(Ordering::SeqCst);
    assert_eq!(
        repo.save(&saved_request).await.unwrap().connection.id,
        saved.connection.id
    );
    assert_eq!(http.calls.load(Ordering::SeqCst), calls);
    let mut conflict = saved_request;
    conflict.api_key = "other-key".into();
    error_code(repo.save(&conflict).await, "request_conflict");
    error_code(
        repo.save(&request(&http, local, true)).await,
        "server_already_connected",
    );
    let serialized = serde_json::to_string(&saved).unwrap();
    for private in [
        "test-administrator-key",
        "admin",
        http.url.as_str(),
        "credential_ref",
        "locator_revision",
        "indexing_profile_id",
    ] {
        assert!(!serialized.contains(private), "{private}");
    }
    let snapshot: Value =
        sqlx::query_scalar("SELECT configuration FROM catalog.sources WHERE id=$1")
            .bind(saved.connection.source_id)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert!(!snapshot.to_string().contains("test-administrator-key"));
    assert!(!snapshot.to_string().contains(&http.url));
    let resolved = repo.resolve(&saved.connection.source_key).await.unwrap();
    resolved.verify_identity().await.unwrap();
    assert_eq!(http.calls.load(Ordering::SeqCst), calls + 4);
    http.fault.store(2, Ordering::SeqCst);
    error_code(resolved.verify_identity().await, "server_identity_mismatch");
    http.fault.store(0, Ordering::SeqCst);
    for role in [
        "screenroom_catalog",
        "screenroom_user",
        "screenroom_worker",
        "screenroom_operator",
    ] {
        let mut connection = f.pool.acquire().await.unwrap();
        sqlx::query(&format!("SET ROLE {role}"))
            .execute(&mut *connection)
            .await
            .unwrap();
        let read = sqlx::query("SELECT credential_ref FROM connections.jellyfin")
            .execute(&mut *connection)
            .await;
        assert_eq!(read.is_ok(), role != "screenroom_catalog");
        assert_eq!(
            sqlx::query("SELECT indexing_profile_id FROM connections.jellyfin")
                .execute(&mut *connection)
                .await
                .is_ok(),
            role == "screenroom_operator"
        );
        sqlx::query("RESET ROLE")
            .execute(&mut *connection)
            .await
            .unwrap();
    }
    assert!(
        sqlx::query("UPDATE connections.jellyfin SET base_url='http://127.0.0.1:1'")
            .execute(&f.pool)
            .await
            .is_err()
    );
    sqlx::query("UPDATE catalog.sources SET configuration='{}' WHERE id=$1")
        .bind(saved.connection.source_id)
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(repo.resolve(&saved.connection.source_key).await.is_err());
    sqlx::query(
        "UPDATE connections.save_receipts SET created_at=clock_timestamp()-interval '25 hours'",
    )
    .execute(&f.pool)
    .await
    .unwrap();
    error_code(repo.save(&conflict).await, "request_expired");
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires task-owned SCREENROOM_TEST_DATABASE_URL"]
async fn explicit_refresh_and_failed_publication_are_transactional() {
    let f = Fixture::new().await;
    let http = Http::new().await;
    let local = user(&f.pool).await;
    let repo = f.repo();
    std::fs::write(&f.path, "not-a-directory").unwrap();
    error_code(
        repo.save(&request(&http, local, true)).await,
        "credential_store_unavailable",
    );
    std::fs::remove_file(&f.path).unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM catalog.sources")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        0
    );
    sqlx::raw_sql("CREATE FUNCTION connections.fail_commit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected commit failure'; END $$; CREATE CONSTRAINT TRIGGER fail_commit AFTER INSERT ON connections.save_receipts DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION connections.fail_commit();").execute(&f.pool).await.unwrap();
    error_code(
        repo.save(&request(&http, local, true)).await,
        "connection_unavailable",
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM catalog.sources")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM identity.jellyfin_user_links")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        std::fs::read_dir(&f.path).unwrap().count(),
        1,
        "uncertain-commit file must be retained"
    );
    sqlx::query("DROP TRIGGER fail_commit ON connections.save_receipts")
        .execute(&f.pool)
        .await
        .unwrap();
    let request = request(&http, local, true);
    let (a, b) = tokio::join!(repo.save(&request), repo.save(&request));
    let saved = a.unwrap();
    assert_eq!(b.unwrap().run_id, saved.run_id);
    assert!(saved.run_id.is_some());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM ingest.runs")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM queue.jobs")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        1
    );
    let pin: Option<Uuid> = sqlx::query_scalar("SELECT embedding_model_id FROM ingest.runs")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert!(pin.is_none());
    let payloads:String=sqlx::query_scalar("SELECT string_agg(v::text,'') FROM (SELECT configuration v FROM catalog.sources UNION ALL SELECT policy_snapshot FROM ingest.runs UNION ALL SELECT input FROM ingest.indexing_receipts UNION ALL SELECT result FROM connections.save_receipts) rows").fetch_one(&f.pool).await.unwrap();
    assert!(!payloads.contains("test-administrator-key"));
    assert!(!format!("{request:?}").contains("test-administrator-key"));
    assert!(
        !format!(
            "{:?}",
            DiscoverRequest {
                base_url: http.url.clone(),
                api_key: "test-administrator-key".into()
            }
        )
        .contains("test-administrator-key")
    );
    f.cleanup().await;
}
