use screenroom_backend::{
    Uuid,
    sqlx::{self, PgPool, postgres::PgPoolOptions},
};
use std::{
    net::{SocketAddr, TcpListener},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct ConfigFile(std::path::PathBuf);
impl ConfigFile {
    fn new() -> Self {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!(".startup-{}.json", Uuid::new_v4()));
        std::fs::write(
            &path,
            format!(
                r#"[{{"id":"{}","display_name":"Explicit startup init"}}]"#,
                Uuid::new_v4()
            ),
        )
        .unwrap();
        Self(path)
    }
}
impl Drop for ConfigFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn address() -> SocketAddr {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
}

fn command(binary: &str, runtime: &str, migration: &str) -> Command {
    let mut command = Command::new(binary);
    command
        .env("DATABASE_URL", runtime)
        .env("SCREENROOM_MIGRATION_DATABASE_URL", migration)
        .env_remove("OTEL_EXPORTER_OTLP_ENDPOINT")
        .env_remove("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT")
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    command
}

async fn exited(process: &mut Process) -> std::process::ExitStatus {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if let Some(status) = process.0.try_wait().unwrap() {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "service did not exit within deadline"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn listening(process: &mut Process, address: SocketAddr) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        assert!(
            process.0.try_wait().unwrap().is_none(),
            "service exited before listening"
        );
        if tokio::net::TcpStream::connect(address).await.is_ok() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "service did not listen within deadline"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM identity.users")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires two fresh disposable databases and separate owner/runtime URLs: USER_STARTUP_{DATABASE_URL,MIGRATION_DATABASE_URL}, USER_INIT_{DATABASE_URL,MIGRATION_DATABASE_URL}"]
async fn actual_service_and_init_migrate_fresh_schema_and_fail_closed() {
    let runtime = std::env::var("USER_STARTUP_DATABASE_URL").unwrap();
    let migration = std::env::var("USER_STARTUP_MIGRATION_DATABASE_URL").unwrap();
    let admin = PgPoolOptions::new().connect(&migration).await.unwrap();
    assert!(
        sqlx::query_scalar::<_, Option<String>>("SELECT to_regclass('identity.users')::text")
            .fetch_one(&admin)
            .await
            .unwrap()
            .is_none()
    );
    let public = address();
    let private = address();
    let mut service = Process(
        command(env!("CARGO_BIN_EXE_screenroom-user"), &runtime, &migration)
            .env("SCREENROOM_USER_LISTEN", public.to_string())
            .env("SCREENROOM_USER_PRIVATE_LISTEN", private.to_string())
            .spawn()
            .unwrap(),
    );
    listening(&mut service, public).await;
    listening(&mut service, private).await;
    assert_eq!(
        count(&admin).await,
        0,
        "service startup must not provision users"
    );
    let restricted = PgPoolOptions::new().connect(&runtime).await.unwrap();
    assert!(
        sqlx::query("CREATE TABLE identity.forbidden_startup_table(id int)")
            .execute(&restricted)
            .await
            .is_err()
    );
    #[cfg(unix)]
    {
        assert!(
            Command::new("kill")
                .args(["-TERM", &service.0.id().to_string()])
                .status()
                .unwrap()
                .success()
        );
        assert!(
            exited(&mut service).await.success(),
            "SIGTERM must shut down both listeners gracefully"
        );
    }
    #[cfg(not(unix))]
    {
        service.0.kill().unwrap();
        service.0.wait().unwrap();
    }

    let init_runtime = std::env::var("USER_INIT_DATABASE_URL").unwrap();
    let init_migration = std::env::var("USER_INIT_MIGRATION_DATABASE_URL").unwrap();
    let init_admin = PgPoolOptions::new().connect(&init_migration).await.unwrap();
    assert!(
        sqlx::query_scalar::<_, Option<String>>("SELECT to_regclass('identity.users')::text")
            .fetch_one(&init_admin)
            .await
            .unwrap()
            .is_none()
    );
    let file = ConfigFile::new();
    let mut init = Process(
        command(
            env!("CARGO_BIN_EXE_screenroom-user-init"),
            &init_runtime,
            &init_migration,
        )
        .arg(&file.0)
        .spawn()
        .unwrap(),
    );
    assert!(exited(&mut init).await.success());
    assert_eq!(count(&init_admin).await, 1);

    for pool in [&admin, &init_admin] {
        sqlx::query("UPDATE _sqlx_migrations SET checksum=$1 WHERE version=(SELECT min(version) FROM _sqlx_migrations)")
            .bind(vec![0u8; 48]).execute(pool).await.unwrap();
    }
    let mut failed = Process(
        command(env!("CARGO_BIN_EXE_screenroom-user"), &runtime, &migration)
            .env("SCREENROOM_USER_LISTEN", public.to_string())
            .env("SCREENROOM_USER_PRIVATE_LISTEN", private.to_string())
            .spawn()
            .unwrap(),
    );
    assert!(!exited(&mut failed).await.success());
    assert!(tokio::net::TcpStream::connect(public).await.is_err());
    assert!(tokio::net::TcpStream::connect(private).await.is_err());
    let file = ConfigFile::new();
    let mut failed_init = Process(
        command(
            env!("CARGO_BIN_EXE_screenroom-user-init"),
            &init_runtime,
            &init_migration,
        )
        .arg(&file.0)
        .spawn()
        .unwrap(),
    );
    assert!(!exited(&mut failed_init).await.success());
    assert_eq!(count(&admin).await, 0);
    assert_eq!(
        count(&init_admin).await,
        1,
        "migration failure must prevent explicit initialization writes"
    );
    for (process, runtime, migration) in [
        (&mut failed, &runtime, &migration),
        (&mut failed_init, &init_runtime, &init_migration),
    ] {
        use std::io::Read;
        let mut error = String::new();
        process
            .0
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut error)
            .unwrap();
        assert!(
            error.contains("Migration"),
            "startup must report migration failure"
        );
        assert!(
            !error.contains(runtime) && !error.contains(migration),
            "startup errors must not disclose connection URLs"
        );
    }
}
