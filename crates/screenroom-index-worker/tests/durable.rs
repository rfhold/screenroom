//! Explicit opt-in tests against an isolated, disposable PostgreSQL database.
use screenroom_backend::{
    catalog::CatalogRepository,
    db,
    embedding::{EmbeddingClient, EmbeddingConfig},
    ingest::{IngestRepository, PolicyWrite},
    sqlx,
    storage::{ObjectStorage, StorageConfig},
};
use screenroom_catalog_adapter::{Faults, MockAdapters};
use screenroom_index_worker::{Step, Worker};
use serde_json::json;
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

struct IoDouble {
    url: String,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    stall: Arc<AtomicBool>,
    entered: Arc<AtomicBool>,
}
impl IoDouble {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let stall = Arc::new(AtomicBool::new(false));
        let entered = Arc::new(AtomicBool::new(false));
        let stalled = stall.clone();
        let reached = entered.clone();
        let thread = std::thread::spawn(move || {
            let mut objects = HashMap::<String, (Vec<u8>, String)>::new();
            while !done.load(Ordering::Relaxed) {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(2));
                    continue;
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut bytes = vec![];
                let mut chunk = [0; 8192];
                let header_end = loop {
                    let n = stream.read(&mut chunk).unwrap();
                    if n == 0 {
                        break None;
                    }
                    bytes.extend_from_slice(&chunk[..n]);
                    if let Some(i) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        break Some(i + 4);
                    }
                };
                let Some(end) = header_end else { continue };
                let header = String::from_utf8_lossy(&bytes[..end]).to_string();
                let length = header
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|s| s.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                while bytes.len() < end + length {
                    let n = stream.read(&mut chunk).unwrap();
                    if n == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&chunk[..n]);
                }
                let mut parts = header.lines().next().unwrap().split_whitespace();
                let method = parts.next().unwrap();
                let path = parts.next().unwrap();
                if method == "PUT" && stalled.load(Ordering::Relaxed) {
                    reached.store(true, Ordering::Relaxed);
                    while stalled.load(Ordering::Relaxed) && !done.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                }
                let (status, body, length, mime) = if path == "/v1/embeddings" {
                    let mut vector = vec![0.0f32; 1024];
                    vector[0] = 1.0;
                    let body = serde_json::to_vec(
                        &json!({"model":"test","data":[{"index":0,"embedding":vector}]}),
                    )
                    .unwrap();
                    (200, body.clone(), body.len(), "application/json")
                } else if method == "PUT" {
                    let mime = header
                        .lines()
                        .find_map(|line| {
                            line.split_once(':')
                                .filter(|(name, _)| name.eq_ignore_ascii_case("content-type"))
                                .map(|(_, value)| value.trim().to_owned())
                        })
                        .unwrap();
                    objects.insert(path.into(), (bytes[end..end + length].to_vec(), mime));
                    (200, vec![], 0, "application/json")
                } else if let Some((object, mime)) = objects.get(path) {
                    (
                        200,
                        if method == "HEAD" {
                            vec![]
                        } else {
                            object.clone()
                        },
                        object.len(),
                        mime.as_str(),
                    )
                } else {
                    (404, vec![], 0, "application/json")
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status} OK\r\nContent-Length: {length}\r\nContent-Type: {mime}\r\nConnection: close\r\n\r\n"
                );
                let _ = stream.write_all(&body);
            }
        });
        Self {
            url,
            stop,
            thread: Some(thread),
            stall,
            entered,
        }
    }
}
impl Drop for IoDouble {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.thread.take().unwrap().join();
    }
}

async fn worker(pool: sqlx::PgPool, io: &IoDouble, faults: Faults) -> Worker {
    worker_revision(pool, io, faults, "test-r1").await
}

async fn worker_revision(
    pool: sqlx::PgPool,
    io: &IoDouble,
    faults: Faults,
    revision: &str,
) -> Worker {
    worker_from_root(
        pool,
        io,
        faults,
        revision,
        std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../fixtures/catalog"
        )),
    )
    .await
}

async fn worker_from_root(
    pool: sqlx::PgPool,
    io: &IoDouble,
    faults: Faults,
    revision: &str,
    root: &std::path::Path,
) -> Worker {
    Worker::new(
        pool,
        MockAdapters::load(root, faults).unwrap(),
        EmbeddingClient::new(EmbeddingConfig {
            base_url: format!("{}/v1", io.url),
            model: "test".into(),
            revision: revision.into(),
            dimensions: 1024,
            api_key: None,
            timeout: Duration::from_secs(5),
        })
        .unwrap(),
        ObjectStorage::new(StorageConfig {
            endpoint: io.url.clone(),
            region: "garage".into(),
            bucket: "test".into(),
            access_key_id: "test".into(),
            secret_access_key: "test-secret".into(),
            timeout: Duration::from_secs(5),
        })
        .unwrap(),
        "durable-test".into(),
    )
    .await
    .unwrap()
}

async fn drain(w: &Worker) {
    for _ in 0..300 {
        match w.step().await.unwrap() {
            Step::Idle => return,
            Step::Committed => {}
            step => panic!("unexpected step {step:?}"),
        }
    }
    panic!("drain did not converge");
}

async fn isolated_pool() -> sqlx::PgPool {
    let pool = unmigrated_pool().await;
    db::migrate(&pool).await.unwrap();
    pool
}

async fn unmigrated_pool() -> sqlx::PgPool {
    static SERVER_BOOTSTRAP: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
    let url = std::env::var("SCREENROOM_W_TEST_DATABASE_URL").unwrap();
    SERVER_BOOTSTRAP
        .get_or_init(|| async {
            let bootstrap = sqlx::PgPool::connect(&url).await.unwrap();
            db::migrate(&bootstrap).await.unwrap();
            bootstrap.close().await;
        })
        .await;
    let admin = sqlx::PgPool::connect(&url).await.unwrap();
    let name = format!("w_{}", screenroom_backend::Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
    let options = url
        .parse::<sqlx::postgres::PgConnectOptions>()
        .unwrap()
        .database(&name);
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires SCREENROOM_W_TEST_DATABASE_URL on a task-owned disposable PostgreSQL server"]
async fn service_startup_migrates_fresh_database_and_fails_closed() {
    use sqlx::ConnectOptions;
    let pool = unmigrated_pool().await;
    let migrations: Option<String> =
        sqlx::query_scalar("SELECT to_regclass('_sqlx_migrations')::text")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        migrations.is_none(),
        "service startup target must be unmigrated"
    );
    let io = IoDouble::new();
    let command = || {
        let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_screenroom-index-worker"));
        command
            .arg("drain")
            .env(
                "DATABASE_URL",
                pool.connect_options().to_url_lossy().as_str(),
            )
            .env_remove("SCREENROOM_MIGRATION_DATABASE_URL")
            .env(
                "SCREENROOM_FIXTURE_ROOT",
                concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/catalog"),
            )
            .env("SCREENROOM_EMBEDDING_BASE_URL", format!("{}/v1", io.url))
            .env("SCREENROOM_EMBEDDING_MODEL", "test")
            .env("SCREENROOM_EMBEDDING_REVISION", "test-r1")
            .env("SCREENROOM_EMBEDDING_DIMENSIONS", "1024")
            .env_remove("SCREENROOM_EMBEDDING_API_KEY")
            .env("SCREENROOM_S3_ENDPOINT", &io.url)
            .env("SCREENROOM_S3_REGION", "garage")
            .env("SCREENROOM_S3_BUCKET", "test")
            .env("SCREENROOM_S3_ACCESS_KEY_ID", "test")
            .env("SCREENROOM_S3_SECRET_ACCESS_KEY", "test-secret")
            .env_remove("OTEL_EXPORTER_OTLP_ENDPOINT")
            .env_remove("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT");
        command
    };
    let first = command().output().unwrap();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let migrations: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE success")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(migrations >= 5);
    let models: i64 = sqlx::query_scalar("SELECT count(*) FROM search.embedding_models")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        models, 1,
        "worker initializes only after automatic migration"
    );
    let repeat = command().output().unwrap();
    assert!(repeat.status.success());
    let blocked = command()
        .env(
            "SCREENROOM_MIGRATION_DATABASE_URL",
            "invalid-migration-secret",
        )
        .output()
        .unwrap();
    assert_eq!(
        blocked.status.code(),
        Some(1),
        "runtime database is reachable but cannot bypass migration failure"
    );
    assert!(!String::from_utf8_lossy(&blocked.stderr).contains("invalid-migration-secret"));
    sqlx::query("UPDATE _sqlx_migrations SET checksum=$1 WHERE version=(SELECT min(version) FROM _sqlx_migrations)")
        .bind(vec![0u8;48]).execute(&pool).await.unwrap();
    let checksum_failure = command().output().unwrap();
    assert_eq!(
        checksum_failure.status.code(),
        Some(1),
        "checksum mismatch cannot start a worker"
    );
    pool.close().await;
}

struct FixtureCopy(std::path::PathBuf);
impl FixtureCopy {
    fn new() -> Self {
        fn copy(source: &std::path::Path, destination: &std::path::Path) {
            std::fs::create_dir_all(destination).unwrap();
            for entry in std::fs::read_dir(source).unwrap() {
                let entry = entry.unwrap();
                let target = destination.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    copy(&entry.path(), &target);
                } else {
                    std::fs::copy(entry.path(), target).unwrap();
                }
            }
        }
        let root = std::env::temp_dir().join(format!(
            "screenroom-w-art-{}",
            screenroom_backend::Uuid::new_v4()
        ));
        copy(
            std::path::Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../fixtures/catalog"
            )),
            &root,
        );
        Self(root)
    }
}
impl Drop for FixtureCopy {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::test]
#[ignore = "requires SCREENROOM_W_TEST_DATABASE_URL on a task-owned disposable PostgreSQL server"]
async fn physically_missing_artwork_variants_are_omitted_and_repaired_by_new_ingestion() {
    use screenroom_backend::{catalog::Scope, media::MediaRepository};
    use screenroom_catalog_adapter::Error as AdapterError;

    let pool = isolated_pool().await;
    let io = IoDouble::new();
    let catalog = CatalogRepository::new(pool.clone());
    let ingest = IngestRepository::new(pool.clone());
    let media = MediaRepository::new(pool.clone());
    for native in [false, true] {
        let fixture = FixtureCopy::new();
        let adapters = MockAdapters::load(&fixture.0, Faults::default()).unwrap();
        let art = adapters
            .artwork()
            .iter()
            .find(|art| art.id == "tmdb")
            .unwrap();
        let file = if native {
            art.render_file.as_ref().unwrap()
        } else {
            &art.file
        };
        let path = fixture.0.join("artwork").join(file);
        let bytes = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        let missing = MockAdapters::load(&fixture.0, Faults::default()).unwrap();
        assert!(matches!(
            missing.read_artwork("tmdb", native),
            Err(AdapterError::MissingArtwork)
        ));
        assert!(missing.read_artwork("tmdb", !native).is_ok());

        let source_key = if native {
            "physical-native"
        } else {
            "physical-original"
        };
        let source = catalog
            .configure_source(source_key, "tmdb", &json!({"adapter":"mock"}), &json!({}))
            .await
            .unwrap();
        let policy = ingest.create_policy(source.id, &PolicyWrite {
            enabled:true, selection_json:json!({"feeds":[{"feed_key":"one-film","feed":"MovieUpcoming","max_pages":1,"page_size":1,"locale":"en-US","region":null,"trending_window":null}]}),
            max_items:10, max_requests:10, incremental_seconds:3600, reconcile_seconds:3600,
            enrich_metadata:false, enrich_people:false, enrich_artwork:true,
        }).await.unwrap();
        let w = worker_from_root(pool.clone(), &io, Faults::default(), "test-r1", &fixture.0).await;
        let run = ingest.start_run(policy.id).await.unwrap();
        drain(&w).await;
        assert_eq!(ingest.run(run.id).await.unwrap().unwrap().state, "complete");
        let metadata_and_vectors:i64 = sqlx::query_scalar("SELECT count(*) FROM catalog.source_items si JOIN search.documents d ON d.source_item_id=si.id JOIN search.embeddings e ON e.document_id=d.id AND e.input_hash=d.content_hash WHERE si.source_id=$1")
            .bind(source.id).fetch_one(&pool).await.unwrap();
        assert_eq!(
            metadata_and_vectors, 1,
            "missing optional bytes must not stop metadata or embedding jobs"
        );
        let omissions = ingest.artwork_omissions(run.id, None, 10).await.unwrap();
        assert_eq!(omissions.len(), 1);
        assert_eq!(omissions[0].source_artwork_id, "tmdb");
        assert_eq!(serde_json::to_value(omissions[0].code).unwrap(), "missing");
        let generations:i64 = sqlx::query_scalar("SELECT count(*) FROM media.artwork_generations g JOIN media.artwork a ON a.id=g.artwork_id WHERE a.source_id=$1 AND a.source_artwork_id='tmdb'")
            .bind(source.id).fetch_one(&pool).await.unwrap();
        assert_eq!(
            generations, 0,
            "missing required original or native cannot fabricate a generation"
        );
        let variants:i64 = sqlx::query_scalar("SELECT count(*) FROM media.variants v JOIN media.artwork a ON a.id=v.artwork_id WHERE a.source_id=$1 AND a.source_artwork_id='tmdb'")
            .bind(source.id).fetch_one(&pool).await.unwrap();
        assert_eq!(variants, 0);

        std::fs::write(&path, &bytes).unwrap();
        let repaired =
            worker_from_root(pool.clone(), &io, Faults::default(), "test-r1", &fixture.0).await;
        let repair = ingest.start_run(policy.id).await.unwrap();
        drain(&repaired).await;
        assert_eq!(
            ingest.run(repair.id).await.unwrap().unwrap().state,
            "complete"
        );
        assert!(
            ingest
                .artwork_omissions(repair.id, None, 10)
                .await
                .unwrap()
                .is_empty()
        );
        let ready:(screenroom_backend::Uuid, Option<i64>, i64) = sqlx::query_as("SELECT id,revision,current_generation FROM media.artwork WHERE source_id=$1 AND source_artwork_id='tmdb'")
            .bind(source.id).fetch_one(&pool).await.unwrap();
        assert_eq!((ready.1, ready.2), (Some(1), 1));
        let restored = MockAdapters::load(&fixture.0, Faults::default()).unwrap();
        for variant in ["original", "native"] {
            let object = media
                .resolve(&Scope::default(), ready.0, 1, variant)
                .await
                .unwrap()
                .unwrap();
            let expected = restored.read_artwork("tmdb", variant == "native").unwrap();
            use sha2::{Digest, Sha256};
            assert_eq!(object.location.sha256, Sha256::digest(&expected).to_vec());
        }
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires SCREENROOM_W_TEST_DATABASE_URL on a task-owned disposable PostgreSQL server"]
async fn actual_artwork_inputs_a_b_a_publish_complete_generations_and_keep_old_objects() {
    use screenroom_backend::{catalog::Scope, media::MediaRepository};
    use screenroom_catalog_adapter::{Feed, FeedPolicy};
    let pool = isolated_pool().await;
    let fixture = FixtureCopy::new();
    let io = IoDouble::new();
    let adapters = MockAdapters::load(&fixture.0, Faults::default()).unwrap();
    let policy = FeedPolicy {
        feed: Feed::MovieUpcoming,
        max_pages: 1,
        page_size: 1,
        locale: "en-US".into(),
        region: None,
        trending_window: None,
    };
    let page = adapters.tmdb_page(&policy, None).unwrap();
    let key = &page.items[0].item.artwork_keys[0];
    let art = adapters.artwork().iter().find(|a| &a.id == key).unwrap();
    assert!(!art.file.ends_with(".svg") && art.render_file.is_none());
    let image_path = fixture.0.join("artwork").join(&art.file);
    let a = std::fs::read(&image_path).unwrap();
    let other = adapters
        .artwork()
        .iter()
        .find(|other| other.id != art.id && other.file.ends_with(".png"))
        .unwrap();
    let b = std::fs::read(fixture.0.join("artwork").join(&other.file)).unwrap();
    assert_ne!(a, b);
    let catalog = CatalogRepository::new(pool.clone());
    let ingest = IngestRepository::new(pool.clone());
    let media = MediaRepository::new(pool.clone());
    let source = catalog
        .configure_source(
            "revision-tmdb",
            "tmdb",
            &json!({"adapter":"mock"}),
            &json!({}),
        )
        .await
        .unwrap();
    let p=ingest.create_policy(source.id,&PolicyWrite {
        enabled:true,selection_json:json!({"feeds":[{"feed_key":"one-film","feed":"MovieUpcoming","max_pages":1,"page_size":1,"locale":"en-US","region":null,"trending_window":null}]}),
        max_items:10,max_requests:10,incremental_seconds:3600,reconcile_seconds:3600,enrich_metadata:false,enrich_people:false,enrich_artwork:true,
    }).await.unwrap();
    let w = worker_from_root(pool.clone(), &io, Faults::default(), "test-r1", &fixture.0).await;
    ingest.start_run(p.id).await.unwrap();
    drain(&w).await;
    let id: screenroom_backend::Uuid = sqlx::query_scalar(
        "SELECT id FROM media.artwork WHERE source_id=$1 AND source_artwork_id=$2",
    )
    .bind(source.id)
    .bind(key)
    .fetch_one(&pool)
    .await
    .unwrap();
    let first = media
        .resolve(&Scope::default(), id, 1, "native")
        .await
        .unwrap()
        .unwrap();
    for (bytes, revision) in [(&b, 2i64), (&a, 3i64)] {
        std::fs::write(&image_path, bytes).unwrap();
        let w = worker_from_root(pool.clone(), &io, Faults::default(), "test-r1", &fixture.0).await;
        ingest.start_run(p.id).await.unwrap();
        assert_eq!(w.step().await.unwrap(), Step::Committed);
        let pending: (Option<i64>, i64) =
            sqlx::query_as("SELECT revision,current_generation FROM media.artwork WHERE id=$1")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(pending, (Some(revision - 1), revision));
        assert!(
            media
                .resolve(&Scope::default(), id, revision, "native")
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(w.step().await.unwrap(), Step::Committed);
        let still_old: Option<i64> =
            sqlx::query_scalar("SELECT revision FROM media.artwork WHERE id=$1")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            still_old,
            Some(revision - 1),
            "one variant must not advance the pointer"
        );
        drain(&w).await;
        let ready = media
            .resolve(&Scope::default(), id, revision, "native")
            .await
            .unwrap()
            .unwrap();
        assert!(!ready.location.object_key.is_empty());
        let old = media
            .resolve(&Scope::default(), id, 1, "native")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(old.location.object_key, first.location.object_key);
    }
    let fingerprints: Vec<Vec<u8>> = sqlx::query_scalar(
        "SELECT fingerprint FROM media.artwork_generations WHERE artwork_id=$1 ORDER BY revision",
    )
    .bind(id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(fingerprints.len(), 3);
    assert_eq!(fingerprints[0], fingerprints[2]);
    assert_ne!(fingerprints[0], fingerprints[1]);
    // An unchanged repeat reuses generation 3 instead of allocating a fourth revision.
    let unchanged =
        worker_from_root(pool.clone(), &io, Faults::default(), "test-r1", &fixture.0).await;
    ingest.start_run(p.id).await.unwrap();
    drain(&unchanged).await;
    let current: i64 =
        sqlx::query_scalar("SELECT current_generation FROM media.artwork WHERE id=$1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(current, 3);
    std::fs::write(&image_path, b"invalid-image-input").unwrap();
    let invalid =
        worker_from_root(pool.clone(), &io, Faults::default(), "test-r1", &fixture.0).await;
    let invalid_run = ingest.start_run(p.id).await.unwrap();
    drain(&invalid).await;
    assert_eq!(
        ingest.run(invalid_run.id).await.unwrap().unwrap().state,
        "complete"
    );
    let omissions = ingest
        .artwork_omissions(invalid_run.id, None, 10)
        .await
        .unwrap();
    assert_eq!(omissions.len(), 1);
    assert_eq!(omissions[0].source_artwork_id, *key);
    assert_eq!(serde_json::to_value(omissions[0].code).unwrap(), "invalid");
    let still_ready = media
        .resolve(&Scope::default(), id, 3, "native")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(still_ready.location.object_key, first.location.object_key);
    std::fs::write(&image_path, &b).unwrap();
    let repaired =
        worker_from_root(pool.clone(), &io, Faults::default(), "test-r1", &fixture.0).await;
    let repair = ingest.start_run(p.id).await.unwrap();
    drain(&repaired).await;
    assert!(
        ingest
            .artwork_omissions(repair.id, None, 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        media
            .resolve(&Scope::default(), id, 4, "native")
            .await
            .unwrap()
            .is_some()
    );
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires SCREENROOM_W_TEST_DATABASE_URL on a task-owned disposable PostgreSQL server"]
async fn standalone_tmdb_hierarchy_restart_and_child_corruption() {
    let pool = isolated_pool().await;
    let io = IoDouble::new();
    let catalog = CatalogRepository::new(pool.clone());
    let ingest = IngestRepository::new(pool.clone());
    let source = catalog
        .configure_source(
            "hierarchy-tmdb",
            "tmdb",
            &json!({"adapter":"mock"}),
            &json!({}),
        )
        .await
        .unwrap();
    let p=ingest.create_policy(source.id,&PolicyWrite {
        enabled:true,selection_json:json!({"feeds":[{"feed_key":"tv-root","feed":"TvPopular","max_pages":1,"page_size":1,"locale":"en-US","region":null,"trending_window":null}]}),
        max_items:1,max_requests:1,incremental_seconds:3600,reconcile_seconds:3600,
        enrich_metadata:false,enrich_people:false,enrich_artwork:false,
    }).await.unwrap();
    for (faults, expected, diagnostic) in [
        (Faults::default(), "complete", None),
        (
            Faults {
                partial_page: Some(0),
                ..Default::default()
            },
            "complete",
            None,
        ),
        (
            Faults {
                loop_cursor_page: Some(1),
                ..Default::default()
            },
            "failed",
            Some("cursor_loop"),
        ),
        (
            Faults {
                duplicate_page: Some(1),
                ..Default::default()
            },
            "failed",
            Some("repeated_page"),
        ),
    ] {
        let run = ingest.start_run(p.id).await.unwrap();
        let mut saw_child_cursor = false;
        for _ in 0..20 {
            let w = worker(pool.clone(), &io, faults.clone()).await;
            drain(&w).await;
            let current = ingest.run(run.id).await.unwrap().unwrap();
            let state = ingest
                .run_checkpoint(&current, "traversal-v1")
                .await
                .unwrap()
                .unwrap();
            saw_child_cursor |= state["children"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["cursor"].is_string());
            if current.state != "paused" {
                break;
            }
            assert!(current.budget_requests <= 1 && current.budget_items <= 1);
            w.resume(run.id).await.unwrap();
        }
        let done = ingest.run(run.id).await.unwrap().unwrap();
        assert_eq!(done.state, expected);
        if let Some(code) = diagnostic {
            assert_eq!(done.requests, 4);
            assert_eq!(
                ingest
                    .run_checkpoint(&done, "traversal-v1")
                    .await
                    .unwrap()
                    .unwrap()["error_code"],
                code
            );
            assert!(saw_child_cursor);
        } else if faults.partial_page.is_none() {
            assert!(
                saw_child_cursor,
                "episode continuation must survive worker replacement"
            );
            let kinds:Vec<String>=sqlx::query_scalar("SELECT i.kind FROM catalog.source_items si JOIN catalog.items i ON i.id=si.item_id WHERE si.source_id=$1 ORDER BY i.kind").bind(source.id).fetch_all(&pool).await.unwrap();
            assert_eq!(kinds, vec!["episode", "episode", "season", "series"]);
            let children:i64=sqlx::query_scalar("SELECT count(*) FROM catalog.source_items si JOIN catalog.items i ON i.id=si.item_id WHERE si.source_id=$1 AND i.parent_id IS NOT NULL").bind(source.id).fetch_one(&pool).await.unwrap();
            assert_eq!(children, 3);
            let memberships: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM ingest.feed_memberships WHERE source_id=$1",
            )
            .bind(source.id)
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(memberships, 1, "children are not feed roots");
        } else {
            assert_eq!(done.coverage["partial"], true);
        }
        let retained: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM catalog.source_items WHERE source_id=$1 AND state='enabled'",
        )
        .bind(source.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(retained, 4, "child scopes never authorize title deletion");
    }
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires SCREENROOM_W_TEST_DATABASE_URL on a task-owned disposable PostgreSQL server"]
async fn absent_membership_embedding_rebuild_uses_internal_reference() {
    use screenroom_backend::{
        catalog::Scope,
        ingest::{CheckpointWrite, Completion, Followup, JobKind, PageCommit},
        search::SearchRepository,
    };
    let pool = isolated_pool().await;
    let io = IoDouble::new();
    let catalog = CatalogRepository::new(pool.clone());
    let ingest = IngestRepository::new(pool.clone());
    let search = SearchRepository::new(pool.clone());
    let source = catalog
        .configure_source(
            "hidden-jellyfin",
            "jellyfin",
            &json!({"adapter":"mock"}),
            &json!({}),
        )
        .await
        .unwrap();
    let p = ingest
        .create_policy(
            source.id,
            &PolicyWrite {
                enabled: true,
                selection_json: json!({"library_ids":["movies"],"page_size":3}),
                max_items: 100,
                max_requests: 100,
                incremental_seconds: 3600,
                reconcile_seconds: 3600,
                enrich_metadata: false,
                enrich_people: false,
                enrich_artwork: false,
            },
        )
        .await
        .unwrap();
    let initial = worker(pool.clone(), &io, Faults::default()).await;
    ingest.start_run(p.id).await.unwrap();
    drain(&initial).await;
    initial.activate_model().await.unwrap();
    let faults = Faults {
        removed_item: Some("sintel".into()),
        ..Default::default()
    };
    let removed = worker(pool.clone(), &io, faults.clone()).await;
    ingest.start_run(p.id).await.unwrap();
    drain(&removed).await;
    assert!(
        catalog
            .resolve_reference(&Scope::default(), "hidden-jellyfin", "sintel")
            .await
            .unwrap()
            .is_none()
    );
    let doc = search
        .document_for_reference("hidden-jellyfin", "sintel")
        .await
        .unwrap()
        .unwrap();
    let replacement = worker_revision(pool.clone(), &io, faults, "test-r2").await;
    let model = search
        .register_model("test", "test-r2", 1024)
        .await
        .unwrap();
    assert!(replacement.activate_model().await.is_err());
    // Explicit rebuild jobs use the same durable followup contract as page-produced jobs.
    let run = ingest.start_run(p.id).await.unwrap();
    let claim = ingest
        .claim("rebuild-dispatch", 300)
        .await
        .unwrap()
        .unwrap();
    ingest
        .commit_page(
            &claim,
            &PageCommit {
                libraries: vec![],
                items: vec![],
                feeds: vec![],
                artwork_inputs: vec![],
                artwork_omissions: vec![],
                followups: vec![Followup {
                    kind: JobKind::Embedding,
                    dedupe_key: format!("rebuild:{}:{}", doc.id, model.id),
                    input: json!({"external_id":"sintel","model_id":model.id}),
                }],
                checkpoint: CheckpointWrite {
                    scope_key: "rebuild-dispatch".into(),
                    cursor: json!({}),
                    watermark: None,
                },
                requests: 0,
                completion: Completion::Continue,
            },
        )
        .await
        .unwrap();
    drain(&replacement).await;
    assert_eq!(ingest.run(run.id).await.unwrap().unwrap().state, "complete");
    let current:i64=sqlx::query_scalar("SELECT count(*) FROM search.embeddings WHERE document_id=$1 AND model_id=$2 AND input_hash=$3").bind(doc.id).bind(model.id).bind(&doc.content_hash).fetch_one(&pool).await.unwrap();
    assert_eq!(current, 1);
    replacement.activate_model().await.unwrap();
    assert!(
        catalog
            .resolve_reference(&Scope::default(), "hidden-jellyfin", "sintel")
            .await
            .unwrap()
            .is_none()
    );
    pool.close().await;
}

#[cfg(unix)]
#[tokio::test]
#[ignore = "requires SCREENROOM_W_TEST_DATABASE_URL on a task-owned disposable PostgreSQL server"]
async fn sigterm_during_upload_exits_without_publication_and_lease_is_reclaimable() {
    use sqlx::ConnectOptions;
    let pool = isolated_pool().await;
    let io = IoDouble::new();
    io.stall.store(true, Ordering::Relaxed);
    let catalog = CatalogRepository::new(pool.clone());
    let ingest = IngestRepository::new(pool.clone());
    let source = catalog
        .configure_source(
            "shutdown-tmdb",
            "tmdb",
            &json!({"adapter":"mock"}),
            &json!({}),
        )
        .await
        .unwrap();
    ingest.create_policy(source.id,&PolicyWrite {
        enabled:true,selection_json:json!({"feeds":[{"feed_key":"one-film","feed":"MoviePopular","max_pages":1,"page_size":1,"locale":"en-US","region":null,"trending_window":null}]}),
        max_items:10,max_requests:10,incremental_seconds:3600,reconcile_seconds:3600,enrich_metadata:false,enrich_people:false,enrich_artwork:true,
    }).await.unwrap();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_screenroom-index-worker"))
        .arg("serve")
        .env(
            "DATABASE_URL",
            pool.connect_options().to_url_lossy().as_str(),
        )
        .env(
            "SCREENROOM_FIXTURE_ROOT",
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../fixtures/catalog"),
        )
        .env("SCREENROOM_EMBEDDING_BASE_URL", format!("{}/v1", io.url))
        .env("SCREENROOM_EMBEDDING_MODEL", "test")
        .env("SCREENROOM_EMBEDDING_REVISION", "test-r1")
        .env("SCREENROOM_EMBEDDING_DIMENSIONS", "1024")
        .env_remove("SCREENROOM_EMBEDDING_API_KEY")
        .env("SCREENROOM_S3_ENDPOINT", &io.url)
        .env("SCREENROOM_S3_REGION", "garage")
        .env("SCREENROOM_S3_BUCKET", "test")
        .env("SCREENROOM_S3_ACCESS_KEY_ID", "test")
        .env("SCREENROOM_S3_SECRET_ACCESS_KEY", "test-secret")
        .env_remove("OTEL_EXPORTER_OTLP_ENDPOINT")
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !io.entered.load(Ordering::Relaxed) && std::time::Instant::now() < deadline {
        if let Some(status) = child.try_wait().unwrap() {
            panic!("worker exited before delayed upload: {status}");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    if !io.entered.load(Ordering::Relaxed) {
        let _ = child.kill();
        let _ = child.wait();
        panic!("worker never reached delayed upload");
    }
    let leased: (screenroom_backend::Uuid, i64) =
        sqlx::query_as("SELECT id,fence FROM ingest.jobs WHERE kind='artwork' AND state='leased'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        std::process::Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("SIGTERM did not exit within three seconds");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert!(status.success());
    let ready: i64 = sqlx::query_scalar("SELECT count(*) FROM media.variants WHERE source_id=$1")
        .bind(source.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(ready, 0, "unverified upload cannot publish on shutdown");
    sqlx::query(
        "UPDATE ingest.jobs SET lease_until=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(leased.0)
    .execute(&pool)
    .await
    .unwrap();
    let reclaimed = ingest
        .claim("replacement-after-term", 300)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reclaimed.id, leased.0);
    assert!(reclaimed.fence > leased.1);
    ingest
        .retry(&reclaimed, "shutdown_recovery", 0)
        .await
        .unwrap();
    io.stall.store(false, Ordering::Relaxed);
    let replacement = worker(pool.clone(), &io, Faults::default()).await;
    drain(&replacement).await;
    let ready: i64 = sqlx::query_scalar("SELECT count(*) FROM media.variants WHERE source_id=$1")
        .bind(source.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(ready > 0);
    pool.close().await;
}

#[tokio::test]
#[ignore = "requires SCREENROOM_W_TEST_DATABASE_URL on a task-owned disposable PostgreSQL server"]
async fn restart_budget_reconciliation_and_verified_followups() {
    let pool = isolated_pool().await;
    let catalog = CatalogRepository::new(pool.clone());
    let ingest = IngestRepository::new(pool.clone());
    let source = catalog
        .configure_source(
            "mock-jellyfin",
            "jellyfin",
            &json!({"adapter":"mock"}),
            &json!({}),
        )
        .await
        .unwrap();
    let p = ingest
        .create_policy(
            source.id,
            &PolicyWrite {
                enabled: true,
                selection_json: json!({"library_ids":["movies","tv"],"page_size":3}),
                max_items: 6,
                max_requests: 8,
                incremental_seconds: 3600,
                reconcile_seconds: 3600,
                enrich_metadata: true,
                enrich_people: true,
                enrich_artwork: true,
            },
        )
        .await
        .unwrap();
    let io = IoDouble::new();
    let w = worker(pool.clone(), &io, Faults::default()).await;
    let run = ingest.start_run(p.id).await.unwrap();
    drain(&w).await;
    let paused = ingest.run(run.id).await.unwrap().unwrap();
    assert_eq!(paused.state, "paused");
    assert_eq!(paused.items, 6);
    drop(w);
    let w = worker(pool.clone(), &io, Faults::default()).await;
    w.resume(run.id).await.unwrap();
    drain(&w).await;
    assert_eq!(ingest.run(run.id).await.unwrap().unwrap().state, "complete");
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM catalog.source_items")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 12);
    let vectors:i64=sqlx::query_scalar("SELECT count(*) FROM search.embeddings e JOIN search.documents d ON d.id=e.document_id WHERE e.input_hash=d.content_hash").fetch_one(&pool).await.unwrap();
    assert_eq!(vectors, 12);
    let objects: i64 = sqlx::query_scalar("SELECT count(*) FROM media.objects WHERE state='ready'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(objects > 0);
    w.activate_model().await.unwrap();
    // Repeat traversal must preserve source identities, vectors, and content-addressed objects.
    let repeat = ingest.start_run(p.id).await.unwrap();
    drain(&w).await;
    w.resume(repeat.id).await.unwrap();
    drain(&w).await;
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM catalog.source_items")
            .fetch_one(&pool)
            .await
            .unwrap(),
        count
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM media.objects")
            .fetch_one(&pool)
            .await
            .unwrap(),
        objects
    );
    // Partial coverage must not remove a previously present membership.
    let partial = worker(
        pool.clone(),
        &io,
        Faults {
            partial_page: Some(0),
            removed_item: Some("sintel".into()),
            ..Default::default()
        },
    )
    .await;
    let partial_run = ingest.start_run(p.id).await.unwrap();
    drain(&partial).await;
    assert_eq!(
        ingest.run(partial_run.id).await.unwrap().unwrap().state,
        "complete"
    );
    let present:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM catalog.library_items li JOIN catalog.source_items si ON si.id=li.source_item_id WHERE si.external_id='sintel' AND li.state='enabled')").fetch_one(&pool).await.unwrap();
    assert!(present);
    // Full selected traversal is the only absence authority.
    let removed = worker(
        pool.clone(),
        &io,
        Faults {
            removed_item: Some("sintel".into()),
            ..Default::default()
        },
    )
    .await;
    let removal = ingest.start_run(p.id).await.unwrap();
    drain(&removed).await;
    removed.resume(removal.id).await.unwrap();
    drain(&removed).await;
    let absent:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM catalog.library_items li JOIN catalog.source_items si ON si.id=li.source_item_id WHERE si.external_id='sintel' AND li.state='absent')").fetch_one(&pool).await.unwrap();
    assert!(absent);
    let tmdb = catalog
        .configure_source("mock-tmdb", "tmdb", &json!({"adapter":"mock"}), &json!({}))
        .await
        .unwrap();
    let feeds = [
        "MoviePopular",
        "MovieUpcoming",
        "MovieNowPlaying",
        "MovieTrending",
        "TvPopular",
        "TvOnTheAir",
        "TvTrending",
    ]
    .into_iter()
    .map(|feed| {
        json!({"feed_key":format!("{feed}:en-US:day:2"),"feed":feed,
            "max_pages":2,"page_size":1,"locale":"en-US","region":null,
            "trending_window":if feed.ends_with("Trending"){Some("Day")}else{None}})
    })
    .collect::<Vec<_>>();
    let tmdb_policy = ingest
        .create_policy(
            tmdb.id,
            &PolicyWrite {
                enabled: true,
                selection_json: json!({"feeds":feeds}),
                max_items: 2,
                max_requests: 2,
                incremental_seconds: 3600,
                reconcile_seconds: 3600,
                enrich_metadata: true,
                enrich_people: true,
                enrich_artwork: true,
            },
        )
        .await
        .unwrap();
    let tmdb_run = ingest.start_run(tmdb_policy.id).await.unwrap();
    for _ in 0..30 {
        // Restart between every tranche: completed feed evidence and next-feed position are durable.
        let restarted = worker(pool.clone(), &io, Faults::default()).await;
        drain(&restarted).await;
        let state = ingest.run(tmdb_run.id).await.unwrap().unwrap();
        if state.state == "complete" {
            break;
        }
        assert_eq!(state.state, "paused");
        restarted.resume(tmdb_run.id).await.unwrap();
    }
    assert_eq!(
        ingest.run(tmdb_run.id).await.unwrap().unwrap().state,
        "complete"
    );
    let feed_count: i64 = sqlx::query_scalar(
        "SELECT count(DISTINCT feed_key) FROM ingest.feed_memberships WHERE source_id=$1",
    )
    .bind(tmdb.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(feed_count, 7);
    let membership_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM ingest.feed_memberships WHERE source_id=$1")
            .bind(tmdb.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(membership_count, 11);
    let orders:Vec<(String,Vec<i32>)>=sqlx::query_as("SELECT feed_key,array_agg(ordinal ORDER BY ordinal) FROM ingest.feed_memberships WHERE source_id=$1 GROUP BY feed_key").bind(tmdb.id).fetch_all(&pool).await.unwrap();
    for (key, ordinals) in orders {
        assert_eq!(
            ordinals,
            if key.starts_with("Movie") {
                vec![0, 1]
            } else {
                vec![0]
            }
        );
    }
    let title_count: i64 = sqlx::query_scalar("SELECT count(*) FROM catalog.items")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(title_count, 12);
    let missing_source = catalog
        .configure_source(
            "missing-art-test",
            "tmdb",
            &json!({"adapter":"mock"}),
            &json!({}),
        )
        .await
        .unwrap();
    let missing_policy=ingest.create_policy(missing_source.id,&PolicyWrite {
        enabled:true,selection_json:json!({"feeds":[{"feed_key":"upcoming-one","feed":"MovieUpcoming","max_pages":1,"page_size":1,"locale":"en-US","region":null,"trending_window":null}]}),
        max_items:1,max_requests:1,incremental_seconds:3600,reconcile_seconds:3600,
        enrich_metadata:false,enrich_people:false,enrich_artwork:true,
    }).await.unwrap();
    let missing_run = ingest.start_run(missing_policy.id).await.unwrap();
    let missing_worker = worker(
        pool.clone(),
        &io,
        Faults {
            missing_artwork: Some("his-girl-friday".into()),
            ..Default::default()
        },
    )
    .await;
    let mut retries = 0;
    for _ in 0..20 {
        match missing_worker.step().await.unwrap() {
            Step::Idle => break,
            Step::Retried => retries += 1,
            Step::Committed => {}
            step => panic!("unexpected {step:?}"),
        }
    }
    assert_eq!(retries, 0);
    let fabricated:i64=sqlx::query_scalar("SELECT count(*) FROM media.variants v JOIN media.artwork a ON a.id=v.artwork_id WHERE a.source_id=$1 AND a.source_artwork_id='his-girl-friday'").bind(missing_source.id).fetch_one(&pool).await.unwrap();
    assert_eq!(
        fabricated, 0,
        "missing original/native bytes cannot become ready"
    );
    let omissions = ingest
        .artwork_omissions(missing_run.id, None, 10)
        .await
        .unwrap();
    assert_eq!(omissions.len(), 1);
    assert_eq!(omissions[0].source_artwork_id, "his-girl-friday");
    assert_eq!(serde_json::to_value(omissions[0].code).unwrap(), "missing");
    assert_eq!(
        ingest.run(missing_run.id).await.unwrap().unwrap().state,
        "complete"
    );
    let repair = ingest.start_run(missing_policy.id).await.unwrap();
    let repaired_worker = worker(pool.clone(), &io, Faults::default()).await;
    drain(&repaired_worker).await;
    assert!(
        ingest
            .artwork_omissions(repair.id, None, 10)
            .await
            .unwrap()
            .is_empty()
    );
    let repaired:i64=sqlx::query_scalar("SELECT count(*) FROM media.variants v JOIN media.artwork a ON a.id=v.artwork_id WHERE a.source_id=$1 AND a.source_artwork_id='his-girl-friday'").bind(missing_source.id).fetch_one(&pool).await.unwrap();
    assert_eq!(repaired, 2);
    // A dead process cannot publish with its old fence after lease reclamation.
    let lease_run = ingest.start_run(p.id).await.unwrap();
    let old = ingest.claim("dead-worker", 1).await.unwrap().unwrap();
    sqlx::query(
        "UPDATE ingest.jobs SET lease_until=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(old.id)
    .execute(&pool)
    .await
    .unwrap();
    let fresh = ingest
        .claim("replacement-worker", 300)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(old.id, fresh.id);
    assert!(fresh.fence > old.fence);
    assert!(matches!(
        ingest.retry(&old, "unavailable", 0).await,
        Err(screenroom_backend::Error::Stale)
    ));
    catalog.set_source_state(source.id, false).await.unwrap();
    assert!(matches!(
        ingest.retry(&fresh, "unavailable", 0).await,
        Err(screenroom_backend::Error::Stale)
    ));
    ingest.cancel_run(lease_run.id).await.unwrap();
    catalog.set_source_state(source.id, true).await.unwrap();
    let selected = PolicyWrite {
        enabled: true,
        selection_json: json!({"library_ids":["movies"],"page_size":3}),
        max_items: 100,
        max_requests: 100,
        incremental_seconds: 3600,
        reconcile_seconds: 3600,
        enrich_metadata: true,
        enrich_people: true,
        enrich_artwork: true,
    };
    ingest.update_policy(p.id, &selected, false).await.unwrap();
    let selected_worker = worker(
        pool.clone(),
        &io,
        Faults {
            removed_item: Some("beverly-hillbillies-s1-e1".into()),
            ..Default::default()
        },
    )
    .await;
    let selected_run = ingest.start_run(p.id).await.unwrap();
    drain(&selected_worker).await;
    assert_eq!(
        ingest.run(selected_run.id).await.unwrap().unwrap().state,
        "complete"
    );
    let tv_present:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM catalog.library_items li JOIN catalog.source_items si ON si.id=li.source_item_id WHERE si.external_id='beverly-hillbillies-s1-e1' AND li.state='enabled')").fetch_one(&pool).await.unwrap();
    assert!(
        tv_present,
        "an unselected library cannot supply absence evidence"
    );
    let loop_worker = worker(
        pool.clone(),
        &io,
        Faults {
            loop_cursor_page: Some(0),
            ..Default::default()
        },
    )
    .await;
    let looping = ingest.start_run(p.id).await.unwrap();
    drain(&loop_worker).await;
    let looped = ingest.run(looping.id).await.unwrap().unwrap();
    assert_eq!(looped.state, "failed");
    assert_eq!(looped.requests, 1);
    let loop_checkpoint = ingest
        .run_checkpoint(&looped, "traversal-v1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(loop_checkpoint["error_code"], "cursor_loop");
    let diagnostic: String =
        sqlx::query_scalar("SELECT error_code FROM ingest.jobs WHERE run_id=$1 AND kind='ingest'")
            .bind(looping.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(diagnostic, "cursor_loop");
    let duplicate_worker = worker(
        pool.clone(),
        &io,
        Faults {
            duplicate_page: Some(1),
            ..Default::default()
        },
    )
    .await;
    let duplicate = ingest.start_run(p.id).await.unwrap();
    drain(&duplicate_worker).await;
    let repeated = ingest.run(duplicate.id).await.unwrap().unwrap();
    assert_eq!(repeated.state, "failed");
    assert_eq!(repeated.requests, 5);
    assert_eq!(
        ingest
            .run_checkpoint(&repeated, "traversal-v1")
            .await
            .unwrap()
            .unwrap()["error_code"],
        "repeated_page"
    );
    let rate_worker = worker(
        pool.clone(),
        &io,
        Faults {
            rate_limit_page: Some(0),
            ..Default::default()
        },
    )
    .await;
    let limited = ingest.start_run(p.id).await.unwrap();
    drain(&rate_worker).await;
    let limited_run = ingest.run(limited.id).await.unwrap().unwrap();
    assert_eq!(limited_run.state, "paused");
    assert_eq!(limited_run.requests, 5);
    assert_eq!(limited_run.items, 0);
    let checkpoint = ingest
        .run_checkpoint(&limited_run, "traversal-v1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(checkpoint["error_code"], "rate_limited");
    assert!(checkpoint["cursor"].is_null());
    ingest.cancel_run(limited.id).await.unwrap();
    let disabled_run = ingest.start_run(p.id).await.unwrap();
    let disabled_claim = ingest
        .claim("disabled-policy-worker", 300)
        .await
        .unwrap()
        .unwrap();
    ingest
        .update_policy(
            p.id,
            &PolicyWrite {
                enabled: false,
                ..selected
            },
            false,
        )
        .await
        .unwrap();
    assert!(matches!(
        ingest.retry(&disabled_claim, "unavailable", 0).await,
        Err(screenroom_backend::Error::Stale)
    ));
    assert_eq!(
        ingest.run(disabled_run.id).await.unwrap().unwrap().state,
        "cancelled"
    );
    assert!(
        ingest
            .claim("disabled-policy-worker", 300)
            .await
            .unwrap()
            .is_none()
    );
    pool.close().await;
}
