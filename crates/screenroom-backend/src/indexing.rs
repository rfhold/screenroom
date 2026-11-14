//! Versioned admission for saved indexing policies; all execution uses the shared queue.
use crate::{
    Error, Result,
    ingest::{self, Policy, Run, WorkBudget},
    queue::RetryPolicy,
    repair::{ArtworkPolicy, PosterPolicy, RepairExecution, Scope},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Full,
    Incremental,
    Reconcile,
    Repair,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Strategy {
    Missing,
    Changed,
    Stale,
    Failed,
    All,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Components {
    pub metadata: bool,
    pub artwork: bool,
    pub embeddings: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limits {
    pub work_requests: i64,
    pub work_records: usize,
    pub lifetime_requests: i64,
    pub lifetime_records: i64,
    pub deadline_seconds: i32,
    pub work_timeout_seconds: i32,
    #[serde(default)]
    pub retry: RetryPolicy,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Configuration {
    pub indexing: u32,
    pub enabled: bool,
    #[serde(default)]
    pub automatic_runs: bool,
    pub cadence_seconds: i32,
    #[serde(default = "default_reconcile")]
    pub reconcile_seconds: i32,
    #[serde(default = "default_overlap")]
    pub overlap_seconds: i32,
    #[serde(default = "default_concurrency")]
    pub source_concurrency: u32,
    pub mode: Mode,
    pub components: Components,
    pub strategy: Strategy,
    #[serde(default)]
    pub stale_after_seconds: Option<i32>,
    pub scope: Scope,
    pub operator_profile_id: String,
    pub limits: Limits,
    #[serde(default = "default_skip")]
    pub artwork: ArtworkPolicy,
}
fn default_skip() -> ArtworkPolicy {
    ArtworkPolicy {
        posters: PosterPolicy {
            skip_existing: true,
        },
    }
}
fn default_reconcile() -> i32 {
    604800
}
fn default_overlap() -> i32 {
    300
}
fn default_concurrency() -> u32 {
    1
}
impl Configuration {
    pub fn from_value(v: &Value) -> Result<Self> {
        let c: Self = serde_json::from_value(v.clone())
            .map_err(|_| Error::Invalid("indexing configuration"))?;
        c.validate()?;
        Ok(c)
    }
    pub fn validate(&self) -> Result<()> {
        self.scope.validate()?;
        self.limits
            .retry
            .validate()
            .map_err(|_| Error::Invalid("indexing retry"))?;
        let l = &self.limits;
        if self.indexing != 1
            || !(60..=2592000).contains(&self.cadence_seconds)
            || !(60..=2592000).contains(&self.reconcile_seconds)
            || !(1..=86400).contains(&self.overlap_seconds)
            || self.source_concurrency != 1
            || !(7..=1000).contains(&l.work_requests)
            || !(3..=1000).contains(&l.work_records)
            || !(1..=100000000).contains(&l.lifetime_requests)
            || !(1..=100000000).contains(&l.lifetime_records)
            || !(60..=2592000).contains(&l.deadline_seconds)
            || !(1..=240).contains(&l.work_timeout_seconds)
            || self.operator_profile_id.is_empty()
            || self.operator_profile_id.len() > 200
            || !self
                .operator_profile_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        {
            return Err(Error::Invalid("indexing version or limits"));
        }
        if self.mode == Mode::Repair {
            if self.components
                != (Components {
                    metadata: false,
                    artwork: true,
                    embeddings: false,
                })
            {
                return Err(Error::Invalid("repair supports posters only"));
            }
            if self.artwork.posters.skip_existing != (self.strategy == Strategy::Missing) {
                return Err(Error::Invalid(
                    "poster strategy conflicts with skip_existing",
                ));
            }
        } else if !self.components.metadata
            || self.strategy != Strategy::All
            || !self.scope.item_ids.is_empty()
            || self.scope.item_kinds.len() != 2
        {
            return Err(Error::Invalid(
                "full traversal requires all metadata and both item kinds",
            ));
        }
        if self.strategy == Strategy::Stale {
            if self.mode != Mode::Repair
                || !self
                    .stale_after_seconds
                    .is_some_and(|n| (60..=2592000).contains(&n))
            {
                return Err(Error::Invalid("stale repair requires age threshold"));
            }
        } else if self.stale_after_seconds.is_some() {
            return Err(Error::Invalid("age threshold is stale-only"));
        }
        if self.mode != Mode::Repair && !self.artwork.posters.skip_existing {
            return Err(Error::Invalid(
                "full artwork uses revision-aware reuse; skip_existing override is repair-only",
            ));
        }
        Ok(())
    }
    pub fn effective(&self, scope: Option<&Scope>) -> Result<Self> {
        let mut c = self.clone();
        if let Some(scope) = scope {
            let narrowed = RepairExecution {
                repair: 1,
                artwork: self.artwork.clone(),
                scope: self.scope.clone(),
                work_budget: self.work_budget(),
            }
            .effective(Some(scope))?;
            c.scope = narrowed.scope;
        }
        c.validate()?;
        Ok(c)
    }
    fn work_budget(&self) -> WorkBudget {
        WorkBudget::PerWorkCommit {
            max_requests: self.limits.work_requests,
            max_records: self.limits.work_records,
        }
    }
    fn execution(&self) -> Value {
        if self.mode == Mode::Repair {
            json!(RepairExecution {
                repair: 1,
                artwork: self.artwork.clone(),
                scope: self.scope.clone(),
                work_budget: self.work_budget()
            })
        } else {
            json!({"full_index":1,"library_ids":self.scope.library_ids,"operator_profile_id":self.operator_profile_id,"work_budget":self.work_budget()})
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigureRequest {
    pub request_id: Uuid,
    pub existing: Option<Existing>,
    pub configuration: Configuration,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Existing {
    pub policy_id: Uuid,
    pub expected_version: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartRequest {
    pub request_id: Uuid,
    pub policy_id: Uuid,
    pub policy_version: i64,
    pub scope: Option<Scope>,
}
#[derive(Clone)]
pub struct IndexingRepository {
    pool: PgPool,
}
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ExecutionState {
    pub mode: String,
    pub lower_watermark: Option<DateTime<Utc>>,
    pub candidate_watermark: Option<DateTime<Utc>>,
    pub settled_at: Option<DateTime<Utc>>,
    pub fallback_reason: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Progress {
    pub policy_version: i64,
    pub watermark: Option<DateTime<Utc>>,
    pub next_reconcile_at: DateTime<Utc>,
}
async fn receipt<T: serde::de::DeserializeOwned>(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    op: &str,
    input: &Value,
) -> Result<Option<T>> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(id.to_string())
        .execute(&mut **tx)
        .await?;
    let old: Option<(String, Value, Value)> = sqlx::query_as(
        "SELECT operation,input,result FROM ingest.indexing_receipts WHERE request_id=$1",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some((operation, original, result)) = old {
        if operation != op || original != *input {
            return Err(Error::Stale);
        }
        return Ok(Some(
            serde_json::from_value(result).map_err(|_| Error::Invalid("indexing receipt"))?,
        ));
    }
    Ok(None)
}
async fn save(
    tx: &mut Transaction<'_, Postgres>,
    id: Uuid,
    op: &str,
    input: &Value,
    result: Value,
) -> Result<()> {
    sqlx::query("INSERT INTO ingest.indexing_receipts(request_id,operation,input,result) VALUES($1,$2,$3,$4)").bind(id).bind(op).bind(input).bind(result).execute(&mut **tx).await?;
    Ok(())
}
impl IndexingRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    /// Explicit provisioning boundary; never called implicitly by worker claims.
    pub async fn enroll_locator(&self, source: Uuid, revision: &str) -> Result<()> {
        validate_locator(revision)?;
        let changed = sqlx::query("UPDATE catalog.sources SET configuration=jsonb_set(configuration,'{locator_revision}',to_jsonb($2::text)) WHERE id=$1 AND configuration->>'adapter'='jellyfin'")
            .bind(source).bind(revision).execute(&self.pool).await?.rows_affected();
        if changed != 1 {
            return Err(Error::Invalid("configured Jellyfin source required"));
        }
        Ok(())
    }
    pub async fn validate_locator(&self, source_key: &str, revision: &str) -> Result<()> {
        validate_locator(revision)?;
        let matches: bool = sqlx::query_scalar("SELECT EXISTS(SELECT FROM catalog.sources WHERE source_key=$1 AND configuration->>'locator_revision'=$2 AND state='enabled')").bind(source_key).bind(revision).fetch_one(&self.pool).await?;
        if !matches {
            return Err(Error::Invalid("source locator requires enrollment"));
        }
        Ok(())
    }
    pub async fn execution(&self, run: Uuid) -> Result<Option<ExecutionState>> {
        Ok(sqlx::query_as("SELECT mode,lower_watermark,candidate_watermark,settled_at,fallback_reason FROM ingest.indexing_runs WHERE run_id=$1").bind(run).fetch_optional(&self.pool).await?)
    }
    pub async fn progress(&self, policy: Uuid) -> Result<Option<Progress>> {
        Ok(sqlx::query_as("SELECT policy_version,watermark,next_reconcile_at FROM ingest.indexing_progress WHERE policy_id=$1").bind(policy).fetch_optional(&self.pool).await?)
    }
    pub async fn configure(&self, source: Uuid, request: &ConfigureRequest) -> Result<Policy> {
        let tx = self.pool.begin().await?;
        let (policy, tx) = Self::configure_in(tx, source, request).await?;
        tx.commit().await?;
        Ok(policy)
    }
    pub(crate) async fn configure_in<'a>(
        mut tx: Transaction<'a, Postgres>,
        source: Uuid,
        request: &ConfigureRequest,
    ) -> Result<(Policy, Transaction<'a, Postgres>)> {
        request.configuration.validate()?;
        let input = json!({"source_id":source,"request":request});
        if let Some(p) = receipt(&mut tx, request.request_id, "configure", &input).await? {
            return Ok((p, tx));
        }
        let adapter:Option<String>=sqlx::query_scalar("SELECT configuration->>'adapter' FROM catalog.sources WHERE id=$1 AND state='enabled' FOR UPDATE").bind(source).fetch_optional(&mut *tx).await?.flatten();
        if adapter.as_deref() != Some("jellyfin") {
            return Err(Error::Invalid(
                "indexing requires configured Jellyfin source",
            ));
        }
        let c = &request.configuration;
        require_locator(&mut tx, source).await?;
        crate::repair::validate_scope(&mut tx, source, &c.scope).await?;
        let busy:Option<Uuid>=sqlx::query_scalar("SELECT r.id FROM ingest.runs r WHERE r.source_id=$1 AND (r.state IN ('running','paused') OR EXISTS(SELECT FROM queue.jobs j WHERE j.scope_id=r.id AND j.state IN ('ready','leased'))) LIMIT 1").bind(source).fetch_optional(&mut *tx).await?;
        if let Some(run_id) = busy {
            return Err(Error::SourceBusy {
                source_id: source,
                run_id,
            });
        }
        let p: Policy = if let Some(e) = &request.existing {
            let old: Policy = sqlx::query_as(
                "SELECT * FROM ingest.policies WHERE id=$1 AND source_id=$2 FOR UPDATE",
            )
            .bind(e.policy_id)
            .bind(source)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error::NotFound)?;
            Configuration::from_value(&old.selection_json)?;
            if old.version != e.expected_version {
                return Err(Error::Stale);
            }
            sqlx::query_as("UPDATE ingest.policies SET version=version+1,enabled=$2,automatic_runs=$3,selection_json=$4,max_items=$5,max_requests=$6,incremental_seconds=$7,reconcile_seconds=$10,enrich_metadata=$8,enrich_artwork=$9,next_due_at=clock_timestamp()+make_interval(secs=>$7) WHERE id=$1 RETURNING *")
                .bind(old.id).bind(c.enabled).bind(c.automatic_runs).bind(json!(c)).bind(c.limits.lifetime_records).bind(c.limits.lifetime_requests).bind(c.cadence_seconds).bind(c.components.metadata).bind(c.components.artwork).bind(c.reconcile_seconds).fetch_one(&mut *tx).await?
        } else {
            sqlx::query_as("INSERT INTO ingest.policies(source_id,enabled,automatic_runs,selection_json,max_items,max_requests,incremental_seconds,reconcile_seconds,enrich_metadata,enrich_people,enrich_artwork,next_due_at) VALUES($1,$2,$3,$4,$5,$6,$7,$10,$8,false,$9,clock_timestamp()+make_interval(secs=>$7)) RETURNING *")
                .bind(source).bind(c.enabled).bind(c.automatic_runs).bind(json!(c)).bind(c.limits.lifetime_records).bind(c.limits.lifetime_requests).bind(c.cadence_seconds).bind(c.components.metadata).bind(c.components.artwork).bind(c.reconcile_seconds).fetch_one(&mut *tx).await?
        };
        sqlx::query("INSERT INTO ingest.indexing_progress(policy_id,policy_version) VALUES($1,$2) ON CONFLICT(policy_id) DO UPDATE SET policy_version=EXCLUDED.policy_version,watermark=NULL,next_reconcile_at=clock_timestamp()")
            .bind(p.id).bind(p.version).execute(&mut *tx).await?;
        save(&mut tx, request.request_id, "configure", &input, json!(p)).await?;
        Ok((p, tx))
    }
    pub async fn start(&self, request: &StartRequest) -> Result<Run> {
        let tx = self.pool.begin().await?;
        let (run, tx) = Self::start_in(tx, request).await?;
        tx.commit().await?;
        Ok(run)
    }
    pub(crate) async fn start_in<'a>(
        mut tx: Transaction<'a, Postgres>,
        request: &StartRequest,
    ) -> Result<(Run, Transaction<'a, Postgres>)> {
        let input = json!(request);
        if let Some(run) = receipt(&mut tx, request.request_id, "start", &input).await? {
            return Ok((run, tx));
        }
        ingest::lock_policy_source(&mut tx, request.policy_id).await?;
        let p: Policy = sqlx::query_as("SELECT * FROM ingest.policies WHERE id=$1 FOR UPDATE")
            .bind(request.policy_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error::NotFound)?;
        if p.version != request.policy_version {
            return Err(Error::Stale);
        }
        let c = Configuration::from_value(&p.selection_json)?.effective(request.scope.as_ref())?;
        require_locator(&mut tx, p.source_id).await?;
        let (watermark, reconcile_due): (Option<DateTime<Utc>>, bool) = sqlx::query_as("SELECT watermark,next_reconcile_at<=clock_timestamp() FROM ingest.indexing_progress WHERE policy_id=$1 AND policy_version=$2 FOR UPDATE")
            .bind(p.id).bind(p.version).fetch_one(&mut *tx).await?;
        let actual_mode = if c.mode == Mode::Incremental && (watermark.is_none() || reconcile_due) {
            Mode::Reconcile
        } else {
            c.mode
        };
        let run = ingest::start_selection_locked(&mut tx, &p, &c.execution()).await?;
        if run.selection_snapshot != c.execution() {
            return Err(Error::Stale);
        }
        let binding_current:bool=sqlx::query_scalar("SELECT NOT EXISTS(SELECT FROM ingest.indexing_runs ir JOIN catalog.sources s ON s.id=$2 WHERE ir.run_id=$1 AND ir.source_configuration<>s.configuration)").bind(run.id).bind(p.source_id).fetch_one(&mut *tx).await?;
        if !binding_current {
            return Err(Error::Stale);
        }
        if c.mode == Mode::Repair {
            let selection = RepairExecution::from_value(&run.selection_snapshot)?;
            crate::repair::select_candidates(&mut tx, &run, &selection).await?;
            let selected: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM ingest.maintenance_candidates WHERE run_id=$1",
            )
            .bind(run.id)
            .fetch_one(&mut *tx)
            .await?;
            if selected > c.limits.lifetime_records {
                return Err(Error::Invalid("repair candidate limit exceeded"));
            }
        }
        // A narrowed manual run cannot advance the saved policy's whole-scope watermark.
        let whole_scope = c.scope == Configuration::from_value(&p.selection_json)?.scope;
        sqlx::query("INSERT INTO ingest.indexing_runs(run_id,source_configuration,mode,lower_watermark,candidate_watermark) SELECT $1,configuration,$3,$4,$5 FROM catalog.sources WHERE id=$2 ON CONFLICT(run_id) DO NOTHING")
            .bind(run.id).bind(p.source_id).bind(match actual_mode { Mode::Full=>"full",Mode::Reconcile=>"reconcile",Mode::Repair=>"repair",Mode::Incremental=>"incremental" })
            .bind(watermark.map(|w| w-chrono::Duration::seconds(i64::from(c.overlap_seconds))))
            .bind(if whole_scope && c.mode!=Mode::Repair { Some(run.started_at) } else { None }).execute(&mut *tx).await?;
        sqlx::query("UPDATE ingest.policies SET next_due_at=clock_timestamp()+make_interval(secs=>incremental_seconds) WHERE id=$1").bind(p.id).execute(&mut *tx).await?;
        save(&mut tx, request.request_id, "start", &input, json!(run)).await?;
        Ok((run, tx))
    }
    pub async fn policies(
        &self,
        source: Uuid,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<Policy>> {
        if !(1..=200).contains(&limit) {
            return Err(Error::Invalid("page"));
        }
        Ok(sqlx::query_as("SELECT * FROM ingest.policies WHERE source_id=$1 AND selection_json ? 'indexing' AND ($2::uuid IS NULL OR id>$2) ORDER BY id LIMIT $3").bind(source).bind(after).bind(i64::from(limit)).fetch_all(&self.pool).await?)
    }
}

fn validate_locator(revision: &str) -> Result<()> {
    screenroom_catalog_adapter::jellyfin::ConfigurationRevision::parse(revision)
        .map_err(|_| Error::Invalid("source locator requires enrollment"))?;
    Ok(())
}
async fn require_locator(tx: &mut Transaction<'_, Postgres>, source: Uuid) -> Result<()> {
    let revision: Option<String> = sqlx::query_scalar(
        "SELECT configuration->>'locator_revision' FROM catalog.sources WHERE id=$1",
    )
    .bind(source)
    .fetch_one(&mut **tx)
    .await?;
    validate_locator(
        revision
            .as_deref()
            .ok_or(Error::Invalid("source locator requires enrollment"))?,
    )
}
pub fn configuration(run: &Run) -> Result<Option<Configuration>> {
    let value = &run.policy_snapshot["selection_json"];
    if value.get("indexing").is_some() {
        Ok(Some(Configuration::from_value(value)?))
    } else {
        Ok(None)
    }
}
pub fn needs_model(p: &Policy) -> Result<bool> {
    if p.selection_json.get("indexing").is_some() {
        Ok(Configuration::from_value(&p.selection_json)?
            .components
            .embeddings)
    } else {
        Ok(true)
    }
}
pub fn deadline(run: &Run) -> Result<Option<DateTime<Utc>>> {
    Ok(configuration(run)?
        .map(|c| run.started_at + chrono::Duration::seconds(i64::from(c.limits.deadline_seconds))))
}
pub(crate) async fn check_deadline(tx: &mut Transaction<'_, Postgres>, run: &Run) -> Result<()> {
    if let Some(deadline) = deadline(run)? {
        let expired: bool = sqlx::query_scalar("SELECT clock_timestamp()>=$1")
            .bind(deadline)
            .fetch_one(&mut **tx)
            .await?;
        if expired {
            return Err(Error::Invalid("indexing deadline expired"));
        }
    }
    Ok(())
}

pub(crate) async fn settle(tx: &mut Transaction<'_, Postgres>, run: Uuid) -> Result<()> {
    let state: Run = sqlx::query_as("SELECT * FROM ingest.runs WHERE id=$1")
        .bind(run)
        .fetch_one(&mut **tx)
        .await?;
    check_deadline(tx, &state).await?;
    let eligible: Option<(Uuid, i64, DateTime<Utc>, String, i32)> = sqlx::query_as("SELECT r.policy_id,r.policy_version,ir.candidate_watermark,ir.mode,p.reconcile_seconds FROM ingest.indexing_runs ir JOIN ingest.runs r ON r.id=ir.run_id JOIN ingest.policies p ON p.id=r.policy_id JOIN catalog.sources s ON s.id=r.source_id WHERE r.id=$1 AND ir.settled_at IS NULL AND ir.candidate_watermark IS NOT NULL AND r.state='complete' AND p.enabled AND p.version=r.policy_version AND s.state='enabled' AND s.configuration=ir.source_configuration AND NOT EXISTS(SELECT FROM queue.jobs j WHERE j.scope_id=r.id AND j.state<>'complete')")
        .bind(run).fetch_optional(&mut **tx).await?;
    let Some((policy, version, candidate, mode, reconcile)) = eligible else {
        return Ok(());
    };
    let coverage = ingest::coverage_in(tx, run).await?;
    if coverage.pending_artwork_publications
        + coverage.omissions
        + coverage.unresolved_operations
        + coverage.unresolved_attempts
        > 0
    {
        return Ok(());
    }
    sqlx::query("UPDATE ingest.indexing_progress SET watermark=GREATEST(watermark,$3),next_reconcile_at=CASE WHEN $4 THEN clock_timestamp()+make_interval(secs=>$5) ELSE next_reconcile_at END WHERE policy_id=$1 AND policy_version=$2")
        .bind(policy).bind(version).bind(candidate).bind(mode!="incremental").bind(reconcile).execute(&mut **tx).await?;
    sqlx::query("UPDATE ingest.indexing_runs SET settled_at=clock_timestamp() WHERE run_id=$1")
        .bind(run)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
