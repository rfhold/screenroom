use screenroom_backend::{
    Uuid,
    sqlx::{self, PgPool},
};
use tonic::Status;

#[derive(Clone)]
pub struct Repository {
    pool: PgPool,
}

pub struct LocalUser {
    pub id: Uuid,
    pub display_name: String,
    pub state: String,
}

pub fn uuid(value: &str) -> Result<Uuid, Status> {
    Uuid::parse_str(value).map_err(|_| Status::invalid_argument("invalid local user UUID"))
}

pub fn bounded(value: &str) -> Result<(), Status> {
    if value.trim().is_empty() || value.chars().count() > 200 || value.chars().any(char::is_control)
    {
        return Err(Status::invalid_argument("invalid identifier"));
    }
    Ok(())
}

fn database(_: sqlx::Error) -> Status {
    Status::unavailable("identity database unavailable")
}

impl Repository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub(crate) fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn ready(&self) -> Result<(), Status> {
        sqlx::query("SELECT id,display_name,state FROM identity.users LIMIT 0")
            .execute(&self.pool)
            .await
            .map_err(database)?;
        sqlx::query(
            "SELECT user_id,source_id,jellyfin_user_id FROM identity.jellyfin_user_links LIMIT 0",
        )
        .execute(&self.pool)
        .await
        .map_err(database)?;
        Ok(())
    }

    pub async fn user(&self, id: Uuid) -> Result<LocalUser, Status> {
        let (id, display_name, state) = sqlx::query_as::<_, (Uuid, String, String)>(
            "SELECT id,display_name,state FROM identity.users WHERE id=$1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(database)?
        .ok_or_else(|| Status::not_found("local user not found"))?;
        if state != "enabled" {
            return Err(Status::failed_precondition("local user disabled"));
        }
        Ok(LocalUser {
            id,
            display_name,
            state,
        })
    }

    pub async fn link(&self, id: Uuid, source: &str) -> Result<Option<String>, Status> {
        sqlx::query_scalar("SELECT jellyfin_user_id FROM identity.jellyfin_user_links WHERE user_id=$1 AND source_id=$2")
            .bind(id).bind(source).fetch_optional(&self.pool).await.map_err(database)
    }

    pub async fn revision(&self, id: Uuid) -> Result<String, Status> {
        sqlx::query_scalar(
            "SELECT updated_at::text FROM identity.users WHERE id=$1 AND state='enabled'",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(database)?
        .ok_or_else(|| Status::failed_precondition("local user unavailable"))
    }

    pub(crate) async fn set_current_link(
        &self,
        id: Uuid,
        source: &str,
        external: &str,
        revision: &str,
    ) -> Result<(), Status> {
        bounded(source)?;
        bounded(external)?;
        let mut tx = self.pool.begin().await.map_err(database)?;
        let state: Option<(String, String)> = sqlx::query_as(
            "SELECT state,updated_at::text FROM identity.users WHERE id=$1 FOR UPDATE",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(database)?;
        match state.as_ref().map(|(state, _)| state.as_str()) {
            None => return Err(Status::not_found("local user not found")),
            Some("enabled") => (),
            _ => return Err(Status::failed_precondition("local user disabled")),
        }
        if state
            .as_ref()
            .is_none_or(|(_, current)| current != revision)
        {
            return Err(Status::aborted("profile generation changed"));
        }
        let available:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM connections.jellyfin c JOIN catalog.sources s ON s.id=c.source_id WHERE s.source_key=$1 AND s.state='enabled' AND s.configuration->>'connection_id'=c.id::text AND s.configuration->>'connection_generation'=c.generation::text AND s.configuration->>'locator_revision'=c.revision)")
                .bind(source).fetch_one(&mut *tx).await.map_err(database)?;
        if !available {
            return Err(Status::unavailable("provider unavailable"));
        }
        sqlx::query("INSERT INTO identity.jellyfin_user_links(user_id,source_id,jellyfin_user_id) VALUES($1,$2,$3) ON CONFLICT(user_id,source_id) DO UPDATE SET jellyfin_user_id=EXCLUDED.jellyfin_user_id,updated_at=now() WHERE identity.jellyfin_user_links.jellyfin_user_id IS DISTINCT FROM EXCLUDED.jellyfin_user_id")
            .bind(id).bind(source).bind(external).execute(&mut *tx).await.map_err(database)?;
        sqlx::query("UPDATE identity.users SET updated_at=GREATEST(clock_timestamp(),updated_at + interval '1 microsecond') WHERE id=$1").bind(id).execute(&mut *tx).await.map_err(database)?;
        tx.commit().await.map_err(database)
    }

    pub(crate) async fn clear_enabled_link(&self, id: Uuid, source: &str) -> Result<(), Status> {
        bounded(source)?;
        let mut tx = self.pool.begin().await.map_err(database)?;
        let state: Option<String> =
            sqlx::query_scalar("SELECT state FROM identity.users WHERE id=$1 FOR UPDATE")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(database)?;
        if state.is_none() {
            return Err(Status::not_found("local user not found"));
        }
        if state.as_deref() != Some("enabled") {
            return Err(Status::failed_precondition("local user disabled"));
        }
        sqlx::query("DELETE FROM identity.jellyfin_user_links WHERE user_id=$1 AND source_id=$2")
            .bind(id)
            .bind(source)
            .execute(&mut *tx)
            .await
            .map_err(database)?;
        sqlx::query("UPDATE identity.users SET updated_at=GREATEST(clock_timestamp(),updated_at + interval '1 microsecond') WHERE id=$1").bind(id).execute(&mut *tx).await.map_err(database)?;
        tx.commit().await.map_err(database)
    }

    pub async fn init_user(&self, id: Uuid, display_name: &str) -> Result<(), Status> {
        bounded(display_name)?;
        sqlx::query("INSERT INTO identity.users(id,public_id,display_name) VALUES($1,$2,$3) ON CONFLICT(id) DO UPDATE SET display_name=EXCLUDED.display_name,updated_at=now() WHERE identity.users.display_name IS DISTINCT FROM EXCLUDED.display_name")
            .bind(id).bind(id.to_string()).bind(display_name).execute(&self.pool).await.map_err(database)?;
        Ok(())
    }

    pub async fn create_missing_user(&self, id: Uuid, display_name: &str) -> Result<bool, Status> {
        bounded(display_name)?;
        let mut tx = self.pool.begin().await.map_err(database)?;
        let created: Option<Uuid> = sqlx::query_scalar("INSERT INTO identity.users(id,public_id,display_name) VALUES($1,$2,$3) ON CONFLICT(id) DO NOTHING RETURNING id")
            .bind(id).bind(id.to_string()).bind(display_name).fetch_optional(&mut *tx).await.map_err(database)?;
        tx.commit().await.map_err(database)?;
        Ok(created.is_some())
    }
}
