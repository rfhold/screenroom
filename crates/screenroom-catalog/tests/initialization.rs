use screenroom_backend::{
    Uuid,
    db::MIGRATOR,
    sqlx::{self, postgres::PgPoolOptions},
};

#[tokio::test]
#[ignore = "requires explicit task-owned disposable PostgreSQL 18 + pgvector URL"]
async fn startup_rejects_checksum_drift_and_serves_fresh_empty_catalog() {
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
    MIGRATOR.run(&pool).await.unwrap();
    sqlx::query("UPDATE _sqlx_migrations SET checksum=decode('00','hex') WHERE version=(SELECT min(version) FROM _sqlx_migrations)").execute(&pool).await.unwrap();
    let failure = tokio::process::Command::new(env!("CARGO_BIN_EXE_screenroom-catalog"))
        .env("DATABASE_URL", &url)
        .env("SCREENROOM_MIGRATION_DATABASE_URL", &url)
        .env("SCREENROOM_CATALOG_LISTEN", "127.0.0.1:0")
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
    pool.close().await;
    sqlx::query(&format!("DROP DATABASE {database} WITH (FORCE)"))
        .execute(&installation)
        .await
        .unwrap();
    installation.close().await;
}
