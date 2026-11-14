//! Provider-neutral complete personal snapshots. Reads never contact providers.
use crate::{
    Error, Result, Uuid,
    sqlx::{self, PgPool},
};
use chrono::{DateTime, Utc};
use screenroom_catalog_adapter::user_state::WatchedSnapshot;
use std::collections::BTreeMap;

pub const FRESH_SECONDS: i32 = 900;

#[derive(Clone, Debug)]
pub struct Binding {
    pub user_id: Uuid,
    pub source_id: Uuid,
    pub revision: String,
}

#[derive(Clone)]
pub struct WatchedRepository {
    pool: PgPool,
}

impl WatchedRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn revision(&self, user: Uuid) -> Result<Option<String>> {
        Ok(sqlx::query_scalar(
            "SELECT updated_at::text FROM identity.users WHERE id=$1 AND state='enabled'",
        )
        .bind(user)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn publish(
        &self,
        binding: &Binding,
        snapshot: &WatchedSnapshot,
        fetched_at: DateTime<Utc>,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let valid: Option<bool> = sqlx::query_scalar("SELECT updated_at::text=$2 AND state='enabled' FROM identity.users WHERE id=$1 FOR UPDATE")
            .bind(binding.user_id).bind(&binding.revision).fetch_optional(&mut *tx).await?;
        if valid != Some(true) {
            return Err(Error::Stale);
        }
        let enabled: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM catalog.sources WHERE id=$1 AND state='enabled')",
        )
        .bind(binding.source_id)
        .fetch_one(&mut *tx)
        .await?;
        if !enabled {
            return Err(Error::Stale);
        }
        let previous: Option<DateTime<Utc>> = sqlx::query_scalar("SELECT fetched_at FROM personal.watched_snapshots WHERE user_id=$1 AND source_id=$2 AND binding_revision=$3")
            .bind(binding.user_id).bind(binding.source_id).bind(&binding.revision).fetch_optional(&mut *tx).await?;
        if previous.is_some_and(|previous| previous > fetched_at) {
            return Err(Error::Stale);
        }
        // Replacement and observations become visible together; failed publication rolls back.
        sqlx::query("DELETE FROM personal.watched_snapshots WHERE user_id=$1 AND source_id=$2")
            .bind(binding.user_id)
            .bind(binding.source_id)
            .execute(&mut *tx)
            .await?;
        let generation = Uuid::new_v4();
        sqlx::query("INSERT INTO personal.watched_snapshots(user_id,source_id,generation,binding_revision,fetched_at,complete) VALUES($1,$2,$3,$4,$5,true)")
            .bind(binding.user_id).bind(binding.source_id).bind(generation).bind(&binding.revision).bind(fetched_at).execute(&mut *tx).await?;
        for batch in snapshot.observations().chunks(500) {
            let ids: Vec<_> = batch.iter().map(|o| o.external_item_id.clone()).collect();
            let states: Vec<_> = batch.iter().map(|o| o.watched).collect();
            sqlx::query("INSERT INTO personal.watched_observations(generation,external_item_id,watched) SELECT $1,unnest($2::text[]),unnest($3::boolean[])")
                .bind(generation).bind(ids).bind(states).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn invalidate(&self, binding: &Binding) -> Result<()> {
        sqlx::query("DELETE FROM personal.watched_snapshots WHERE user_id=$1 AND source_id=$2 AND binding_revision=$3")
            .bind(binding.user_id).bind(binding.source_id).bind(&binding.revision).execute(&self.pool).await?;
        Ok(())
    }

    /// Keys are chosen source-item row IDs, not canonical IDs or any-provider matches.
    pub async fn read(&self, user: Uuid, source_items: &[Uuid]) -> Result<BTreeMap<Uuid, bool>> {
        let rows: Vec<(Uuid, bool)> = sqlx::query_as("SELECT si.id,o.watched FROM catalog.source_items si JOIN catalog.sources s ON s.id=si.source_id AND s.state='enabled' JOIN personal.watched_snapshots w ON w.source_id=si.source_id AND w.user_id=$1 JOIN identity.users u ON u.id=w.user_id AND u.state='enabled' AND u.updated_at::text=w.binding_revision JOIN personal.watched_observations o ON o.generation=w.generation AND o.external_item_id=si.external_id WHERE si.id=ANY($2) AND si.state='enabled' AND w.complete AND w.fetched_at <= now() AND w.fetched_at > now()-make_interval(secs => $3::int) AND o.watched IS NOT NULL")
            .bind(user).bind(source_items).bind(FRESH_SECONDS).fetch_all(&self.pool).await?;
        Ok(rows.into_iter().collect())
    }
}
