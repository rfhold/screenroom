use screenroom_backend::{
    Uuid,
    catalog::CatalogRepository,
    ingest::{IngestRepository, PolicyWrite},
    sqlx::{self, postgres::PgPoolOptions},
};
use serde_json::json;

fn command(url: &str, path: &std::path::Path) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_screenroom-catalog-init"));
    command
        .arg(path)
        .env("DATABASE_URL", url)
        .env("SCREENROOM_MIGRATION_DATABASE_URL", url)
        .kill_on_drop(true);
    command
}

#[tokio::test]
#[ignore = "requires explicit task-owned disposable PostgreSQL 18 + pgvector URL"]
async fn atomic_initializer_and_startup() {
    let base = std::env::var("SCREENROOM_G_TEST_DATABASE_URL").expect("task-owned database URL");
    let installation = PgPoolOptions::new()
        .max_connections(2)
        .connect(&base)
        .await
        .unwrap();
    let database = format!("screenroom_g_init_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {database}"))
        .execute(&installation)
        .await
        .unwrap();
    let url = format!("{}/{database}", base.rsplit_once('/').unwrap().0);
    let pool = PgPoolOptions::new()
        .max_connections(3)
        .connect(&url)
        .await
        .unwrap();
    let path = std::env::temp_dir().join(format!("screenroom-g-init-{}.json", Uuid::new_v4()));
    let policy = PolicyWrite {
        enabled: true,
        selection_json: json!({"library_ids":["movies"]}),
        max_items: 100,
        max_requests: 100,
        incremental_seconds: 60,
        reconcile_seconds: 3600,
        enrich_metadata: false,
        enrich_people: false,
        enrich_artwork: false,
    };
    let source = json!({"key":"installed", "provider":"jellyfin", "configuration":{"adapter":"mock"},
        "capabilities":{}, "libraries":[{"id":"movies","external_id":"movies","title":"Movies"}], "policy":policy});
    std::fs::write(
        &path,
        serde_json::to_vec(&json!({"sources":[source.clone()]})).unwrap(),
    )
    .unwrap();
    let (first, second) =
        tokio::join!(command(&url, &path).output(), command(&url, &path).output());
    for result in [first.unwrap(), second.unwrap()] {
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let catalog = CatalogRepository::new(pool.clone());
    let ingest = IngestRepository::new(pool.clone());
    let installed = catalog.sources().await.unwrap().remove(0);
    let policies = ingest.policies(Some(installed.id)).await.unwrap();
    assert_eq!(policies.len(), 1);
    catalog.set_source_state(installed.id, false).await.unwrap();
    sqlx::query("UPDATE catalog.libraries SET state='disabled' WHERE source_id=$1")
        .bind(installed.id)
        .execute(&pool)
        .await
        .unwrap();
    let mut operator_policy = policy;
    operator_policy.enabled = false;
    operator_policy.max_items = 17;
    ingest
        .update_policy(policies[0].id, &operator_policy, false)
        .await
        .unwrap();
    let mut changed = source.clone();
    changed["configuration"] = json!({"adapter":"operator-controlled"});
    changed["libraries"][0]["title"] = json!("Do not overwrite");
    std::fs::write(
        &path,
        serde_json::to_vec(&json!({"sources":[changed]})).unwrap(),
    )
    .unwrap();
    assert!(
        command(&url, &path)
            .output()
            .await
            .unwrap()
            .status
            .success()
    );
    let installed_again = catalog.sources().await.unwrap().remove(0);
    assert_eq!(installed_again.state, "disabled");
    assert_eq!(installed_again.configuration, installed.configuration);
    let libraries: Vec<(String, String)> =
        sqlx::query_as("SELECT title,state FROM catalog.libraries WHERE source_id=$1")
            .bind(installed.id)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(libraries, vec![("Movies".into(), "disabled".into())]);
    let policies = ingest.policies(Some(installed.id)).await.unwrap();
    assert_eq!(policies.len(), 1);
    assert_eq!(policies[0].version, 2);
    assert!(!policies[0].enabled);
    assert_eq!(policies[0].max_items, 17);
    let mut conflict = source.clone();
    conflict["provider"] = json!("tmdb");
    conflict["policy"] = serde_json::Value::Null;
    let mut new_source = source;
    new_source["key"] = json!("a-new-before-conflict");
    std::fs::write(
        &path,
        serde_json::to_vec(&json!({"sources":[new_source,conflict]})).unwrap(),
    )
    .unwrap();
    assert!(
        !command(&url, &path)
            .output()
            .await
            .unwrap()
            .status
            .success()
    );
    assert_eq!(
        catalog.sources().await.unwrap().len(),
        1,
        "late source mismatch must roll back earlier installation writes"
    );
    sqlx::query("UPDATE _sqlx_migrations SET checksum=decode('00','hex') WHERE version=(SELECT min(version) FROM _sqlx_migrations)").execute(&pool).await.unwrap();
    let failure = command(&url, &path).output().await.unwrap();
    assert!(!failure.status.success());
    assert!(!String::from_utf8_lossy(&failure.stderr).contains(&url));
    let mut main = tokio::process::Command::new(env!("CARGO_BIN_EXE_screenroom-catalog"));
    let failure = main
        .env("DATABASE_URL", &url)
        .env("SCREENROOM_MIGRATION_DATABASE_URL", &url)
        .env("SCREENROOM_CATALOG_LISTEN", "127.0.0.1:0")
        .env("SCREENROOM_CATALOG_PRIVATE_LISTEN", "127.0.0.1:0")
        .env("SCREENROOM_CATALOG_PUBLIC_ORIGIN", "http://localhost:50052")
        .output()
        .await
        .unwrap();
    assert!(!failure.status.success());
    assert!(!String::from_utf8_lossy(&failure.stdout).contains("RPC server listening"));
    assert!(!String::from_utf8_lossy(&failure.stderr).contains(&url));
    let fresh_database = format!("{database}_main");
    sqlx::query(&format!("CREATE DATABASE {fresh_database}"))
        .execute(&installation)
        .await
        .unwrap();
    let fresh_url = format!("{}/{fresh_database}", base.rsplit_once('/').unwrap().0);
    let mut fresh = tokio::process::Command::new(env!("CARGO_BIN_EXE_screenroom-catalog"))
        .env("DATABASE_URL", &fresh_url)
        .env("SCREENROOM_MIGRATION_DATABASE_URL", &fresh_url)
        .env("SCREENROOM_CATALOG_LISTEN", "127.0.0.1:0")
        .env("SCREENROOM_CATALOG_PRIVATE_LISTEN", "127.0.0.1:0")
        .env("SCREENROOM_CATALOG_PUBLIC_ORIGIN", "http://localhost:50052")
        .env("RUST_LOG", "info")
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut lines =
        tokio::io::AsyncBufReadExt::lines(tokio::io::BufReader::new(fresh.stdout.take().unwrap()));
    let address = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while let Some(line) = lines.next_line().await.unwrap() {
            if let Ok(event) = serde_json::from_str::<serde_json::Value>(&line)
                && event["fields"]["message"] == "RPC server listening"
            {
                return event["fields"]["address"].as_str().unwrap().to_owned();
            }
        }
        panic!("fresh catalog did not become ready");
    })
    .await
    .unwrap();
    let channel = tonic::transport::Endpoint::from_shared(format!("http://{address}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    let info =
        screenroom_proto::catalog::v1::catalog_service_client::CatalogServiceClient::new(channel)
            .get_catalog_info(screenroom_proto::catalog::v1::GetCatalogInfoRequest {})
            .await
            .unwrap()
            .into_inner()
            .info
            .unwrap();
    assert!(info.branding.is_empty());
    assert!(info.notices.is_empty());
    assert!(
        tokio::process::Command::new("kill")
            .args(["-TERM", &fresh.id().unwrap().to_string()])
            .status()
            .await
            .unwrap()
            .success()
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(7), fresh.wait())
            .await
            .unwrap()
            .unwrap()
            .success()
    );
    sqlx::query(&format!("DROP DATABASE {fresh_database} WITH (FORCE)"))
        .execute(&installation)
        .await
        .unwrap();
    std::fs::remove_file(path).unwrap();
    pool.close().await;
    sqlx::query(&format!("DROP DATABASE {database} WITH (FORCE)"))
        .execute(&installation)
        .await
        .unwrap();
    installation.close().await;
}
