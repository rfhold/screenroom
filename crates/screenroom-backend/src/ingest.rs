use crate::{
    Error, Result,
    catalog::{ItemWrite, write_item},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Policy {
    pub id: Uuid,
    pub source_id: Uuid,
    pub version: i64,
    pub enabled: bool,
    pub automatic_runs: bool,
    pub selection_json: Value,
    pub max_items: i64,
    pub max_requests: i64,
    pub incremental_seconds: i32,
    pub reconcile_seconds: i32,
    pub enrich_metadata: bool,
    pub enrich_people: bool,
    pub enrich_artwork: bool,
    pub next_due_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Run {
    pub source_id: Uuid,
    pub id: Uuid,
    pub policy_id: Uuid,
    pub policy_version: i64,
    pub policy_snapshot: Value,
    pub selection_snapshot: Value,
    pub state: String,
    pub items: i64,
    pub requests: i64,
    pub budget_items: i64,
    pub budget_requests: i64,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub coverage: Value,
    pub embedding_model_id: Option<Uuid>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobKind {
    Ingest,
    Embedding,
    Artwork,
}
impl JobKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ingest => "ingest",
            Self::Embedding => "embedding",
            Self::Artwork => "artwork",
        }
    }
}
#[derive(Debug, Clone, FromRow)]
pub struct Claim {
    pub id: Uuid,
    pub run_id: Option<Uuid>,
    pub kind: String,
    pub input: Value,
    pub lease_owner: String,
    pub fence: i64,
    pub attempts: i32,
    pub lease_until: DateTime<Utc>,
}
impl Claim {
    fn transport(&self) -> crate::queue::Claim {
        crate::queue::Claim {
            id: self.id,
            scope_id: self.run_id,
            kind: self.kind.clone(),
            input: self.input.clone(),
            lease_owner: self.lease_owner.clone(),
            fence: self.fence,
            attempts: self.attempts,
            lease_until: self.lease_until,
        }
    }
}
impl From<crate::queue::Claim> for Claim {
    fn from(c: crate::queue::Claim) -> Self {
        Self {
            id: c.id,
            run_id: c.scope_id,
            kind: c.kind,
            input: c.input,
            lease_owner: c.lease_owner,
            fence: c.fence,
            attempts: c.attempts,
            lease_until: c.lease_until,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Followup {
    pub kind: JobKind,
    pub dedupe_key: String,
    pub input: Value,
}
#[derive(Debug, Clone)]
pub struct LibraryWrite {
    pub public_id: String,
    pub external_id: String,
    pub title: String,
}
#[derive(Debug, Clone)]
pub struct CheckpointWrite {
    pub scope_key: String,
    pub cursor: Value,
    pub watermark: Option<String>,
}
#[derive(Debug, Clone)]
pub enum Completion {
    Continue,
    Pause,
    Complete { evidence: Value },
}
#[derive(Debug, Clone)]
pub struct PageCommit {
    pub libraries: Vec<LibraryWrite>,
    pub items: Vec<ItemWrite>,
    pub artwork_inputs: Vec<crate::media::ArtworkInputWrite>,
    pub artwork_omissions: Vec<crate::media::ArtworkOmission>,
    pub followups: Vec<Followup>,
    pub checkpoint: CheckpointWrite,
    pub requests: i64,
    pub completion: Completion,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum WorkBudget {
    PerWorkCommit {
        max_requests: i64,
        max_records: usize,
    },
}
pub(crate) const EXECUTION_BUDGET_SQL: &str = r#"CASE WHEN
  jsonb_typeof(r.selection_snapshot->'work_budget')='object'
  AND r.selection_snapshot->'work_budget' = jsonb_build_object(
    'mode','per_work_commit','max_requests',r.selection_snapshot->'work_budget'->'max_requests',
    'max_records',r.selection_snapshot->'work_budget'->'max_records')
  AND jsonb_typeof(r.selection_snapshot->'work_budget'->'max_requests')='number'
  AND jsonb_typeof(r.selection_snapshot->'work_budget'->'max_records')='number'
  AND r.selection_snapshot->'work_budget'->>'max_requests' ~ '^[0-9]+$'
  AND r.selection_snapshot->'work_budget'->>'max_records' ~ '^[0-9]+$'
  THEN (r.selection_snapshot->'work_budget'->>'max_requests')::numeric BETWEEN 1 AND 100000
   AND (r.selection_snapshot->'work_budget'->>'max_records')::numeric BETWEEN 1 AND 1000
  ELSE false END"#;
impl WorkBudget {
    pub fn from_selection(selection: &Value) -> Result<Self> {
        let budget: Self = selection
            .get("work_budget")
            .cloned()
            .ok_or(Error::Invalid("work budget required"))
            .and_then(|value| {
                serde_json::from_value(value).map_err(|_| Error::Invalid("work budget"))
            })?;
        let Self::PerWorkCommit {
            max_requests,
            max_records,
        } = budget;
        if !(1..=100_000).contains(&max_requests) || !(1..=1000).contains(&max_records) {
            return Err(Error::Invalid("work budget bounds"));
        }
        Ok(budget)
    }
    fn execution_limits(selection: &Value) -> Result<(i64, usize)> {
        match Self::from_selection(selection)? {
            Self::PerWorkCommit {
                max_requests,
                max_records,
            } => Ok((max_requests, max_records)),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationClass {
    Discovery,
    Page,
    Ancestor,
    Collection,
    Artwork,
    Embedding,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestOutcome {
    Success,
    Skipped,
    Retryable,
    Permanent,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestErrorClass {
    Throttled,
    Transport,
    Timeout,
    Unavailable,
    Invalid,
    Unauthorized,
}
#[derive(Debug, Clone, FromRow)]
pub struct RequestReceipt {
    pub id: Uuid,
    pub run_id: Uuid,
    pub job_id: Uuid,
    pub fence: i64,
}
#[derive(Debug, Clone)]
pub struct RequestOutcomeWrite {
    pub receipt_id: Uuid,
    pub outcome: RequestOutcome,
    pub error_class: Option<RequestErrorClass>,
}
#[derive(Debug, Clone)]
pub struct LibraryTouch {
    pub external_id: String,
    pub library_external_id: String,
}
#[derive(Debug, Clone, Default)]
pub struct TraversalWrites {
    pub primary_external_ids: Vec<String>,
    pub library_touches: Vec<LibraryTouch>,
    pub outcomes: Vec<RequestOutcomeWrite>,
    pub collection_members: Vec<crate::catalog::CollectionMembershipWrite>,
}
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct RunCoverage {
    #[sqlx(default)]
    pub unresolved_attempts: i64,
    pub primary_unique: i64,
    pub followup_ready: i64,
    pub followup_delayed: i64,
    pub followup_leased: i64,
    pub followup_failed: i64,
    pub unknown_attempts: i64,
    pub retryable_attempts: i64,
    pub permanent_attempts: i64,
    pub omissions: i64,
    pub unresolved_operations: i64,
    pub pending_artwork_publications: i64,
}
#[derive(Debug, Clone, FromRow)]
pub struct PendingOperation {
    pub id: Uuid,
    pub operation: String,
    pub operation_key: String,
}
#[derive(Debug, Clone, FromRow)]
pub struct ObservedItem {
    pub external_id: String,
    pub public_id: String,
    pub kind: String,
}

#[derive(Clone)]
pub struct IngestRepository {
    pub(crate) pool: PgPool,
}
impl IngestRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    pub async fn artwork_omissions(
        &self,
        run: Uuid,
        after: Option<&str>,
        limit: u32,
    ) -> Result<Vec<crate::media::ArtworkOmission>> {
        if !(1..=1000).contains(&limit) || after.is_some_and(|id| id.len() > 200) {
            return Err(Error::Invalid("artwork omission page"));
        }
        let rows: Vec<(String, String)> = sqlx::query_as("SELECT source_artwork_id,code FROM ingest.artwork_omissions WHERE run_id=$1 AND ($2::text IS NULL OR source_artwork_id>$2) ORDER BY source_artwork_id LIMIT $3")
            .bind(run).bind(after).bind(i64::from(limit)).fetch_all(&self.pool).await?;
        rows.into_iter()
            .map(|(source_artwork_id, code)| {
                Ok(crate::media::ArtworkOmission {
                    source_artwork_id,
                    code: serde_json::from_value(Value::String(code))
                        .map_err(|_| Error::Invalid("artwork omission code"))?,
                })
            })
            .collect()
    }
    pub async fn policy(&self, id: Uuid) -> Result<Option<Policy>> {
        Ok(sqlx::query_as("SELECT * FROM ingest.policies WHERE id=$1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?)
    }
    pub async fn policies(&self, source: Option<Uuid>) -> Result<Vec<Policy>> {
        Ok(sqlx::query_as("SELECT * FROM ingest.policies WHERE ($1::uuid IS NULL OR source_id=$1) ORDER BY id LIMIT 1000").bind(source).fetch_all(&self.pool).await?)
    }
    pub async fn due_policies(&self, limit: u32) -> Result<Vec<Policy>> {
        self.due_policies_after(limit, None).await
    }
    pub async fn due_policies_after(
        &self,
        limit: u32,
        after: Option<(DateTime<Utc>, Uuid)>,
    ) -> Result<Vec<Policy>> {
        if !(1..=100).contains(&limit) {
            return Err(Error::Invalid("limit"));
        }
        Ok(sqlx::query_as("SELECT p.* FROM ingest.policies p JOIN catalog.sources s ON s.id=p.source_id WHERE p.enabled AND p.automatic_runs AND s.state='enabled' AND ($2::timestamptz IS NULL OR (p.next_due_at,p.id)>($2,$3)) AND p.next_due_at<=now() AND NOT EXISTS(SELECT FROM ingest.runs r WHERE r.source_id=p.source_id AND (r.state IN ('running','paused') OR EXISTS(SELECT FROM queue.jobs j WHERE j.scope_id=r.id AND j.state IN ('ready','leased')))) ORDER BY p.next_due_at,p.id LIMIT $1").bind(i64::from(limit)).bind(after.map(|cursor|cursor.0)).bind(after.map(|cursor|cursor.1)).fetch_all(&self.pool).await?)
    }
    pub async fn defer_invalid_automatic_policy(&self, policy: &Policy) -> Result<()> {
        sqlx::query("UPDATE ingest.policies SET next_due_at=clock_timestamp()+make_interval(secs=>incremental_seconds) WHERE id=$1 AND version=$2 AND enabled AND automatic_runs AND next_due_at=$3 AND next_due_at<=clock_timestamp()")
            .bind(policy.id).bind(policy.version).bind(policy.next_due_at).execute(&self.pool).await?;
        Ok(())
    }
    pub async fn run(&self, id: Uuid) -> Result<Option<Run>> {
        Ok(sqlx::query_as("SELECT * FROM ingest.runs WHERE id=$1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?)
    }
    pub async fn cancel_run(&self, id: Uuid) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let policy: Uuid = sqlx::query_scalar("SELECT policy_id FROM ingest.runs WHERE id=$1")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error::NotFound)?;
        lock_policy_source(&mut tx, policy).await?;
        let changed = sqlx::query("UPDATE ingest.runs SET state='cancelled',finished_at=now() WHERE id=$1 AND state IN ('running','paused')").bind(id).execute(&mut *tx).await?.rows_affected();
        if changed != 1 {
            return Err(Error::Stale);
        }
        crate::queue::cancel_scope(&mut tx, "indexing", id, false).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn checkpoint(
        &self,
        policy_id: Uuid,
        scope: &str,
        version: i64,
    ) -> Result<Option<Value>> {
        Ok(sqlx::query_scalar("SELECT c.cursor FROM ingest.checkpoints c JOIN ingest.policies p ON p.source_id=c.source_id WHERE p.id=$1 AND c.scope_key=$2 AND c.policy_version=$3").bind(policy_id).bind(checkpoint_scope(policy_id,scope)).bind(version).fetch_optional(&self.pool).await?)
    }
    pub async fn run_checkpoint(&self, run: &Run, scope: &str) -> Result<Option<Value>> {
        #[derive(Default, Deserialize)]
        #[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
        enum HistoricalWorkBudget {
            #[default]
            Lifetime,
            PerWorkCommit {
                max_requests: i64,
                max_records: usize,
            },
        }
        let budget = run
            .selection_snapshot
            .get("work_budget")
            .cloned()
            .map(serde_json::from_value::<HistoricalWorkBudget>)
            .transpose()
            .map_err(|_| Error::Invalid("historical work budget"))?
            .unwrap_or_default();
        if let HistoricalWorkBudget::PerWorkCommit {
            max_requests,
            max_records,
        } = budget
        {
            if !(1..=100_000).contains(&max_requests) || !(1..=1000).contains(&max_records) {
                return Err(Error::Invalid("historical work budget bounds"));
            }
            return Ok(sqlx::query_scalar("SELECT cursor FROM ingest.checkpoints WHERE source_id=$1 AND scope_key=$2 AND policy_version=$3 AND observed_run_id=$4")
                .bind(run.source_id).bind(checkpoint_scope(run.policy_id, scope)).bind(run.policy_version).bind(run.id)
                .fetch_optional(&self.pool).await?);
        }
        if run.requests == 0 {
            return Ok(None);
        }
        self.checkpoint(run.policy_id, scope, run.policy_version)
            .await
    }
    pub async fn claim_canonical(
        &self,
        owner: &str,
        lease_seconds: i32,
        capabilities: &[String],
    ) -> Result<Option<Claim>> {
        self.expire_deadlines().await?;
        self.retire_exhausted().await?;
        let mut tx = self.pool.begin().await?;
        let locked:Vec<Uuid>=sqlx::query_scalar("SELECT s.id FROM catalog.sources s WHERE s.state='enabled' AND EXISTS(SELECT FROM ingest.runs r JOIN ingest.indexing_runs ir ON ir.run_id=r.id WHERE r.source_id=s.id AND r.state IN ('running','complete')) ORDER BY s.id FOR UPDATE OF s SKIP LOCKED").fetch_all(&mut *tx).await?;
        let scopes: Vec<Uuid> = sqlx::query_scalar("SELECT r.id FROM ingest.runs r JOIN ingest.policies p ON p.id=r.policy_id JOIN catalog.sources s ON s.id=r.source_id WHERE p.enabled AND p.version=r.policy_version AND s.state='enabled' AND r.state IN ('running','complete') AND (NOT EXISTS(SELECT FROM ingest.indexing_runs ir WHERE ir.run_id=r.id) OR s.id=ANY($1)) AND ingest.indexing_source_available(r.id) AND NOT EXISTS(SELECT FROM ingest.indexing_runs ir WHERE ir.run_id=r.id AND ir.source_configuration<>s.configuration) AND NOT EXISTS(SELECT FROM ingest.source_cooldowns sc WHERE sc.source_id=s.id AND sc.until_at>clock_timestamp())").bind(locked).fetch_all(&mut *tx).await?;
        let scopes: Vec<Uuid> = sqlx::query_scalar(
            &format!("SELECT r.id FROM ingest.runs r WHERE r.id=ANY($1) AND ({EXECUTION_BUDGET_SQL}) AND ingest.indexing_deadline_valid(r.id) AND EXISTS(SELECT FROM ingest.indexing_runs x WHERE x.run_id=r.id)"),
        )
        .bind(&scopes)
        .fetch_all(&mut *tx)
        .await?;
        let claim = crate::queue::claim(
            &mut tx,
            "indexing",
            &scopes,
            capabilities,
            owner,
            lease_seconds,
        )
        .await?;
        tx.commit().await?;
        Ok(claim.map(Into::into))
    }
    pub async fn expire_deadlines(&self) -> Result<()> {
        const STRANDED: &str = "r.state='failed' AND EXISTS(SELECT FROM queue.jobs j WHERE j.scope_id=r.id AND j.queue_name='indexing' AND j.state IN ('ready','leased')) AND (NOT ingest.indexing_deadline_valid(r.id) OR EXISTS(SELECT FROM ingest.runs newer WHERE newer.source_id=r.source_id AND newer.id<>r.id AND (newer.started_at>r.started_at OR newer.state IN ('running','paused'))))";
        let eligible = format!(
            "({EXECUTION_BUDGET_SQL}) AND (({STRANDED}) OR (r.state IN ('running','paused','complete') AND (r.state<>'complete' OR ir.candidate_watermark IS NOT NULL OR EXISTS(SELECT FROM queue.jobs j WHERE j.scope_id=r.id AND j.state IN ('ready','leased'))) AND ir.settled_at IS NULL AND NOT ingest.indexing_deadline_valid(r.id)))"
        );
        let runs: Vec<Run> = sqlx::query_as(&format!("SELECT r.* FROM ingest.runs r JOIN ingest.indexing_runs ir ON ir.run_id=r.id WHERE {eligible} ORDER BY r.source_id,r.policy_id,r.id LIMIT 32")).fetch_all(&self.pool).await?;
        for run in runs {
            let mut tx = self.pool.begin().await?;
            lock_policy_source(&mut tx, run.policy_id).await?;
            sqlx::query("SELECT id FROM ingest.policies WHERE id=$1 FOR UPDATE")
                .bind(run.policy_id)
                .execute(&mut *tx)
                .await?;
            let run: Run = sqlx::query_as("SELECT * FROM ingest.runs WHERE id=$1 FOR UPDATE")
                .bind(run.id)
                .fetch_one(&mut *tx)
                .await?;
            let expired: bool = sqlx::query_scalar(&format!("SELECT EXISTS(SELECT FROM ingest.runs r JOIN ingest.indexing_runs ir ON ir.run_id=r.id WHERE r.id=$1 AND {eligible})"))
                .bind(run.id).fetch_one(&mut *tx).await?;
            if expired {
                crate::queue::cancel_scope(&mut tx, "indexing", run.id, false).await?;
                if run.state != "failed" {
                    sqlx::query("UPDATE ingest.runs SET state='failed',finished_at=clock_timestamp(),coverage=coverage||'{\"terminal_reason\":\"deadline_expired\"}'::jsonb WHERE id=$1").bind(run.id).execute(&mut *tx).await?;
                }
            }
            tx.commit().await?;
        }
        Ok(())
    }
    async fn retire_exhausted(&self) -> Result<()> {
        // Bound maintenance per claim; terminal failed runs also repair pre-existing dedupe blockers.
        let candidates: Vec<(Uuid, Uuid, Uuid)> = sqlx::query_as(&format!("SELECT r.source_id,r.policy_id,r.id FROM ingest.runs r WHERE ({EXECUTION_BUDGET_SQL}) AND EXISTS(SELECT FROM ingest.indexing_runs ir WHERE ir.run_id=r.id) AND r.state IN ('running','complete','failed') AND EXISTS(SELECT FROM queue.jobs j WHERE j.scope_id=r.id AND ((j.state IN ('ready','leased') AND (j.state='ready' OR j.lease_until<clock_timestamp()) AND j.attempts>=j.max_attempts) OR (r.state='failed' AND j.state='leased') OR (r.state='running' AND j.kind='ingest' AND j.state='failed'))) ORDER BY r.source_id,r.policy_id,r.id LIMIT 32"))
            .fetch_all(&self.pool).await?;
        for (source, policy, run) in candidates {
            let mut tx = self.pool.begin().await?;
            sqlx::query("SELECT id FROM catalog.sources WHERE id=$1 FOR SHARE")
                .bind(source)
                .execute(&mut *tx)
                .await?;
            sqlx::query("SELECT id FROM ingest.policies WHERE id=$1 FOR UPDATE")
                .bind(policy)
                .execute(&mut *tx)
                .await?;
            sqlx::query("SELECT id FROM ingest.runs WHERE id=$1 FOR UPDATE")
                .bind(run)
                .execute(&mut *tx)
                .await?;
            let admitted: bool = sqlx::query_scalar(&format!("SELECT ({EXECUTION_BUDGET_SQL}) AND EXISTS(SELECT FROM ingest.indexing_runs ir WHERE ir.run_id=r.id) FROM ingest.runs r WHERE r.id=$1"))
                .bind(run).fetch_one(&mut *tx).await?;
            if !admitted {
                tx.rollback().await?;
                continue;
            }
            crate::queue::retire_exhausted(&mut tx, "indexing", run).await?;
            fail_exhausted_run(&mut tx, run).await?;
            tx.commit().await?;
        }
        Ok(())
    }
    pub async fn renew(&self, c: &Claim, seconds: i32) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        guard(&mut tx, c).await?;
        crate::queue::renew(&mut tx, &c.transport(), seconds).await?;
        tx.commit().await?;
        Ok(())
    }
    /// Reserves one attempt before I/O. Lease changes and resume never refund it.
    pub async fn reserve_work_request(
        &self,
        c: &Claim,
        operation: OperationClass,
    ) -> Result<RequestReceipt> {
        let mut tx = self.pool.begin().await?;
        let (run, _) = guard(&mut tx, c).await?;
        if let Some(config) = crate::indexing::configuration(&run)?
            && (run.requests >= config.limits.lifetime_requests
                || sqlx::query_scalar::<_, bool>("SELECT clock_timestamp()>=$1")
                    .bind(crate::indexing::deadline(&run)?)
                    .fetch_one(&mut *tx)
                    .await?)
        {
            return Err(Error::Invalid(
                "indexing lifetime request or deadline exhausted",
            ));
        }
        let (max_requests, _) = WorkBudget::execution_limits(&run.selection_snapshot)?;
        let cooling: bool = sqlx::query_scalar(
            "SELECT until_at>clock_timestamp() FROM ingest.source_cooldowns WHERE source_id=$1",
        )
        .bind(run.source_id)
        .fetch_one(&mut *tx)
        .await?;
        if cooling {
            return Err(Error::Invalid("source cooldown"));
        }
        let n = sqlx::query("UPDATE queue.jobs SET window_requests=window_requests+1 WHERE id=$1 AND window_requests<$2")
            .bind(c.id).bind(max_requests).execute(&mut *tx).await?.rows_affected();
        if n != 1 {
            return Err(Error::Invalid("work request window exhausted"));
        }
        if run.requests == i64::MAX {
            return Err(Error::Invalid("request counter exhausted"));
        }
        sqlx::query("UPDATE ingest.runs SET requests=requests+1 WHERE id=$1")
            .bind(run.id)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO ingest.request_work_units(job_id) VALUES($1) ON CONFLICT DO NOTHING",
        )
        .bind(c.id)
        .execute(&mut *tx)
        .await?;
        let receipt = sqlx::query_as("INSERT INTO ingest.request_receipts(run_id,job_id,fence,operation,work_unit) SELECT $1,$2,$3,$4,unit FROM ingest.request_work_units WHERE job_id=$2 RETURNING id,run_id,job_id,fence")
            .bind(run.id).bind(c.id).bind(c.fence).bind(safe_enum(operation)?).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok(receipt)
    }
    /// Records a sanitized outcome without replenishing the work window.
    pub async fn record_work_outcome(
        &self,
        c: &Claim,
        outcome: &RequestOutcomeWrite,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        guard(&mut tx, c).await?;
        write_outcome(&mut tx, c, outcome).await?;
        tx.commit().await?;
        Ok(())
    }
    /// Persists a source-wide cooldown and retries this job without refunding attempts.
    pub async fn defer_work(
        &self,
        c: &Claim,
        outcome: &RequestOutcomeWrite,
        delay_seconds: i32,
    ) -> Result<()> {
        if !(1..=86400).contains(&delay_seconds)
            || !matches!(outcome.outcome, RequestOutcome::Retryable)
            || !matches!(outcome.error_class, Some(RequestErrorClass::Throttled))
        {
            return Err(Error::Invalid("source cooldown"));
        }
        let mut tx = self.pool.begin().await?;
        let (run, _) = guard(&mut tx, c).await?;
        write_outcome(&mut tx, c, outcome).await?;
        sqlx::query("UPDATE ingest.source_cooldowns SET until_at=GREATEST(until_at,clock_timestamp()+make_interval(secs=>$2)) WHERE source_id=$1")
            .bind(run.source_id).bind(delay_seconds).execute(&mut *tx).await?;
        crate::queue::fail(
            &mut tx,
            &c.transport(),
            crate::queue::FailureClass::Throttled,
            "throttled",
            Some(delay_seconds),
        )
        .await?;
        if c.kind == "ingest" {
            fail_exhausted_run(&mut tx, run.id).await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn run_coverage(&self, run_id: Uuid) -> Result<RunCoverage> {
        let mut tx = self.pool.begin().await?;
        let result = coverage_in(&mut tx, run_id).await?;
        tx.commit().await?;
        Ok(result)
    }
    /// Records an unresolved dependency, not an HTTP attempt or a catalog item.
    pub async fn record_unresolved_operation(
        &self,
        c: &Claim,
        operation: OperationClass,
        key: &str,
    ) -> Result<Uuid> {
        if key.is_empty()
            || key.len() > 400
            || !key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_:-.,".contains(&b))
        {
            return Err(Error::Invalid("operation key"));
        }
        let mut tx = self.pool.begin().await?;
        let (run, _) = guard(&mut tx, c).await?;
        let id = sqlx::query_scalar("INSERT INTO ingest.request_receipts(run_id,job_id,fence,operation,outcome,is_request,operation_key) VALUES($1,$2,$3,$4,'unresolved',false,$5) ON CONFLICT(run_id,operation,operation_key) WHERE operation_key IS NOT NULL DO UPDATE SET outcome='unresolved' RETURNING id")
            .bind(run.id).bind(c.id).bind(c.fence).bind(safe_enum(operation)?).bind(key).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok(id)
    }
    /// Run-observed rows, including collection containers, without an in-memory frontier.
    pub async fn observed_items(
        &self,
        run_id: Uuid,
        kind: &str,
        after: Option<&str>,
        limit: u32,
    ) -> Result<Vec<ObservedItem>> {
        if !matches!(
            kind,
            "movie" | "series" | "season" | "episode" | "collection"
        ) || !(1..=200).contains(&limit)
            || after.is_some_and(|id| id.len() > 200)
        {
            return Err(Error::Invalid("observed item page"));
        }
        let run = self.run(run_id).await?.ok_or(Error::Stale)?;
        let selected = selected_library_ids(&run.selection_snapshot)?;
        Ok(sqlx::query_as("SELECT si.external_id,i.public_id,i.kind FROM catalog.source_items si JOIN catalog.items i ON i.id=si.item_id JOIN catalog.sources s ON s.id=si.source_id WHERE si.source_id=$1 AND si.state='enabled' AND s.state='enabled' AND i.kind=$2 AND ($3::text IS NULL OR si.external_id>$3) AND EXISTS(SELECT FROM ingest.item_observations o JOIN catalog.libraries l ON l.source_id=o.source_id AND l.external_id=o.library_external_id WHERE o.source_id=si.source_id AND o.external_id=si.external_id AND o.run_id=$4 AND l.state='enabled' AND l.public_id=ANY($5)) ORDER BY si.external_id LIMIT $6")
            .bind(run.source_id).bind(kind).bind(after).bind(run.id).bind(selected).bind(i64::from(limit)).fetch_all(&self.pool).await?)
    }
    pub async fn pending_operations(
        &self,
        run_id: Uuid,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<PendingOperation>> {
        if !(1..=200).contains(&limit) {
            return Err(Error::Invalid("pending operation page"));
        }
        Ok(sqlx::query_as("SELECT id,operation,operation_key FROM ingest.request_receipts WHERE run_id=$1 AND outcome='unresolved' AND NOT is_request AND ($2::uuid IS NULL OR id>$2) ORDER BY id LIMIT $3")
            .bind(run_id).bind(after).bind(i64::from(limit)).fetch_all(&self.pool).await?)
    }
    /// Resolves a persisted dependency only after the caller validates its materialization.
    pub async fn resolve_operation(&self, c: &Claim, id: Uuid) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let (run, _) = guard(&mut tx, c).await?;
        let n = sqlx::query("UPDATE ingest.request_receipts SET outcome='success' WHERE id=$1 AND run_id=$2 AND outcome='unresolved' AND NOT is_request")
            .bind(id).bind(run.id).execute(&mut *tx).await?.rows_affected();
        if n != 1 {
            return Err(Error::Stale);
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn fail(
        &self,
        c: &Claim,
        class: crate::queue::FailureClass,
        error_code: &str,
        retry_after: Option<i32>,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let _ = guard(&mut tx, c).await?;
        crate::queue::fail(&mut tx, &c.transport(), class, error_code, retry_after).await?;
        if c.kind == "ingest" {
            fail_exhausted_run(&mut tx, c.run_id.ok_or(Error::Stale)?).await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn resume(&self, run_id: Uuid) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let policy_id: Uuid = sqlx::query_scalar("SELECT policy_id FROM ingest.runs WHERE id=$1")
            .bind(run_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error::NotFound)?;
        lock_policy_source(&mut tx, policy_id).await?;
        sqlx::query("SELECT id FROM ingest.policies WHERE id=$1 FOR UPDATE")
            .bind(policy_id)
            .execute(&mut *tx)
            .await?;
        let n=sqlx::query("UPDATE ingest.runs r SET state='running' FROM ingest.policies p,catalog.sources s WHERE r.id=$1 AND r.policy_id=p.id AND p.source_id=s.id AND p.version=r.policy_version AND p.enabled AND s.state='enabled' AND r.state='paused' AND r.selection_snapshot->'work_budget'->>'mode'='per_work_commit' AND EXISTS(SELECT FROM ingest.indexing_runs ir WHERE ir.run_id=r.id)")
            .bind(run_id).execute(&mut *tx).await?.rows_affected();
        if n != 1 {
            return Err(Error::Stale);
        }
        enqueue(
            &mut tx,
            run_id,
            &Followup {
                kind: JobKind::Ingest,
                dedupe_key: format!("run:{run_id}"),
                input: json!({}),
            },
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn pause(&self, run_id: Uuid) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let policy: Uuid = sqlx::query_scalar("SELECT policy_id FROM ingest.runs WHERE id=$1")
            .bind(run_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error::NotFound)?;
        lock_policy_source(&mut tx, policy).await?;
        let n =
            sqlx::query("UPDATE ingest.runs r SET state='paused' WHERE id=$1 AND state='running' AND selection_snapshot->'work_budget'->>'mode'='per_work_commit' AND EXISTS(SELECT FROM ingest.indexing_runs ir WHERE ir.run_id=r.id)")
                .bind(run_id)
                .execute(&mut *tx)
                .await?
                .rows_affected();
        if n != 1 {
            return Err(Error::Stale);
        }
        crate::queue::cancel_scope(&mut tx, "indexing", run_id, true).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn commit_traversal_page(
        &self,
        c: &Claim,
        page: &PageCommit,
        writes: &TraversalWrites,
    ) -> Result<()> {
        self.commit_page_inner(c, page, Some(writes), None, None)
            .await
    }
    pub async fn commit_repair_page(
        &self,
        c: &Claim,
        page: &PageCommit,
        writes: &crate::repair::RepairWrites,
    ) -> Result<()> {
        self.commit_page_inner(c, page, None, Some(writes), None)
            .await
    }
    pub async fn commit_failed_traversal_page(
        &self,
        c: &Claim,
        page: &PageCommit,
        class: crate::queue::FailureClass,
        code: &str,
        retry_after: Option<i32>,
    ) -> Result<()> {
        if !matches!(page.completion, Completion::Continue) {
            return Err(Error::Invalid("failed page completion"));
        }
        self.commit_page_inner(
            c,
            page,
            Some(&TraversalWrites::default()),
            None,
            Some((class, code, retry_after)),
        )
        .await
    }
    async fn commit_page_inner(
        &self,
        c: &Claim,
        page: &PageCommit,
        traversal: Option<&TraversalWrites>,
        maintenance: Option<&crate::repair::RepairWrites>,
        failure: Option<(crate::queue::FailureClass, &str, Option<i32>)>,
    ) -> Result<()> {
        if page.items.len() > 1000
            || page.followups.len() > 2000
            || page.artwork_inputs.len() > 2000
            || page.artwork_omissions.len() > 2000
            || page.requests < 0
            || page.requests > 1000
            || page.checkpoint.scope_key.is_empty()
            || page.checkpoint.scope_key.len() > 300
            || page.checkpoint.cursor.to_string().len() > 9 * 1024 * 1024
        {
            return Err(Error::Invalid("page bounds"));
        }
        let mut manifests = std::collections::BTreeMap::new();
        for input in &page.artwork_inputs {
            let fingerprint = input.fingerprint()?;
            if manifests
                .insert(&input.reference.source_artwork_id, fingerprint)
                .is_some_and(|previous| previous != fingerprint)
            {
                return Err(Error::Invalid("conflicting artwork inputs"));
            }
        }
        let mut tx = self.pool.begin().await?;
        let (run, p) = guard(&mut tx, c).await?;
        let fallback_transition = page.checkpoint.cursor.get("fallback_reason").is_some_and(|v|!v.is_null())
            && sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT FROM ingest.indexing_runs WHERE run_id=$1 AND mode='incremental' AND fallback_reason IS NULL)").bind(run.id).fetch_one(&mut *tx).await?;
        if page.checkpoint.cursor.to_string().len() > 32768
            && !crate::indexing::configuration(&run)?
                .is_some_and(|c| c.mode == crate::indexing::Mode::Incremental)
        {
            return Err(Error::Invalid("checkpoint bounds"));
        }
        if let Some(config) = crate::indexing::configuration(&run)? {
            let mut observed: std::collections::BTreeSet<&str> = page
                .items
                .iter()
                .map(|item| item.external_id.as_str())
                .collect();
            if let Some(writes) = traversal {
                observed.extend(
                    writes
                        .library_touches
                        .iter()
                        .map(|touch| touch.external_id.as_str()),
                );
            }
            let charged = sqlx::query("UPDATE ingest.indexing_runs SET observed_records=observed_records+$2 WHERE run_id=$1 AND observed_records+$2<=$3")
                .bind(run.id).bind(observed.len() as i64).bind(config.limits.lifetime_records).execute(&mut *tx).await?.rows_affected();
            if charged != 1 {
                return Err(Error::Invalid("indexing lifetime records exhausted"));
            }
        }
        if c.kind != "ingest" || run.state != "running" {
            return Err(Error::Stale);
        }
        let (max_requests, max_records) = WorkBudget::execution_limits(&run.selection_snapshot)?;
        let is_maintenance = run.selection_snapshot.get("repair").is_some();
        if is_maintenance != maintenance.is_some() {
            return Err(Error::Invalid(
                "maintenance requires typed candidate writes",
            ));
        }
        if is_maintenance
            && (!page.items.is_empty()
                || !page.libraries.is_empty()
                || !page.followups.is_empty()
                || !page.artwork_omissions.is_empty())
        {
            return Err(Error::Invalid(
                "maintenance page cannot mutate metadata or scheduling",
            ));
        }
        if (traversal.is_none() && maintenance.is_none())
            || page.requests != 0
            || page.items.len() > max_records
        {
            return Err(Error::Invalid("full traversal page bounds"));
        }
        if matches!(page.completion, Completion::Complete { .. }) {
            let unresolved: bool = sqlx::query_scalar("SELECT EXISTS(SELECT FROM ingest.request_receipts WHERE run_id=$1 AND outcome='unresolved')")
                    .bind(run.id).fetch_one(&mut *tx).await?;
            if unresolved {
                return Err(Error::Invalid("unresolved traversal operations"));
            }
        }
        run.items
            .checked_add(page.items.len() as i64)
            .ok_or(Error::Invalid("item counter exhausted"))?;
        run.requests
            .checked_add(page.requests)
            .ok_or(Error::Invalid("request counter exhausted"))?;
        for l in &page.libraries {
            sqlx::query("INSERT INTO catalog.libraries(source_id,public_id,external_id,title) VALUES($1,$2,$3,$4) ON CONFLICT(source_id,external_id) DO UPDATE SET public_id=EXCLUDED.public_id,title=EXCLUDED.title")
                .bind(p.source_id).bind(&l.public_id).bind(&l.external_id).bind(&l.title).execute(&mut *tx).await?;
        }
        for item in &page.items {
            if run.selection_snapshot.get("library_ids").is_some() {
                let selected = selected_library_ids(&run.selection_snapshot)?;
                if item.library_external_ids.is_empty() {
                    return Err(Error::Invalid("library traversal item has no membership"));
                }
                for external in &item.library_external_ids {
                    let allowed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM catalog.libraries WHERE source_id=$1 AND external_id=$2 AND public_id=ANY($3))").bind(p.source_id).bind(external).bind(&selected).fetch_one(&mut *tx).await?;
                    if !allowed {
                        return Err(Error::Invalid("item outside selected libraries"));
                    }
                }
            }
            write_item(&mut tx, p.source_id, run.id, item).await?;
            if item.library_external_ids.len() > 100 {
                return Err(Error::Invalid("item observation bounds"));
            }
            sqlx::query("INSERT INTO ingest.item_observations(run_id,source_id,external_id,library_external_id) SELECT $1,$2,$3,unnest($4::text[]) ON CONFLICT DO NOTHING")
                    .bind(run.id).bind(p.source_id).bind(&item.external_id).bind(&item.library_external_ids).execute(&mut *tx).await?;
        }
        let mut new_omissions = 0_i64;
        for omission in &page.artwork_omissions {
            let known = sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT FROM ingest.artwork_omissions WHERE run_id=$1 AND source_artwork_id=$2)")
                    .bind(run.id).bind(&omission.source_artwork_id).fetch_one(&mut *tx).await?;
            crate::media::write_omission(&mut tx, run.id, omission).await?;
            if !known {
                new_omissions += 1;
            }
        }
        for input in &page.artwork_inputs {
            crate::media::stage_input(&mut tx, p.source_id, run.id, input).await?;
        }
        for job in &page.followups {
            if matches!(job.kind, JobKind::Artwork) {
                return Err(Error::Invalid("artwork jobs require typed inputs"));
            }
            enqueue(&mut tx, run.id, job).await?;
        }
        let mut work_progress = false;
        if let Some(writes) = maintenance {
            work_progress = crate::repair::write_candidates(&mut tx, &run, page, writes).await?;
            for outcome in &writes.outcomes {
                write_outcome(&mut tx, c, outcome).await?;
            }
            if work_progress {
                sqlx::query("UPDATE queue.jobs SET window_requests=0 WHERE id=$1")
                    .bind(c.id)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        if let Some(writes) = traversal {
            if writes.primary_external_ids.len() > max_records
                || writes.library_touches.len() > 2000
                || writes.outcomes.len() > max_requests as usize
                || writes.collection_members.len() > 2000
            {
                return Err(Error::Invalid("traversal write bounds"));
            }
            let previous = sqlx::query_scalar::<_, Value>("SELECT cursor FROM ingest.checkpoints WHERE source_id=$1 AND scope_key=$2 AND policy_version=$3 AND observed_run_id=$4")
                .bind(p.source_id).bind(checkpoint_scope(p.id, &page.checkpoint.scope_key)).bind(p.version)
                .bind(run.id)
                .fetch_optional(&mut *tx).await?;
            for external in &writes.primary_external_ids {
                if external.is_empty() || external.len() > 200 {
                    return Err(Error::Invalid("primary receipt"));
                }
            }
            for touch in &writes.library_touches {
                let selected = selected_library_ids(&run.selection_snapshot)?;
                let n = sqlx::query("INSERT INTO catalog.library_items(library_id,source_item_id,source_id,last_seen_run_id) SELECT l.id,si.id,l.source_id,$4 FROM catalog.libraries l JOIN catalog.source_items si ON si.source_id=l.source_id WHERE l.source_id=$1 AND l.external_id=$2 AND l.public_id=ANY($5) AND l.state='enabled' AND si.external_id=$3 AND si.state='enabled' ON CONFLICT(library_id,source_item_id) DO UPDATE SET last_seen_run_id=EXCLUDED.last_seen_run_id,state='enabled'")
                    .bind(p.source_id).bind(&touch.library_external_id).bind(&touch.external_id).bind(run.id).bind(selected).execute(&mut *tx).await?.rows_affected();
                if n != 1 {
                    return Err(Error::Invalid("library touch outside run selection"));
                }
                sqlx::query("INSERT INTO ingest.item_observations(run_id,source_id,external_id,library_external_id) VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING")
                    .bind(run.id).bind(p.source_id).bind(&touch.external_id).bind(&touch.library_external_id).execute(&mut *tx).await?;
            }
            let unique = writes
                .primary_external_ids
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len();
            let known: i64 = sqlx::query_scalar("SELECT count(*) FROM catalog.source_items si WHERE si.source_id=$1 AND si.external_id=ANY($2) AND si.state='enabled' AND EXISTS(SELECT FROM ingest.item_observations o WHERE o.run_id=$3 AND o.source_id=si.source_id AND o.external_id=si.external_id)")
                .bind(p.source_id).bind(&writes.primary_external_ids).bind(run.id).fetch_one(&mut *tx).await?;
            if known != unique as i64 {
                return Err(Error::Invalid("primary item outside run observations"));
            }
            let inserted = sqlx::query("INSERT INTO ingest.primary_receipts(run_id,source_id,external_id) SELECT $1,$2,external_id FROM catalog.source_items WHERE source_id=$2 AND external_id=ANY($3) ON CONFLICT DO NOTHING")
                .bind(run.id).bind(p.source_id).bind(&writes.primary_external_ids).execute(&mut *tx).await?.rows_affected();
            for outcome in &writes.outcomes {
                write_outcome(&mut tx, c, outcome).await?;
            }
            for member in &writes.collection_members {
                crate::catalog::write_collection_member(&mut tx, &run, member).await?;
            }
            let successful = writes
                .outcomes
                .iter()
                .any(|outcome| matches!(outcome.outcome, RequestOutcome::Success));
            let terminal_art_receipts = writes
                .outcomes
                .iter()
                .filter(|outcome| matches!(outcome.outcome, RequestOutcome::Permanent))
                .map(|outcome| outcome.receipt_id)
                .collect::<Vec<_>>();
            let terminal_art: i64 = if new_omissions > 0 {
                sqlx::query_scalar("SELECT count(*) FROM ingest.request_receipts WHERE id=ANY($1) AND run_id=$2 AND job_id=$3 AND fence=$4 AND operation='artwork' AND outcome='permanent' AND error_class IN ('invalid','unavailable') AND is_request")
                    .bind(terminal_art_receipts).bind(run.id).bind(c.id).bind(c.fence).fetch_one(&mut *tx).await?
            } else {
                0
            };
            let omission_progress = new_omissions > 0 && terminal_art >= new_omissions;
            let progress = omission_progress
                || successful
                    && (inserted > 0
                        || previous
                            .as_ref()
                            .is_some_and(|cursor| cursor != &page.checkpoint.cursor)
                        || (previous.is_none() && page.checkpoint.cursor != json!({})));
            work_progress = progress;
            if progress {
                sqlx::query("UPDATE queue.jobs SET window_requests=0 WHERE id=$1")
                    .bind(c.id)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        // Root types first become authoritative at discovery. Change execution mode
        // in the same commit, without adding a new cursor field for older workers.
        if page.checkpoint.scope_key == "full-index-v1"
            && !page.libraries.is_empty()
            && let Some(roots) = page
                .checkpoint
                .cursor
                .get("roots")
                .and_then(Value::as_array)
            && roots.iter().any(|r| r["kind"] == "collections")
            && roots
                .iter()
                .any(|r| matches!(r["kind"].as_str(), Some("movies" | "television")))
        {
            sqlx::query("UPDATE ingest.indexing_runs SET mode='reconcile',fallback_reason='mixed_collection_scope' WHERE run_id=$1 AND mode='incremental' AND NOT EXISTS(SELECT FROM ingest.checkpoints WHERE source_id=$2 AND scope_key=$3 AND policy_version=$4 AND observed_run_id=$1 AND jsonb_typeof(cursor->'roots')='array')")
                .bind(run.id).bind(p.source_id).bind(checkpoint_scope(p.id,&page.checkpoint.scope_key)).bind(p.version).execute(&mut *tx).await?;
        }
        if let Some(reason) = page
            .checkpoint
            .cursor
            .get("fallback_reason")
            .and_then(Value::as_str)
        {
            if !matches!(
                reason,
                "container_descendant_refresh" | "filter_rejected" | "unsupported_capability"
            ) {
                return Err(Error::Invalid("delta fallback"));
            }
            sqlx::query("UPDATE ingest.indexing_runs SET fallback_reason=$2 WHERE run_id=$1 AND mode='incremental'").bind(run.id).bind(reason).execute(&mut *tx).await?;
        }
        sqlx::query("INSERT INTO ingest.checkpoints(source_id,scope_key,policy_version,cursor,watermark,observed_run_id) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(source_id,scope_key,policy_version) DO UPDATE SET cursor=EXCLUDED.cursor,watermark=EXCLUDED.watermark,observed_run_id=EXCLUDED.observed_run_id,updated_at=now()")
            .bind(p.source_id).bind(checkpoint_scope(p.id,&page.checkpoint.scope_key)).bind(p.version).bind(&page.checkpoint.cursor).bind(&page.checkpoint.watermark).bind(run.id).execute(&mut *tx).await?;
        let (state, evidence) = match &page.completion {
            Completion::Continue => ("running", json!({})),
            Completion::Pause => ("paused", json!({"budget_paused":true})),
            Completion::Complete { evidence } => ("complete", evidence.clone()),
        };
        let evidence = if is_maintenance && state == "complete" {
            json!({"traversal_complete":true,"authoritative":false})
        } else {
            evidence
        };
        sqlx::query("UPDATE ingest.runs SET items=items+$2,state=$3,coverage=$4,finished_at=CASE WHEN $3='complete' THEN now() ELSE NULL END WHERE id=$1")
            .bind(run.id).bind(page.items.len() as i64).bind(state).bind(evidence).execute(&mut *tx).await?;
        if state == "complete" {
            sqlx::query("UPDATE ingest.policies SET next_due_at=now()+make_interval(secs=>incremental_seconds) WHERE id=$1").bind(p.id).execute(&mut *tx).await?;
        }
        if failure.is_none()
            && work_progress
            && let Some(writes) = traversal
        {
            let outcomes = &writes.outcomes;
            let fallback = fallback_transition
                && outcomes
                    .iter()
                    .any(|o| matches!(o.outcome, RequestOutcome::Permanent));
            let successful = outcomes
                .iter()
                .any(|o| matches!(o.outcome, RequestOutcome::Success))
                && !outcomes.iter().any(|o| {
                    matches!(
                        o.outcome,
                        RequestOutcome::Permanent | RequestOutcome::Retryable
                    )
                });
            if successful || fallback {
                sqlx::query("INSERT INTO ingest.request_resolutions(receipt_id,reason) SELECT rr.id,$2 FROM ingest.request_receipts rr JOIN ingest.request_work_units u ON u.job_id=rr.job_id AND u.unit=rr.work_unit WHERE rr.job_id=$1 AND (rr.outcome IN ('unknown','retryable') OR (($3 OR EXISTS(SELECT FROM queue.redrives redrive WHERE redrive.job_id=rr.job_id)) AND rr.outcome='permanent')) ON CONFLICT DO NOTHING")
                    .bind(c.id).bind(if fallback {"full_scan_fallback"} else {"successful_replay"}).bind(fallback).execute(&mut *tx).await?;
            }
            sqlx::query("UPDATE ingest.request_work_units SET unit=unit+1 WHERE job_id=$1")
                .bind(c.id)
                .execute(&mut *tx)
                .await?;
        }
        if failure.is_none()
            && let Some(writes) = maintenance
        {
            use crate::repair::Disposition;
            let terminal = writes
                .candidates
                .iter()
                .any(|c| !matches!(c.disposition, Disposition::Checked));
            let successful = terminal
                && writes.candidates.iter().all(|c| {
                    matches!(
                        c.disposition,
                        Disposition::Staged | Disposition::SkippedReady
                    )
                });
            if successful {
                sqlx::query("INSERT INTO ingest.request_resolutions(receipt_id,reason) SELECT rr.id,'successful_replay' FROM ingest.request_receipts rr JOIN ingest.request_work_units u ON u.job_id=rr.job_id AND u.unit=rr.work_unit WHERE rr.job_id=$1 AND rr.outcome IN ('unknown','retryable','permanent') ON CONFLICT DO NOTHING").bind(c.id).execute(&mut *tx).await?;
            }
            if terminal {
                sqlx::query("UPDATE ingest.request_work_units SET unit=unit+1 WHERE job_id=$1")
                    .bind(c.id)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        if let Some((class, code, retry_after)) = failure {
            crate::queue::fail(&mut tx, &c.transport(), class, code, retry_after).await?;
            fail_exhausted_run(&mut tx, run.id).await?;
        } else if state == "running" || state == "paused" {
            crate::queue::release(&mut tx, &c.transport(), work_progress).await?;
        } else {
            finish(&mut tx, c).await?;
        }
        crate::indexing::check_deadline(&mut tx, &run).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn complete_job(&self, c: &Claim) -> Result<()> {
        if c.kind == "ingest" {
            return Err(Error::Invalid(
                "ingestion must commit a page and checkpoint",
            ));
        }
        let mut tx = self.pool.begin().await?;
        guard(&mut tx, c).await?;
        finish(&mut tx, c).await?;
        tx.commit().await?;
        Ok(())
    }
}

pub(crate) async fn coverage_in(
    tx: &mut Transaction<'_, Postgres>,
    run_id: Uuid,
) -> Result<RunCoverage> {
    let pending_art = format!(
        "SELECT count(*) FROM media.artwork a JOIN catalog.sources s ON s.id=a.source_id JOIN ingest.runs observed ON observed.id=$1 AND observed.source_id=a.source_id JOIN LATERAL (SELECT staged.* FROM media.artwork_generations staged JOIN ingest.request_receipts observation ON observation.run_id=observed.id AND NOT observation.is_request AND observation.operation='artwork' AND observation.operation_key='publication:'||a.id::text||':'||staged.revision::text WHERE staged.artwork_id=a.id ORDER BY staged.revision DESC LIMIT 1) g ON true WHERE NOT(g.state='published' AND a.state='enabled' AND s.state='enabled' AND ({}))",
        crate::media::FULL_GENERATION_READY
    );
    let mut coverage: RunCoverage = sqlx::query_as(&format!("SELECT (SELECT count(*) FROM ingest.primary_receipts WHERE run_id=$1) AS primary_unique, count(*) FILTER(WHERE kind<>'ingest' AND state='ready' AND available_at<=clock_timestamp()) AS followup_ready, count(*) FILTER(WHERE kind<>'ingest' AND state='ready' AND available_at>clock_timestamp()) AS followup_delayed, count(*) FILTER(WHERE kind<>'ingest' AND state='leased') AS followup_leased, count(*) FILTER(WHERE kind<>'ingest' AND state='failed') AS followup_failed, (SELECT count(*) FROM ingest.request_receipts WHERE run_id=$1 AND outcome='unknown' AND is_request) AS unknown_attempts, (SELECT count(*) FROM ingest.request_receipts WHERE run_id=$1 AND outcome='retryable') AS retryable_attempts, (SELECT count(*) FROM ingest.request_receipts WHERE run_id=$1 AND outcome='permanent') AS permanent_attempts, (SELECT count(*) FROM ingest.artwork_omissions WHERE run_id=$1) AS omissions, (SELECT count(*) FROM ingest.request_receipts WHERE run_id=$1 AND outcome='unresolved') AS unresolved_operations, ({pending_art}) AS pending_artwork_publications FROM queue.jobs WHERE scope_id=$1"))
        .bind(run_id).fetch_one(&mut **tx).await?;
    coverage.unresolved_attempts=sqlx::query_scalar("SELECT count(*) FROM ingest.request_receipts rr WHERE rr.run_id=$1 AND ((rr.outcome='unknown' AND rr.is_request) OR rr.outcome='permanent') AND NOT EXISTS(SELECT FROM ingest.request_resolutions resolution WHERE resolution.receipt_id=rr.id)").bind(run_id).fetch_one(&mut **tx).await?;
    Ok(coverage)
}

pub fn selected_library_ids(selection: &Value) -> Result<Vec<String>> {
    let ids: Vec<String> = serde_json::from_value(
        selection
            .get("library_ids")
            .cloned()
            .ok_or(Error::Invalid("library selection"))?,
    )
    .map_err(|_| Error::Invalid("library selection"))?;
    validate_selection_keys(&ids)?;
    if ids.iter().any(|id| id == "all") {
        return Err(Error::Invalid("expand all to explicit selected libraries"));
    }
    Ok(ids)
}
fn checkpoint_scope(policy_id: Uuid, scope: &str) -> String {
    format!("policy:{policy_id}:{scope}")
}

fn safe_enum(value: impl Serialize) -> Result<String> {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .ok_or(Error::Invalid("safe operation class"))
}
pub(crate) async fn write_outcomes(
    tx: &mut Transaction<'_, Postgres>,
    c: &Claim,
    outcomes: &[RequestOutcomeWrite],
) -> Result<()> {
    if outcomes.len() > 100_000 {
        return Err(Error::Invalid("request outcome bounds"));
    }
    for outcome in outcomes {
        write_outcome(tx, c, outcome).await?;
    }
    Ok(())
}
async fn write_outcome(
    tx: &mut Transaction<'_, Postgres>,
    c: &Claim,
    outcome: &RequestOutcomeWrite,
) -> Result<()> {
    if matches!(
        outcome.outcome,
        RequestOutcome::Success | RequestOutcome::Skipped
    ) != outcome.error_class.is_none()
    {
        return Err(Error::Invalid("request outcome class"));
    }
    let n = sqlx::query("UPDATE ingest.request_receipts SET outcome=$4,error_class=$5 WHERE id=$1 AND job_id=$2 AND fence=$3 AND outcome='unknown' AND run_id=$6 AND is_request")
        .bind(outcome.receipt_id).bind(c.id).bind(c.fence).bind(safe_enum(outcome.outcome)?)
        .bind(outcome.error_class.map(safe_enum).transpose()?).bind(c.run_id).execute(&mut **tx).await?.rows_affected();
    if n != 1 {
        return Err(Error::Stale);
    }
    Ok(())
}
fn validate_selection_keys(ids: &[String]) -> Result<()> {
    let unique: std::collections::BTreeSet<_> = ids.iter().collect();
    if ids.is_empty()
        || ids.len() > 100
        || ids.len() != unique.len()
        || ids.iter().any(|s| s.is_empty() || s.len() > 200)
    {
        return Err(Error::Invalid("selection keys"));
    }
    Ok(())
}

pub(crate) async fn lock_policy_source(
    tx: &mut Transaction<'_, Postgres>,
    policy_id: Uuid,
) -> Result<()> {
    let source: Uuid = sqlx::query_scalar("SELECT source_id FROM ingest.policies WHERE id=$1")
        .bind(policy_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(Error::NotFound)?;
    sqlx::query("SELECT id FROM catalog.sources WHERE id=$1 FOR UPDATE")
        .bind(source)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub(crate) async fn start_selection_locked(
    tx: &mut Transaction<'_, Postgres>,
    p: &Policy,
    selection: &Value,
) -> Result<Run> {
    WorkBudget::execution_limits(selection)?;
    let enabled: bool =
        sqlx::query_scalar("SELECT state='enabled' FROM catalog.sources WHERE id=$1 FOR SHARE")
            .bind(p.source_id)
            .fetch_one(&mut **tx)
            .await?;
    if !p.enabled || !enabled {
        return Err(Error::Stale);
    }
    if let Some(run) = sqlx::query_as::<_, Run>(
        "SELECT * FROM ingest.runs WHERE source_id=$1 AND state IN ('running','paused')",
    )
    .bind(p.source_id)
    .fetch_optional(&mut **tx)
    .await?
    {
        if run.policy_id != p.id {
            return Err(Error::SourceBusy {
                source_id: p.source_id,
                run_id: run.id,
            });
        }
        if crate::indexing::needs_model(p)? {
            let valid: Option<Uuid>=sqlx::query_scalar("SELECT id FROM search.embedding_models WHERE id=$1 AND state IN ('building','active')").bind(run.embedding_model_id).fetch_optional(&mut **tx).await?;
            if valid.is_none() {
                return Err(Error::Invalid("run has no valid pinned embedding model"));
            }
        }
        return Ok(run);
    }
    if let Some(run_id) = sqlx::query_scalar("SELECT r.id FROM ingest.runs r WHERE r.source_id=$1 AND r.selection_snapshot ? 'repair' AND r.state<>'failed' AND EXISTS(SELECT FROM queue.jobs j WHERE j.scope_id=r.id AND j.state IN ('ready','leased')) ORDER BY r.started_at,r.id LIMIT 1")
        .bind(p.source_id).fetch_optional(&mut **tx).await? {
        return Err(Error::SourceBusy { source_id:p.source_id, run_id });
    }
    let model: Option<Uuid> = if !crate::indexing::needs_model(p)? {
        None
    } else {
        sqlx::query_scalar("SELECT id FROM search.embedding_models WHERE state IN ('building','active') ORDER BY (state='active') DESC,name,revision,id LIMIT 1").fetch_optional(&mut **tx).await?
    };
    if crate::indexing::needs_model(p)? && model.is_none() {
        return Err(Error::Invalid("run has no valid pinned embedding model"));
    }
    let snapshot = serde_json::to_value(p).map_err(|_| Error::Invalid("policy"))?;
    let run:Run=sqlx::query_as("INSERT INTO ingest.runs(policy_id,policy_version,policy_snapshot,selection_snapshot,source_id,embedding_model_id) VALUES($1,$2,$3,$4,$5,$6) RETURNING *")
        .bind(p.id).bind(p.version).bind(snapshot).bind(selection).bind(p.source_id).bind(model).fetch_one(&mut **tx).await?;
    enqueue(
        tx,
        run.id,
        &Followup {
            kind: JobKind::Ingest,
            dedupe_key: format!("run:{}", run.id),
            input: json!({}),
        },
    )
    .await?;
    Ok(run)
}

async fn fail_exhausted_run(tx: &mut Transaction<'_, Postgres>, run: Uuid) -> Result<()> {
    let changed=sqlx::query("UPDATE ingest.runs r SET state='failed',finished_at=now() WHERE r.id=$1 AND r.state='running' AND EXISTS(SELECT FROM queue.jobs j WHERE j.scope_id=r.id AND j.kind='ingest' AND j.state='failed')")
        .bind(run).execute(&mut **tx).await?.rows_affected();
    let leased_in_failed: bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM ingest.runs r JOIN queue.jobs j ON j.scope_id=r.id WHERE r.id=$1 AND r.state='failed' AND j.state='leased')").bind(run).fetch_one(&mut **tx).await?;
    if changed == 1 || leased_in_failed {
        crate::queue::cancel_scope(tx, "indexing", run, true).await?;
    }
    Ok(())
}

pub(crate) async fn enqueue(
    tx: &mut Transaction<'_, Postgres>,
    run: Uuid,
    j: &Followup,
) -> Result<()> {
    if j.dedupe_key.is_empty() || j.dedupe_key.len() > 500 || j.input.to_string().len() > 32768 {
        return Err(Error::Invalid("job"));
    }
    let (adapter,source_key,model): (String,String,Option<Uuid>) = sqlx::query_as("SELECT COALESCE(s.configuration->>'adapter','unsupported'),s.source_key,r.embedding_model_id FROM ingest.runs r JOIN catalog.sources s ON s.id=r.source_id WHERE r.id=$1").bind(run).fetch_one(&mut **tx).await?;
    let mut requirements = vec![format!("adapter:{adapter}")];
    if adapter == "jellyfin" {
        requirements.push(format!("source:{source_key}"));
        let revision: Option<String> = sqlx::query_scalar("SELECT s.configuration->>'locator_revision' FROM catalog.sources s JOIN ingest.runs r ON r.source_id=s.id WHERE r.id=$1").bind(run).fetch_one(&mut **tx).await?;
        if let Some(revision) = revision {
            requirements.push(format!("locator:{revision}"));
        }
    }
    let mut input = j.input.clone();
    let mut dedupe_key = j.dedupe_key.clone();
    if matches!(j.kind, JobKind::Embedding) {
        let model = model.ok_or(Error::Invalid("run has no pinned embedding model"))?;
        input["model_id"] = json!(model);
        let external = input["external_id"]
            .as_str()
            .ok_or(Error::Invalid("embedding reference"))?;
        let hash: Option<String>=sqlx::query_scalar("SELECT encode(d.content_hash,'hex') FROM search.documents d JOIN catalog.source_items si ON si.id=d.source_item_id JOIN ingest.runs r ON r.source_id=si.source_id WHERE r.id=$1 AND si.external_id=$2").bind(run).bind(external).fetch_optional(&mut **tx).await?;
        if let Some(hash) = &hash {
            dedupe_key = format!("{}:{hash}", j.dedupe_key);
        }
        input["input_hash"] = json!(hash);
        requirements.push(format!("model:{model}"));
    }
    let result = crate::queue::enqueue(
        tx,
        &crate::queue::Enqueue {
            queue: "indexing",
            scope_id: run,
            kind: j.kind.as_str(),
            dedupe_key: &dedupe_key,
            input: &input,
            requirements: &requirements,
        },
    )
    .await?;
    if let crate::queue::EnqueueResult::Inserted(id) = result {
        let snapshot: Value =
            sqlx::query_scalar("SELECT policy_snapshot FROM ingest.runs WHERE id=$1")
                .bind(run)
                .fetch_one(&mut **tx)
                .await?;
        if snapshot["selection_json"].get("indexing").is_some() {
            let config = crate::indexing::Configuration::from_value(&snapshot["selection_json"])?;
            crate::queue::configure_retry(tx, id, config.limits.retry).await?;
        }
    }
    Ok(())
}

pub(crate) async fn guard(tx: &mut Transaction<'_, Postgres>, c: &Claim) -> Result<(Run, Policy)> {
    let run_id = c.run_id.ok_or(Error::Stale)?;
    // Source, cooldown, policy, run, job is the shared worker lock order.
    let source: Uuid = sqlx::query_scalar("SELECT source_id FROM ingest.runs WHERE id=$1")
        .bind(run_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(Error::Stale)?;
    let enabled: bool =
        sqlx::query_scalar("SELECT state='enabled' FROM catalog.sources WHERE id=$1 FOR SHARE")
            .bind(source)
            .fetch_one(&mut **tx)
            .await?;
    sqlx::query("INSERT INTO ingest.source_cooldowns(source_id) VALUES($1) ON CONFLICT DO NOTHING")
        .bind(source)
        .execute(&mut **tx)
        .await?;
    sqlx::query("SELECT source_id FROM ingest.source_cooldowns WHERE source_id=$1 FOR UPDATE")
        .bind(source)
        .execute(&mut **tx)
        .await?;
    let p:Policy=sqlx::query_as("SELECT p.* FROM ingest.policies p JOIN ingest.runs r ON r.policy_id=p.id WHERE r.id=$1 FOR UPDATE OF p").bind(run_id).fetch_optional(&mut **tx).await?.ok_or(Error::Stale)?;
    let run: Run = sqlx::query_as("SELECT * FROM ingest.runs WHERE id=$1 FOR UPDATE")
        .bind(run_id)
        .fetch_one(&mut **tx)
        .await?;
    WorkBudget::execution_limits(&run.selection_snapshot)?;
    let admitted: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT FROM ingest.indexing_runs WHERE run_id=$1)")
            .bind(run_id)
            .fetch_one(&mut **tx)
            .await?;
    crate::indexing::check_deadline(tx, &run).await?;
    let unchanged: bool = sqlx::query_scalar("SELECT NOT EXISTS(SELECT FROM ingest.indexing_runs ir JOIN catalog.sources s ON s.id=$2 WHERE ir.run_id=$1 AND ir.source_configuration<>s.configuration)").bind(run_id).bind(source).fetch_one(&mut **tx).await?;
    if !admitted
        || !p.enabled
        || !unchanged
        || !enabled
        || p.version != run.policy_version
        || !matches!(run.state.as_str(), "running" | "complete")
    {
        return Err(Error::Stale);
    }
    crate::queue::guard(tx, &c.transport()).await?;
    Ok((run, p))
}
pub(crate) async fn finish(tx: &mut Transaction<'_, Postgres>, c: &Claim) -> Result<()> {
    if c.kind != "ingest" {
        sqlx::query("INSERT INTO ingest.request_resolutions(receipt_id,reason) SELECT id,'successful_replay' FROM ingest.request_receipts WHERE job_id=$1 AND outcome IN ('unknown','retryable','permanent') ON CONFLICT DO NOTHING").bind(c.id).execute(&mut **tx).await?;
    }
    sqlx::query("UPDATE queue.jobs SET window_requests=0 WHERE id=$1")
        .bind(c.id)
        .execute(&mut **tx)
        .await?;
    crate::queue::ack(tx, &c.transport()).await?;
    if let Some(run) = c.run_id {
        crate::indexing::settle(tx, run).await?;
        let run: Run = sqlx::query_as("SELECT * FROM ingest.runs WHERE id=$1")
            .bind(run)
            .fetch_one(&mut **tx)
            .await?;
        crate::indexing::check_deadline(tx, &run).await?;
    }
    Ok(())
}
