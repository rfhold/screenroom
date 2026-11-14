#![allow(dead_code)]
use screenroom_backend::{
    Uuid,
    db::MIGRATOR,
    sqlx::{self, PgPool, postgres::PgPoolOptions},
};

pub async fn set_link(
    pool: &PgPool,
    user: Uuid,
    source: &str,
    profile: &str,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT id FROM identity.users WHERE id=$1 FOR UPDATE")
        .bind(user)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO identity.jellyfin_user_links(user_id,source_id,jellyfin_user_id) VALUES($1,$2,$3) ON CONFLICT(user_id,source_id) DO UPDATE SET jellyfin_user_id=EXCLUDED.jellyfin_user_id,updated_at=now()")
        .bind(user).bind(source).bind(profile).execute(&mut *tx).await?;
    sqlx::query("UPDATE identity.users SET updated_at=GREATEST(clock_timestamp(),updated_at + interval '1 microsecond') WHERE id=$1").bind(user).execute(&mut *tx).await?;
    tx.commit().await
}

pub async fn clear_link(pool: &PgPool, user: Uuid, source: &str) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT id FROM identity.users WHERE id=$1 FOR UPDATE")
        .bind(user)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM identity.jellyfin_user_links WHERE user_id=$1 AND source_id=$2")
        .bind(user)
        .bind(source)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE identity.users SET updated_at=GREATEST(clock_timestamp(),updated_at + interval '1 microsecond') WHERE id=$1").bind(user).execute(&mut *tx).await?;
    tx.commit().await
}
pub struct Fixture {
    admin: PgPool,
    pub pool: PgPool,
    pub user: PgPool,
    name: String,
}
impl Fixture {
    pub async fn new() -> Self {
        let admin = PgPool::connect(&std::env::var("SCREENROOM_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
        let name = format!("user_{}", Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE DATABASE {name}"))
            .execute(&admin)
            .await
            .unwrap();
        let options = admin.connect_options().as_ref().clone().database(&name);
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .connect_with(options.clone())
            .await
            .unwrap();
        MIGRATOR.run(&pool).await.unwrap();
        let user = PgPoolOptions::new()
            .max_connections(5)
            .after_connect(|c, _| {
                Box::pin(async move {
                    sqlx::query("SET ROLE screenroom_user").execute(c).await?;
                    Ok(())
                })
            })
            .connect_with(options)
            .await
            .unwrap();
        Self {
            admin,
            pool,
            user,
            name,
        }
    }
    pub async fn cleanup(self) {
        assert!(
            self.name
                .strip_prefix("user_")
                .is_some_and(|v| Uuid::parse_str(v).is_ok())
        );
        self.user.close().await;
        self.pool.close().await;
        sqlx::query(&format!("DROP DATABASE {} WITH (FORCE)", self.name))
            .execute(&self.admin)
            .await
            .unwrap();
        self.admin.close().await;
    }
}
pub struct Managed {
    pub directory: screenroom_backend::connections::CredentialDirectory,
    file: std::path::PathBuf,
}
impl Drop for Managed {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.file);
        let _ = std::fs::remove_dir(self.file.parent().unwrap());
    }
}
pub async fn managed(
    pool: &screenroom_backend::sqlx::PgPool,
    source: &str,
    url: &str,
    profile: &str,
) -> Managed {
    let path = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("user-credentials-{}", Uuid::new_v4()));
    managed_in(pool, source, url, profile, path).await
}
pub async fn managed_in(
    pool: &PgPool,
    source: &str,
    url: &str,
    profile: &str,
    path: std::path::PathBuf,
) -> Managed {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir(&path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let credential = Uuid::new_v4();
    let file = path.join(format!("{credential}.key"));
    std::fs::write(&file, "test-owned-token").unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
    let connection = Uuid::new_v4();
    let configuration = serde_json::json!({"adapter":"jellyfin","locator_revision":"a".repeat(64),"connection_id":connection,"connection_generation":1});
    let id:Uuid=sqlx::query_scalar("INSERT INTO catalog.sources(source_key,provider,configuration) VALUES($1,'jellyfin',$2) ON CONFLICT(source_key) DO UPDATE SET configuration=EXCLUDED.configuration RETURNING id").bind(source).bind(configuration).fetch_one(pool).await.unwrap();
    sqlx::query("INSERT INTO connections.jellyfin(id,source_id,server_id,server_name,server_version,base_url,credential_ref,revision,indexing_profile_id) VALUES($1,$2,'user-test','Owned double','10.11.0',$3,$4,$5,$6)").bind(connection).bind(id).bind(url).bind(credential).bind("a".repeat(64)).bind(profile).execute(pool).await.unwrap();
    Managed {
        directory: screenroom_backend::connections::CredentialDirectory::new(path).unwrap(),
        file,
    }
}
