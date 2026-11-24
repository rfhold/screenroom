use screenroom_backend::{
    Error,
    ingest::IngestRepository,
    repair::{self as m, RepairRepository},
    sqlx::{self, PgPool},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use uuid::Uuid;

mod rpc;
pub use rpc::serve;

const BODY_LIMIT: usize = 65_536;

#[derive(Clone)]
pub struct Operator {
    pool: PgPool,
    connections: screenroom_backend::connections::ConnectionRepository,
}
impl Operator {
    pub fn new_rpc(pool: PgPool) -> Self {
        Self {
            connections: screenroom_backend::connections::ConnectionRepository::new(
                pool.clone(),
                None,
            ),
            pool,
        }
    }
    pub fn with_connections(
        mut self,
        connections: screenroom_backend::connections::ConnectionRepository,
    ) -> Self {
        self.connections = connections;
        self
    }
    fn repository(&self) -> RepairRepository {
        RepairRepository::new(self.pool.clone())
    }
    async fn policy_owner(&self, source: Uuid, policy: Uuid) -> Result<(), Error> {
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT FROM ingest.policies WHERE id=$1 AND source_id=$2 AND selection_json->>'indexing'='1')",
        )
        .bind(policy)
        .bind(source)
        .fetch_one(&self.pool)
        .await?;
        if exists { Ok(()) } else { Err(Error::NotFound) }
    }
    async fn run(&self, source: Uuid, run: Uuid) -> Result<m::RunStatus, Error> {
        let status = self.repository().run_status(run).await?;
        if status.run.source_id != source {
            return Err(Error::NotFound);
        }
        Ok(status)
    }
}

fn parse<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, Error> {
    serde_json::from_slice(bytes).map_err(|_| Error::Invalid("json"))
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CanonicalPolicy {
    pub id: Uuid,
    pub source_id: Uuid,
    pub version: i64,
    pub enabled: bool,
    pub automatic_runs: bool,
    pub next_due_at: screenroom_backend::chrono::DateTime<screenroom_backend::chrono::Utc>,
    pub configuration: Value,
}
pub type CanonicalRun = m::RunSummary;
fn canonical_policy(policy: screenroom_backend::ingest::Policy) -> Result<Value, Error> {
    let config = screenroom_backend::indexing::Configuration::from_value(&policy.selection_json)?;
    let configuration = json!({"indexing":config.indexing,"enabled":config.enabled,"automatic_runs":config.automatic_runs,"cadence_seconds":config.cadence_seconds,"reconcile_seconds":config.reconcile_seconds,"overlap_seconds":config.overlap_seconds,"source_concurrency":config.source_concurrency,"mode":config.mode,"components":config.components,"strategy":config.strategy,"stale_after_seconds":config.stale_after_seconds,"scope":config.scope,"limits":config.limits,"artwork":config.artwork});
    value(CanonicalPolicy {
        id: policy.id,
        source_id: policy.source_id,
        version: policy.version,
        enabled: policy.enabled,
        automatic_runs: policy.automatic_runs,
        next_due_at: policy.next_due_at,
        configuration,
    })
}
fn value(v: impl Serialize) -> Result<Value, Error> {
    serde_json::to_value(v).map_err(|_| Error::Unavailable)
}
fn id(s: &str) -> Result<Uuid, Error> {
    Uuid::parse_str(s).map_err(|_| Error::Invalid("id"))
}
fn default_limit() -> u32 {
    100
}
struct Page {
    after: Option<String>,
    limit: u32,
    pending_only: bool,
}
impl Page {
    fn uuid(&self) -> Result<Option<Uuid>, Error> {
        self.after.as_deref().map(id).transpose()
    }
}
fn page<T: Serialize>(items: Vec<T>, cursor: Option<String>, limit: u32) -> Value {
    let next = if items.len() == limit as usize {
        cursor
    } else {
        None
    };
    json!({"items":items,"next_after":next})
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Redrive {
    request_id: Uuid,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Diagnostics {
    run_id: Option<Uuid>,
    capabilities: Option<Vec<String>>,
}

#[derive(Clone, Copy)]
enum Command {
    Health,
    DiscoverConnection,
    SaveConnection,
    ListConnections,
    ListSources,
    ListLibraries(Uuid),
    ListRuns(Uuid),
    ListCandidates(Uuid, Uuid),
    ListPolicies(Uuid),
    ConfigurePolicy(Uuid),
    GetProgress(Uuid, Uuid),
    StartRun(Uuid),
    GetRunStatus(Uuid, Uuid),
    GetExecution(Uuid, Uuid),
    PauseRun(Uuid, Uuid),
    ResumeRun(Uuid, Uuid),
    CancelRun(Uuid, Uuid),
    ListJobs(Uuid, Uuid),
    ListAttempts(Uuid, Uuid, Uuid),
    ListRedrives(Uuid, Uuid, Uuid),
    RedriveJob(Uuid, Uuid, Uuid),
    ObserveQueue,
}

async fn execute(op: &Operator, command: Command, p: Page, body: &[u8]) -> Result<Value, Error> {
    use Command::*;
    let repo = op.repository();
    match command {
        ListConnections => {
            let rows = op.connections.list(p.uuid()?, p.limit).await?;
            let cursor = rows.last().map(|r| r.id.to_string());
            return Ok(page(rows, cursor, p.limit));
        }
        GetProgress(source, policy) => {
            op.policy_owner(source, policy).await?;
            return Ok(json!(
                screenroom_backend::indexing::IndexingRepository::new(op.pool.clone())
                    .progress(policy)
                    .await?
            ));
        }
        GetExecution(source, run) => {
            op.run(source, run).await?;
            return Ok(json!(
                screenroom_backend::indexing::IndexingRepository::new(op.pool.clone())
                    .execution(run)
                    .await?
            ));
        }
        ListPolicies(source) => {
            let rows = screenroom_backend::indexing::IndexingRepository::new(op.pool.clone())
                .policies(source, p.uuid()?, p.limit)
                .await?;
            let cursor = rows.last().map(|r| r.id.to_string());
            let rows = rows
                .into_iter()
                .map(canonical_policy)
                .collect::<Result<Vec<_>, Error>>()?;
            return Ok(page(rows, cursor, p.limit));
        }
        ListJobs(source, run) => {
            op.run(source, run).await?;
            let rows = IngestRepository::new(op.pool.clone())
                .jobs(run, p.uuid()?, p.limit)
                .await?;
            let cursor = rows.last().map(|r| r.id.to_string());
            let ingest = IngestRepository::new(op.pool.clone());
            let failed: Vec<_> = rows
                .iter()
                .filter(|row| row.state == "failed")
                .map(|row| row.id)
                .collect();
            let mut eligibility = ingest.redrive_eligibilities(&failed).await?;
            let mut summaries = Vec::with_capacity(rows.len());
            for row in rows {
                let eligibility = eligibility.remove(&row.id).unwrap_or_else(|| {
                    screenroom_backend::RedriveEligibility {
                        allowed: false,
                        reason: Some("not_terminal_failed".into()),
                    }
                });
                let mut item = value(row)?;
                item["redrive_allowed"] = json!(eligibility.allowed);
                item["redrive_reason"] = json!(eligibility.reason);
                summaries.push(item);
            }
            return Ok(page(summaries, cursor, p.limit));
        }
        ListAttempts(source, run, job) | ListRedrives(source, run, job) => {
            op.run(source, run).await?;
            let owned:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM queue.jobs WHERE id=$1 AND scope_id=$2 AND queue_name='indexing')").bind(job).bind(run).fetch_one(&op.pool).await?;
            if !owned {
                return Err(Error::NotFound);
            }
            let ingest = IngestRepository::new(op.pool.clone());
            if matches!(command, ListAttempts(..)) {
                let after = p
                    .after
                    .as_deref()
                    .unwrap_or("0")
                    .parse()
                    .map_err(|_| Error::Invalid("fence"))?;
                let rows = ingest.job_attempts(job, after, p.limit).await?;
                let cursor = rows.last().map(|r| r.fence.to_string());
                let rows:Vec<_>=rows.into_iter().map(|r|json!({"job_id":r.job_id,"fence":r.fence,"generation":r.generation,"started_at":r.started_at,"finished_at":r.finished_at,"outcome":r.outcome,"error_code":r.error_code})).collect();
                return Ok(page(rows, cursor, p.limit));
            }
            {
                let after = p
                    .after
                    .as_deref()
                    .unwrap_or("0")
                    .parse()
                    .map_err(|_| Error::Invalid("generation"))?;
                let rows = ingest.job_redrives(job, after, p.limit).await?;
                let cursor = rows.last().map(|r| r.generation.to_string());
                return Ok(page(rows, cursor, p.limit));
            }
        }
        Health => {
            sqlx::query("SELECT 1").execute(&op.pool).await?;
            return Ok(json!({"ready":true}));
        }
        ListSources => {
            let rows = repo.sources(p.uuid()?, p.limit).await?;
            let cursor = rows.last().map(|r| r.id.to_string());
            return Ok(page(rows, cursor, p.limit));
        }
        ListLibraries(source) => {
            let rows = repo.libraries(source, p.after.as_deref(), p.limit).await?;
            let cursor = rows.last().map(|r| r.public_id.clone());
            Ok(page(rows, cursor, p.limit))
        }
        ListRuns(source) => {
            let rows = repo.runs(Some(source), p.uuid()?, p.limit).await?;
            let cursor = rows.last().map(|r| r.id.to_string());
            return Ok(page(rows, cursor, p.limit));
        }
        GetRunStatus(source, run) => value(op.run(source, run).await?),
        ListCandidates(source, run) => {
            op.run(source, run).await?;
            let rows = repo
                .candidates(run, p.uuid()?, p.limit, p.pending_only)
                .await?;
            let cursor = rows.last().map(|r| r.source_item_id.to_string());
            let rows: Vec<_>=rows.into_iter().map(|r|json!({"source_item_id":r.source_item_id,"public_id":r.public_id,"kind":r.kind,"state":r.state,"code":safe_code(r.code.as_deref()),"selection_reason":r.selection_reason,"selected_at":r.selected_at,"updated_at":r.updated_at})).collect();
            return Ok(page(rows, cursor, p.limit));
        }
        DiscoverConnection => value(op.connections.discover(&parse(body)?).await?),
        SaveConnection => value(op.connections.save(&parse(body)?).await?),
        ObserveQueue => {
            let request: Diagnostics = parse(body)?;
            if request.capabilities.as_ref().is_some_and(|caps| {
                caps.len() > 100
                    || caps.iter().any(|c| {
                        c.len() > 300
                            || c.is_empty()
                            || !c
                                .bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b"_:-.,/".contains(&b))
                    })
            }) {
                return Err(Error::Invalid("capabilities"));
            }
            let ingest = IngestRepository::new(op.pool.clone());
            let observation = ingest
                .queue_observation(
                    request.run_id,
                    request.capabilities.as_deref().unwrap_or(&[]),
                )
                .await?;
            let mut snapshot = value(observation.counts)?;
            if request.capabilities.is_none() {
                for key in ["claimable", "reclaimable", "blocked"] {
                    snapshot[key] = Value::Null;
                }
            }
            let blockers = ingest
                .queue_blockers(request.run_id, request.capabilities.as_deref())
                .await?;
            Ok(
                json!({"queue_observation":{"started_at":observation.started_at,"finished_at":observation.finished_at},"blockers_as_of":blockers.as_of,"atomic":false,"effective_capabilities":request.capabilities,"worker_liveness":"unknown","queue":snapshot,"blockers":blockers,"count_semantics":{"blocked":"due-ready or expired-leased jobs with domain or capability blockers; attempt-limit-only jobs excluded; overlaps exhausted when another blocker exists","exhausted":"ready or expired-leased jobs at attempt limit, including delayed ready jobs"}}),
            )
        }
        ConfigurePolicy(source) => {
            let mut input: Value = parse(body)?;
            let config = input
                .get_mut("configuration")
                .and_then(Value::as_object_mut)
                .ok_or(Error::Invalid("configuration"))?;
            if config.contains_key("operator_profile_id") {
                return Err(Error::Invalid("private profile selection is server-side"));
            }
            let profile = op.connections.indexing_profile(source).await?;
            config.insert("operator_profile_id".into(), json!(profile));
            let request: screenroom_backend::indexing::ConfigureRequest =
                serde_json::from_value(input).map_err(|_| Error::Invalid("configuration"))?;
            canonical_policy(
                screenroom_backend::indexing::IndexingRepository::new(op.pool.clone())
                    .configure(source, &request)
                    .await?,
            )
        }
        StartRun(source) => {
            let request: screenroom_backend::indexing::StartRequest = parse(body)?;
            op.policy_owner(source, request.policy_id).await?;
            let run = screenroom_backend::indexing::IndexingRepository::new(op.pool.clone())
                .start(&request)
                .await?;
            value(op.run(source, run.id).await?.run)
        }
        RedriveJob(source, run, job) => {
            let request: Redrive = parse(body)?;
            op.run(source, run).await?;
            let owned:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM queue.jobs WHERE id=$1 AND scope_id=$2 AND queue_name='indexing')").bind(job).bind(run).fetch_one(&op.pool).await?;
            if !owned {
                return Err(Error::NotFound);
            }
            value(
                IngestRepository::new(op.pool.clone())
                    .redrive_job(job, request.request_id)
                    .await?,
            )
        }
        PauseRun(source, run) | ResumeRun(source, run) | CancelRun(source, run) => {
            let _: Empty = parse(body)?;
            op.run(source, run).await?;
            let ingest = IngestRepository::new(op.pool.clone());
            match command {
                PauseRun(..) => ingest.pause(run).await?,
                ResumeRun(..) => ingest.resume(run).await?,
                _ => ingest.cancel_run(run).await?,
            }
            value(op.run(source, run).await?)
        }
    }
}
fn safe_code(code: Option<&str>) -> Option<&str> {
    code.filter(|c| {
        [
            "ready",
            "no_poster",
            "unavailable",
            "invalid",
            "unauthorized",
            "transport",
            "timeout",
        ]
        .contains(c)
    })
}
