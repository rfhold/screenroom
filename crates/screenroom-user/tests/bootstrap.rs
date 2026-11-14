use screenroom_backend::{
    Uuid,
    sqlx::{self, PgPool, postgres::PgPoolOptions},
};
use screenroom_user::repository::Repository;
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
};
mod support;

struct Config(PathBuf);
impl Config {
    fn new(value: Value) -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!(".bootstrap-{}.json", Uuid::new_v4()));
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        Self(path)
    }
}
impl Drop for Config {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn launch(config: &Config, create_missing: bool, runtime: &str, migration: &str) -> Process {
    let mut command = Command::new(env!("CARGO_BIN_EXE_screenroom-user-init"));
    command
        .env("DATABASE_URL", runtime)
        .env("SCREENROOM_MIGRATION_DATABASE_URL", migration)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if create_missing {
        command.arg("--create-missing");
    }
    Process(command.arg(&config.0).spawn().unwrap())
}
async fn success(process: &mut Process) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        if let Some(status) = process.0.try_wait().unwrap() {
            return status.success();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "init exceeded timeout"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}
fn user(id: Uuid, name: &str) -> Value {
    json!({"id": id, "display_name": name})
}
async fn snapshot(pool: &PgPool) -> (Vec<String>, Vec<String>) {
    (
        sqlx::query_scalar("SELECT row_to_json(u)::text FROM identity.users u ORDER BY id").fetch_all(pool).await.unwrap(),
        sqlx::query_scalar("SELECT row_to_json(l)::text FROM identity.jellyfin_user_links l ORDER BY user_id,source_id").fetch_all(pool).await.unwrap(),
    )
}

#[test]
fn legacy_link_configuration_is_rejected_before_database_access() {
    for links in [
        json!([]),
        json!(null),
        json!([{"source_id":"owned", "jellyfin_user_id":"unchecked"}]),
    ] {
        let config =
            Config::new(json!([{"id":Uuid::new_v4(),"display_name":"Viewer","links":links}]));
        let output = Command::new(env!("CARGO_BIN_EXE_screenroom-user-init"))
            .env_remove("DATABASE_URL")
            .arg(&config.0)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("select a validated Jellyfin profile")
        );
    }
}

#[tokio::test]
#[ignore = "requires fresh disposable DB and separate restricted/owner LOGIN URLs: USER_BOOTSTRAP_DATABASE_URL, USER_BOOTSTRAP_MIGRATION_DATABASE_URL"]
async fn create_only_bootstrap_preserves_operator_state_and_is_atomic() {
    let runtime = std::env::var("USER_BOOTSTRAP_DATABASE_URL").unwrap();
    let migration = std::env::var("USER_BOOTSTRAP_MIGRATION_DATABASE_URL").unwrap();
    let (alice, bob) = (Uuid::new_v4(), Uuid::new_v4());
    let canonical = Config::new(json!([user(alice, "Alice"), user(bob, "Bob")]));
    assert!(success(&mut launch(&canonical, true, &runtime, &migration)).await);
    let pool = PgPoolOptions::new().connect(&runtime).await.unwrap();
    let repository = Repository::new(pool.clone());
    assert!(
        sqlx::query("CREATE TABLE identity.bootstrap_forbidden(id int)")
            .execute(&pool)
            .await
            .is_err()
    );
    assert!(repository.link(alice, "owned").await.unwrap().is_none());
    repository.init_user(alice, "Operator name").await.unwrap();
    support::set_link(&pool, alice, "owned", "operator-remap")
        .await
        .unwrap();
    sqlx::query("UPDATE identity.users SET display_name='Disabled operator name',state='disabled',updated_at=now() WHERE id=$1").bind(bob).execute(&pool).await.unwrap();
    let before = snapshot(&pool).await;
    assert!(success(&mut launch(&canonical, true, &runtime, &migration)).await);
    assert_eq!(
        snapshot(&pool).await,
        before,
        "bootstrap preserves identity, links, disabled state and timestamps"
    );
    let concurrent = Uuid::new_v4();
    let one = Config::new(json!([user(concurrent, "First candidate")]));
    let two = Config::new(json!([user(concurrent, "Second candidate")]));
    let mut first = launch(&one, true, &runtime, &migration);
    let mut second = launch(&two, true, &runtime, &migration);
    assert!(success(&mut first).await);
    assert!(success(&mut second).await);
    assert!(matches!(
        repository
            .user(concurrent)
            .await
            .unwrap()
            .display_name
            .as_str(),
        "First candidate" | "Second candidate"
    ));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM identity.users WHERE id=$1")
        .bind(concurrent)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let invalid = Uuid::new_v4();
    assert!(repository.create_missing_user(invalid, "").await.is_err());
    assert!(repository.user(invalid).await.is_err());
    let invalid_json = Config::new(json!([
        user(Uuid::new_v4(), "Should not write"),
        user(Uuid::new_v4(), "")
    ]));
    let before = snapshot(&pool).await;
    assert!(!success(&mut launch(&invalid_json, true, &runtime, &migration)).await);
    assert_eq!(
        snapshot(&pool).await,
        before,
        "whole input validates before writes"
    );
    let explicit = Config::new(json!([user(alice, "Explicit replacement")]));
    assert!(success(&mut launch(&explicit, false, &runtime, &migration)).await);
    assert_eq!(
        repository.user(alice).await.unwrap().display_name,
        "Explicit replacement"
    );
    assert_eq!(
        repository.link(alice, "owned").await.unwrap().as_deref(),
        Some("operator-remap")
    );
}
