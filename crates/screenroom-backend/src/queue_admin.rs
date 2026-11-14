use crate::{
    Error, Result, Uuid,
    ingest::{EXECUTION_BUDGET_SQL, IngestRepository, Run, WorkBudget, lock_policy_source},
    queue,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct QueueSnapshot {
    pub ready: i64,
    pub delayed: i64,
    pub leased: i64,
    pub failed: i64,
    pub cancelled: i64,
    pub complete: i64,
    pub claimable: i64,
    pub reclaimable: i64,
    pub blocked: i64,
    pub exhausted: i64,
    pub oldest_ready_seconds: Option<f64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueObservation {
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub finished_at: chrono::DateTime<chrono::Utc>,
    pub counts: QueueSnapshot,
}
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct JobSummary {
    pub id: Uuid,
    pub run_id: Option<Uuid>,
    pub kind: String,
    pub state: String,
    pub available_at: chrono::DateTime<chrono::Utc>,
    pub lease_until: Option<chrono::DateTime<chrono::Utc>>,
    pub attempts: i32,
    pub max_attempts: i32,
    pub generation: i32,
}
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct QueueBlockers {
    pub deadline_expired: i64,
    pub policy_disabled: i64,
    pub policy_version: i64,
    pub source_disabled: i64,
    pub source_changed: i64,
    pub source_concurrency: i64,
    pub run_state: i64,
    pub cooldown: i64,
    pub capability: Option<i64>,
    pub attempt_limit: i64,
    pub next_eligible_at: Option<chrono::DateTime<chrono::Utc>>,
    pub as_of: chrono::DateTime<chrono::Utc>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedriveEligibility {
    pub allowed: bool,
    pub reason: Option<String>,
}
impl IngestRepository {
    /// Independent observation interval, not an atomic snapshot with blocker diagnostics.
    pub async fn queue_observation(
        &self,
        run: Option<Uuid>,
        capabilities: &[String],
    ) -> Result<QueueObservation> {
        let started_at = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&self.pool)
            .await?;
        let counts = self.queue_snapshot(run, capabilities).await?;
        let finished_at = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&self.pool)
            .await?;
        Ok(QueueObservation {
            started_at,
            finished_at,
            counts,
        })
    }
    pub async fn redrive_eligibility(&self, job: Uuid) -> Result<RedriveEligibility> {
        match self.redrive_job_inner(job, Uuid::new_v4(), true).await {
            Ok(_) => Ok(RedriveEligibility {
                allowed: true,
                reason: None,
            }),
            Err(Error::Stale | Error::NotFound) => Ok(RedriveEligibility {
                allowed: false,
                reason: Some("not_current_terminal_job".into()),
            }),
            Err(Error::Invalid("indexing deadline expired")) => Ok(RedriveEligibility {
                allowed: false,
                reason: Some("deadline_expired".into()),
            }),
            Err(Error::Invalid(_)) => Ok(RedriveEligibility {
                allowed: false,
                reason: Some("unsupported_payload".into()),
            }),
            Err(error) => Err(error),
        }
    }
    /// Reasons may overlap; capability evaluation is unknown without an explicit worker context.
    pub async fn queue_blockers(
        &self,
        run: Option<Uuid>,
        caps: Option<&[String]>,
    ) -> Result<QueueBlockers> {
        Ok(sqlx::query_as(&format!(r#"WITH pending AS (
          SELECT j.*,p.enabled AS policy_enabled,p.version=r.policy_version AS policy_current,
           s.state='enabled' AS source_enabled,(r.state IN ('running','complete') AND ({EXECUTION_BUDGET_SQL}) AND EXISTS(SELECT FROM ingest.indexing_runs ir WHERE ir.run_id=r.id)) AS run_current,
          NOT EXISTS(SELECT FROM ingest.indexing_runs ir WHERE ir.run_id=r.id AND ir.source_configuration<>s.configuration) AS source_current,
          ingest.indexing_source_available(r.id) AS source_available,ingest.indexing_deadline_valid(r.id) AS deadline_valid,sc.until_at
          FROM queue.jobs j LEFT JOIN ingest.runs r ON r.id=j.scope_id
          LEFT JOIN ingest.policies p ON p.id=r.policy_id LEFT JOIN catalog.sources s ON s.id=r.source_id
          LEFT JOIN ingest.source_cooldowns sc ON sc.source_id=s.id
          WHERE j.queue_name='indexing' AND ($1::uuid IS NULL OR j.scope_id=$1)
          AND (j.state='ready' OR (j.state='leased' AND j.lease_until<=statement_timestamp())))
        SELECT count(*) FILTER(WHERE NOT COALESCE(policy_enabled,false)) AS policy_disabled,
          count(*) FILTER(WHERE NOT COALESCE(policy_current,false)) AS policy_version,
          count(*) FILTER(WHERE NOT COALESCE(source_enabled,false)) AS source_disabled,
          count(*) FILTER(WHERE NOT source_current) AS source_changed,
          count(*) FILTER(WHERE NOT source_available) AS source_concurrency,
          count(*) FILTER(WHERE NOT deadline_valid) AS deadline_expired,
          count(*) FILTER(WHERE NOT COALESCE(run_current,false)) AS run_state,
          count(*) FILTER(WHERE until_at>statement_timestamp()) AS cooldown,
          CASE WHEN $2::text[] IS NULL THEN NULL ELSE count(*) FILTER(WHERE NOT requirements<@$2) END AS capability,
          count(*) FILTER(WHERE attempts>=max_attempts) AS attempt_limit,
          min(GREATEST(available_at,COALESCE(until_at,available_at),statement_timestamp())) FILTER(
            WHERE policy_enabled AND policy_current AND source_enabled AND run_current AND source_current
            AND source_available AND deadline_valid AND attempts<max_attempts AND ($2::text[] IS NULL OR requirements<@$2)) AS next_eligible_at,
           statement_timestamp() AS as_of FROM pending"#)).bind(run).bind(caps).fetch_one(&self.pool).await?)
    }
    /// Payloads, provider keys and lease-owner strings are intentionally excluded.
    pub async fn jobs(
        &self,
        run: Uuid,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<JobSummary>> {
        if !(1..=200).contains(&limit) {
            return Err(Error::Invalid("page"));
        }
        Ok(sqlx::query_as("SELECT id,scope_id AS run_id,kind,state,available_at,lease_until,attempts,max_attempts,redrive_generation AS generation FROM queue.jobs WHERE queue_name='indexing' AND scope_id=$1 AND ($2::uuid IS NULL OR id>$2) ORDER BY id LIMIT $3").bind(run).bind(after).bind(i64::from(limit)).fetch_all(&self.pool).await?)
    }
    /// Counts are one snapshot; claimable excludes expired leases, reported as reclaimable.
    pub async fn queue_snapshot(
        &self,
        run: Option<Uuid>,
        capabilities: &[String],
    ) -> Result<QueueSnapshot> {
        Ok(sqlx::query_as(&format!(r#"WITH jobs AS (
          SELECT j.*,COALESCE(p.enabled AND p.version=r.policy_version AND s.state='enabled'
           AND r.state IN ('running','complete') AND ({EXECUTION_BUDGET_SQL})
           AND EXISTS(SELECT FROM ingest.indexing_runs ir WHERE ir.run_id=r.id) AND ingest.indexing_deadline_valid(r.id)
          AND ingest.indexing_source_available(r.id)
          AND NOT EXISTS(SELECT FROM ingest.indexing_runs ir WHERE ir.run_id=r.id AND ir.source_configuration<>s.configuration)
          AND NOT EXISTS(SELECT FROM ingest.source_cooldowns sc WHERE sc.source_id=s.id AND sc.until_at>statement_timestamp()),false)
          AND requirements<@$2::text[] AS admitted FROM queue.jobs j LEFT JOIN ingest.runs r ON r.id=j.scope_id
          LEFT JOIN ingest.policies p ON p.id=r.policy_id LEFT JOIN catalog.sources s ON s.id=r.source_id
          WHERE j.queue_name='indexing' AND ($1::uuid IS NULL OR j.scope_id=$1))
        SELECT count(*) FILTER(WHERE state='ready' AND available_at<=statement_timestamp()) AS ready,
          count(*) FILTER(WHERE state='ready' AND available_at>statement_timestamp()) AS delayed,
          count(*) FILTER(WHERE state='leased') AS leased,count(*) FILTER(WHERE state='failed') AS failed,
          count(*) FILTER(WHERE state='cancelled') AS cancelled,count(*) FILTER(WHERE state='complete') AS complete,
          count(*) FILTER(WHERE state='ready' AND available_at<=statement_timestamp() AND admitted AND attempts<max_attempts) AS claimable,
          count(*) FILTER(WHERE state='leased' AND lease_until<=statement_timestamp() AND admitted AND attempts<max_attempts) AS reclaimable,
          count(*) FILTER(WHERE ((state='ready' AND available_at<=statement_timestamp()) OR (state='leased' AND lease_until<=statement_timestamp())) AND NOT admitted) AS blocked,
          count(*) FILTER(WHERE state IN ('ready','leased') AND (state='ready' OR lease_until<=statement_timestamp()) AND attempts>=max_attempts) AS exhausted,
           EXTRACT(epoch FROM statement_timestamp()-min(available_at) FILTER(WHERE state='ready' AND available_at<=statement_timestamp()))::float8 AS oldest_ready_seconds FROM jobs"#))
            .bind(run).bind(capabilities).fetch_one(&self.pool).await?)
    }
    pub async fn job_attempts(
        &self,
        job: Uuid,
        after_fence: i64,
        limit: u32,
    ) -> Result<Vec<queue::Attempt>> {
        let mut tx = self.pool.begin().await?;
        let attempts = queue::attempts(&mut tx, job, after_fence, limit).await?;
        tx.commit().await?;
        Ok(attempts)
    }
    pub async fn job_redrives(
        &self,
        job: Uuid,
        after_generation: i32,
        limit: u32,
    ) -> Result<Vec<queue::RedriveResult>> {
        let mut tx = self.pool.begin().await?;
        let redrives = queue::redrives(&mut tx, job, after_generation, limit).await?;
        tx.commit().await?;
        Ok(redrives)
    }
    /// Failed identities only. No request/window refund. Superseded runs and payloads are rejected.
    /// A failed traversal must be redriven first; its pending followups then become eligible again.
    pub async fn redrive_job(&self, job: Uuid, request: Uuid) -> Result<queue::RedriveResult> {
        self.redrive_job_inner(job, request, false)
            .await?
            .ok_or(Error::Stale)
    }
    async fn redrive_job_inner(
        &self,
        job: Uuid,
        request: Uuid,
        check_only: bool,
    ) -> Result<Option<queue::RedriveResult>> {
        let mut tx = self.pool.begin().await?;
        if let Some(receipt) = queue::redrive_receipt(&mut tx, job, request).await? {
            tx.commit().await?;
            return Ok(Some(receipt));
        }
        let policy_id: Uuid=sqlx::query_scalar("SELECT r.policy_id FROM queue.jobs j JOIN ingest.runs r ON r.id=j.scope_id WHERE j.id=$1 AND j.queue_name='indexing'").bind(job).fetch_optional(&mut *tx).await?.ok_or(Error::NotFound)?;
        lock_policy_source(&mut tx, policy_id).await?;
        let (enabled, version): (bool, i64) =
            sqlx::query_as("SELECT enabled,version FROM ingest.policies WHERE id=$1 FOR UPDATE")
                .bind(policy_id)
                .fetch_one(&mut *tx)
                .await?;
        let run: Run=sqlx::query_as("SELECT r.* FROM ingest.runs r JOIN queue.jobs j ON j.scope_id=r.id WHERE j.id=$1 FOR UPDATE OF r").bind(job).fetch_one(&mut *tx).await?;
        let admitted: bool =
            sqlx::query_scalar("SELECT EXISTS(SELECT FROM ingest.indexing_runs WHERE run_id=$1)")
                .bind(run.id)
                .fetch_one(&mut *tx)
                .await?;
        if !admitted || WorkBudget::from_selection(&run.selection_snapshot).is_err() {
            return Err(Error::Stale);
        }
        crate::indexing::check_deadline(&mut tx, &run).await?;
        let (kind, input, state): (String, Value, String) =
            sqlx::query_as("SELECT kind,input,state FROM queue.jobs WHERE id=$1 FOR UPDATE")
                .bind(job)
                .fetch_one(&mut *tx)
                .await?;
        let current: bool=sqlx::query_scalar("SELECT s.state='enabled' AND NOT EXISTS(SELECT FROM ingest.runs newer WHERE newer.source_id=s.id AND newer.id<>$2 AND (newer.started_at>$3 OR newer.state IN ('running','paused'))) FROM catalog.sources s WHERE s.id=$1")
            .bind(run.source_id).bind(run.id).bind(run.started_at).fetch_one(&mut *tx).await?;
        let source_current:bool=sqlx::query_scalar("SELECT NOT EXISTS(SELECT FROM ingest.indexing_runs ir JOIN catalog.sources s ON s.id=$2 WHERE ir.run_id=$1 AND ir.source_configuration<>s.configuration)").bind(run.id).bind(run.source_id).fetch_one(&mut *tx).await?;
        if !enabled
            || !current
            || !source_current
            || version != run.policy_version
            || state != "failed"
            || !matches!(run.state.as_str(), "running" | "complete" | "failed")
            || (run.state == "failed" && kind != "ingest")
        {
            return Err(Error::Stale);
        }
        if kind == "embedding" {
            let valid: bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM search.documents d JOIN catalog.source_items si ON si.id=d.source_item_id JOIN search.embedding_models m ON m.id=$3 WHERE si.source_id=$1 AND si.external_id=$2 AND si.state='enabled' AND m.state<>'retired' AND encode(d.content_hash,'hex')=$4)")
                .bind(run.source_id).bind(input["external_id"].as_str()).bind(run.embedding_model_id).bind(input["input_hash"].as_str()).fetch_one(&mut *tx).await?;
            if !valid || input["model_id"] != serde_json::json!(run.embedding_model_id) {
                return Err(Error::Stale);
            }
        } else if kind == "artwork" {
            let payload: crate::media::ArtworkJob = serde_json::from_value(input)
                .map_err(|_| Error::Invalid("artwork redrive payload"))?;
            let valid: bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM media.artwork a JOIN media.artwork_generations g ON g.artwork_id=a.id AND g.revision=a.current_generation WHERE a.id=$1 AND a.source_id=$2 AND a.state='enabled' AND g.revision=$3 AND g.fingerprint=$4 AND g.state IN ('pending','published'))")
                .bind(payload.artwork_id).bind(run.source_id).bind(payload.revision).bind(payload.fingerprint.as_slice()).fetch_one(&mut *tx).await?;
            if !valid {
                return Err(Error::Stale);
            }
        } else if kind != "ingest" {
            return Err(Error::Invalid("redrive job kind"));
        }
        if check_only {
            tx.rollback().await?;
            return Ok(None);
        }
        // Keep the failed coverage and all request receipts; redrive lineage explains this transition.
        if run.state == "failed" {
            sqlx::query("UPDATE ingest.runs SET state='running',finished_at=NULL WHERE id=$1")
                .bind(run.id)
                .execute(&mut *tx)
                .await?;
        }
        let result = queue::redrive(&mut tx, job, request).await?;
        sqlx::query("INSERT INTO ingest.run_redrives(request_id,run_id,prior_state,prior_finished_at,prior_coverage) VALUES($1,$2,$3,$4,$5)").bind(request).bind(run.id).bind(&run.state).bind(run.finished_at).bind(&run.coverage).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(Some(result))
    }
}
