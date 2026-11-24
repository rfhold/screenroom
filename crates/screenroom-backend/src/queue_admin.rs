use crate::{
    Error, Result, Uuid,
    ingest::{EXECUTION_BUDGET_SQL, IngestRepository, Run, WorkBudget, lock_policy_source},
    queue,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{FromRow, PgConnection, types::Json};
use std::collections::HashMap;

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

#[derive(FromRow)]
struct RedriveCheck {
    id: Uuid,
    run: Json<Run>,
    admitted: bool,
    current_run: bool,
    kind: String,
    input: Value,
    valid_payload: bool,
    observed_at: chrono::DateTime<chrono::Utc>,
}

impl RedriveCheck {
    fn validate(&self) -> Result<()> {
        let run = &self.run.0;
        if !self.admitted || WorkBudget::from_selection(&run.selection_snapshot).is_err() {
            return Err(Error::Stale);
        }
        if crate::indexing::deadline(run)?.is_some_and(|deadline| deadline <= self.observed_at) {
            return Err(Error::Invalid("indexing deadline expired"));
        }
        if !self.current_run {
            return Err(Error::Stale);
        }
        match self.kind.as_str() {
            "artwork" => {
                serde_json::from_value::<crate::media::ArtworkJob>(self.input.clone())
                    .map_err(|_| Error::Invalid("artwork redrive payload"))?;
            }
            "embedding" | "ingest" => {}
            _ => return Err(Error::Invalid("redrive job kind")),
        }
        if !self.valid_payload {
            return Err(Error::Stale);
        }
        Ok(())
    }
}

async fn redrive_checks(connection: &mut PgConnection, jobs: &[Uuid]) -> Result<Vec<RedriveCheck>> {
    Ok(sqlx::query_as(r#"
        SELECT j.id,to_jsonb(r) AS run,clock_timestamp() AS observed_at,j.kind,j.input,
            ir.run_id IS NOT NULL AS admitted,
            COALESCE(p.enabled AND p.version=r.policy_version AND s.state='enabled'
                AND s.configuration=ir.source_configuration AND j.state='failed'
                AND r.state IN ('running','complete','failed') AND (r.state<>'failed' OR j.kind='ingest')
                AND NOT EXISTS(SELECT FROM ingest.runs newer WHERE newer.source_id=r.source_id
                    AND newer.id<>r.id AND (newer.started_at>r.started_at OR newer.state IN ('running','paused'))),
                false) AS current_run,
            CASE j.kind WHEN 'ingest' THEN true WHEN 'embedding' THEN
                j.input->'model_id'=COALESCE(to_jsonb(r.embedding_model_id),'null'::jsonb)
                AND EXISTS(SELECT FROM search.documents d JOIN catalog.source_items si ON si.id=d.source_item_id
                    JOIN search.embedding_models m ON m.id=r.embedding_model_id
                    WHERE si.source_id=r.source_id AND si.external_id=j.input->>'external_id'
                    AND si.state='enabled' AND m.state<>'retired' AND encode(d.content_hash,'hex')=j.input->>'input_hash')
            WHEN 'artwork' THEN EXISTS(SELECT FROM media.artwork a
                JOIN media.artwork_generations g ON g.artwork_id=a.id AND g.revision=a.current_generation
                WHERE to_jsonb(a.id)=j.input->'artwork_id' AND a.source_id=r.source_id
                AND (a.state='enabled' OR (a.state='absent' AND a.revision IS NULL AND g.state='pending'))
                AND to_jsonb(g.revision)=j.input->'revision' AND g.state IN ('pending','published')
                AND to_jsonb(ARRAY(SELECT get_byte(g.fingerprint,n) FROM generate_series(0,31) n))=j.input->'fingerprint')
            ELSE false END IS TRUE AS valid_payload
        FROM queue.jobs j JOIN ingest.runs r ON r.id=j.scope_id JOIN ingest.policies p ON p.id=r.policy_id
        JOIN catalog.sources s ON s.id=r.source_id LEFT JOIN ingest.indexing_runs ir ON ir.run_id=r.id
        WHERE j.queue_name='indexing' AND j.id=ANY($1)
        "#).bind(jobs).fetch_all(connection).await?)
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
        Ok(self
            .redrive_eligibilities(&[job])
            .await?
            .remove(&job)
            .unwrap())
    }
    pub async fn redrive_eligibilities(
        &self,
        jobs: &[Uuid],
    ) -> Result<HashMap<Uuid, RedriveEligibility>> {
        if jobs.len() > 200 {
            return Err(Error::Invalid("page"));
        }
        if jobs.is_empty() {
            return Ok(HashMap::new());
        }
        let mut connection = self.pool.acquire().await?;
        let checks: HashMap<_, _> = redrive_checks(&mut connection, jobs)
            .await?
            .into_iter()
            .map(|check| (check.id, check))
            .collect();
        jobs.iter()
            .map(|id| {
                let reason = match checks
                    .get(id)
                    .ok_or(Error::NotFound)
                    .and_then(RedriveCheck::validate)
                {
                    Ok(()) => None,
                    Err(Error::Stale | Error::NotFound) => Some("not_current_terminal_job".into()),
                    Err(Error::Invalid("indexing deadline expired")) => {
                        Some("deadline_expired".into())
                    }
                    Err(Error::Invalid(_)) => Some("unsupported_payload".into()),
                    Err(error) => return Err(error),
                };
                Ok((
                    *id,
                    RedriveEligibility {
                        allowed: reason.is_none(),
                        reason,
                    },
                ))
            })
            .collect()
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
        let mut tx = self.pool.begin().await?;
        if let Some(receipt) = queue::redrive_receipt(&mut tx, job, request).await? {
            tx.commit().await?;
            return Ok(receipt);
        }
        let policy_id: Uuid=sqlx::query_scalar("SELECT r.policy_id FROM queue.jobs j JOIN ingest.runs r ON r.id=j.scope_id WHERE j.id=$1 AND j.queue_name='indexing'").bind(job).fetch_optional(&mut *tx).await?.ok_or(Error::NotFound)?;
        lock_policy_source(&mut tx, policy_id).await?;
        sqlx::query("SELECT id FROM ingest.policies WHERE id=$1 FOR UPDATE")
            .bind(policy_id)
            .fetch_one(&mut *tx)
            .await?;
        sqlx::query("SELECT r.id FROM ingest.runs r JOIN queue.jobs j ON j.scope_id=r.id WHERE j.id=$1 FOR UPDATE OF r")
            .bind(job).fetch_one(&mut *tx).await?;
        sqlx::query("SELECT id FROM queue.jobs WHERE id=$1 FOR UPDATE")
            .bind(job)
            .fetch_one(&mut *tx)
            .await?;
        let check = redrive_checks(&mut tx, &[job])
            .await?
            .pop()
            .ok_or(Error::NotFound)?;
        check.validate()?;
        let run = check.run.0;
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
        Ok(result)
    }
}
