//! Canonical poster repair execution and candidate history.
use crate::{
    Error, Result,
    ingest::{self, Run, WorkBudget},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    Movie,
    Series,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub library_ids: Vec<String>,
    pub item_kinds: Vec<ItemKind>,
    #[serde(default)]
    pub item_ids: Vec<String>,
}
fn default_true() -> bool {
    true
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PosterPolicy {
    #[serde(default = "default_true")]
    pub skip_existing: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtworkPolicy {
    pub posters: PosterPolicy,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairExecution {
    pub repair: u32,
    pub artwork: ArtworkPolicy,
    pub scope: Scope,
    pub work_budget: WorkBudget,
}
impl RepairExecution {
    pub fn from_value(value: &Value) -> Result<Self> {
        let s: Self = serde_json::from_value(value.clone())
            .map_err(|_| Error::Invalid("maintenance selection"))?;
        s.validate()?;
        Ok(s)
    }
    pub fn validate(&self) -> Result<()> {
        self.scope.validate()?;
        if self.repair != 1
            || !matches!(
                self.work_budget,
                WorkBudget::PerWorkCommit {
                    max_requests: 7..,
                    ..
                }
            )
            || json!(self).to_string().len() > 32768
        {
            return Err(Error::Invalid("maintenance version or work budget"));
        }
        WorkBudget::from_selection(&json!(self))?;
        Ok(())
    }
    pub fn effective(&self, scope: Option<&Scope>) -> Result<Self> {
        self.validate()?;
        let mut result = self.clone();
        if let Some(scope) = scope {
            scope.validate()?;
            if !scope
                .library_ids
                .iter()
                .all(|x| self.scope.library_ids.contains(x))
                || !scope
                    .item_kinds
                    .iter()
                    .all(|x| self.scope.item_kinds.contains(x))
                || (!self.scope.item_ids.is_empty()
                    && (scope.item_ids.is_empty()
                        || !scope
                            .item_ids
                            .iter()
                            .all(|x| self.scope.item_ids.contains(x))))
            {
                return Err(Error::Invalid("maintenance override expands policy"));
            }
            result.scope = scope.clone();
        }
        Ok(result)
    }
}
impl Scope {
    pub fn validate(&self) -> Result<()> {
        fn ids(ids: &[String], max: usize) -> bool {
            ids.len() <= max
                && ids.iter().all(|s| {
                    !s.is_empty()
                        && s.len() <= 200
                        && s.chars()
                            .all(|c| c.is_ascii_alphanumeric() || "_-:.".contains(c))
                })
                && ids.iter().collect::<std::collections::BTreeSet<_>>().len() == ids.len()
        }
        if self.library_ids.is_empty()
            || !ids(&self.library_ids, 100)
            || !ids(&self.item_ids, 1000)
            || self.item_kinds.is_empty()
            || self.item_kinds.len() > 2
            || (self.item_kinds.len() == 2 && self.item_kinds[0] == self.item_kinds[1])
        {
            return Err(Error::Invalid("maintenance scope"));
        }
        Ok(())
    }
    fn kinds(&self) -> Vec<&'static str> {
        self.item_kinds
            .iter()
            .map(|k| match k {
                ItemKind::Movie => "movie",
                ItemKind::Series => "series",
            })
            .collect()
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Candidate {
    pub selection_reason: Option<String>,
    pub run_id: Uuid,
    pub source_id: Uuid,
    pub source_item_id: Uuid,
    pub external_id: String,
    pub public_id: String,
    pub kind: String,
    pub state: String,
    pub code: Option<String>,
    pub source_artwork_id: Option<String>,
    pub selected_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Disposition {
    Checked,
    SkippedReady,
    Staged,
    Unavailable,
    Failed,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeCode {
    Ready,
    NoPoster,
    Unavailable,
    Invalid,
    Unauthorized,
    Transport,
    Timeout,
}
#[derive(Debug, Clone)]
pub struct CandidateWrite {
    pub source_item_id: Uuid,
    pub disposition: Disposition,
    pub code: Option<OutcomeCode>,
    pub source_artwork_id: Option<String>,
}
#[derive(Debug, Clone, Default)]
pub struct RepairWrites {
    pub candidates: Vec<CandidateWrite>,
    pub outcomes: Vec<ingest::RequestOutcomeWrite>,
}
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Library {
    pub public_id: String,
    pub title: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Source {
    pub id: Uuid,
    pub source_key: String,
    pub provider: String,
    pub state: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct RunSummary {
    pub id: Uuid,
    pub source_id: Uuid,
    pub policy_id: Uuid,
    pub policy_version: i64,
    pub state: String,
    pub items: i64,
    pub requests: i64,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct QueueCounts {
    pub ready: i64,
    pub delayed: i64,
    pub leased: i64,
    pub failed: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct CandidateCounts {
    pub selected: i64,
    pub checked: i64,
    pub skipped_ready: i64,
    pub staged: i64,
    pub unavailable: i64,
    pub failed: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunStatus {
    pub observed_records: Option<i64>,
    pub terminal_reason: Option<String>,
    pub run: RunSummary,
    pub coverage: ingest::RunCoverage,
    pub queue: QueueCounts,
    pub candidates: CandidateCounts,
    pub allowed_actions: Vec<String>,
    pub all_complete: bool,
    pub enumeration_complete: bool,
    pub work_settled: bool,
    pub work_succeeded: bool,
    pub as_of: DateTime<Utc>,
}

pub(crate) fn poster_ready() -> String {
    format!(
        "EXISTS(SELECT FROM media.item_artwork ia JOIN media.artwork a ON a.id=ia.artwork_id JOIN media.artwork_generations g ON g.artwork_id=a.id AND g.revision=a.revision AND g.state='published' WHERE ia.source_item_id=si.id AND a.source_id=si.source_id AND a.state='enabled' AND a.kind='poster' AND {})",
        crate::media::FULL_GENERATION_READY
    )
}
fn scoped_items() -> &'static str {
    "FROM catalog.source_items si JOIN catalog.items i ON i.id=si.item_id JOIN catalog.sources s ON s.id=si.source_id WHERE si.source_id=$1 AND s.state='enabled' AND si.state='enabled' AND i.kind=ANY($3) AND (cardinality($4::text[])=0 OR i.public_id=ANY($4)) AND EXISTS(SELECT FROM catalog.library_items li JOIN catalog.libraries l ON l.id=li.library_id WHERE li.source_item_id=si.id AND li.source_id=si.source_id AND li.state='enabled' AND l.state='enabled' AND l.public_id=ANY($2))"
}
pub(crate) async fn validate_scope(
    tx: &mut Transaction<'_, Postgres>,
    source: Uuid,
    scope: &Scope,
) -> Result<()> {
    scope.validate()?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM catalog.libraries WHERE source_id=$1 AND public_id=ANY($2) AND state='enabled'")
        .bind(source).bind(&scope.library_ids).fetch_one(&mut **tx).await?;
    if count != scope.library_ids.len() as i64 {
        return Err(Error::Invalid("maintenance libraries outside source"));
    }
    if !scope.item_ids.is_empty() {
        let count: i64 = sqlx::query_scalar(&format!(
            "SELECT count(DISTINCT i.public_id) {}",
            scoped_items()
        ))
        .bind(source)
        .bind(&scope.library_ids)
        .bind(scope.kinds())
        .bind(&scope.item_ids)
        .fetch_one(&mut **tx)
        .await?;
        if count != scope.item_ids.len() as i64 {
            return Err(Error::Invalid("maintenance items outside scope"));
        }
    }
    Ok(())
}
#[derive(Clone)]
pub struct RepairRepository {
    pool: PgPool,
}
impl RepairRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    pub async fn sources(&self, after: Option<Uuid>, limit: u32) -> Result<Vec<Source>> {
        page_limit(limit)?;
        Ok(sqlx::query_as("SELECT id,source_key,provider,state FROM catalog.sources WHERE ($1::uuid IS NULL OR id>$1) ORDER BY id LIMIT $2")
            .bind(after).bind(i64::from(limit)).fetch_all(&self.pool).await?)
    }
    pub async fn runs(
        &self,
        source: Option<Uuid>,
        after: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<RunSummary>> {
        page_limit(limit)?;
        Ok(sqlx::query_as("SELECT r.id,r.source_id,r.policy_id,r.policy_version,r.state,r.items,r.requests,r.started_at,r.finished_at FROM ingest.runs r JOIN ingest.indexing_runs x ON x.run_id=r.id WHERE ($1::uuid IS NULL OR r.source_id=$1) AND ($2::uuid IS NULL OR r.id>$2) ORDER BY r.id LIMIT $3")
            .bind(source).bind(after).bind(i64::from(limit)).fetch_all(&self.pool).await?)
    }
    pub async fn run_status(&self, run: Uuid) -> Result<RunStatus> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await?;
        let summary: RunSummary = sqlx::query_as("SELECT r.id,r.source_id,r.policy_id,r.policy_version,r.state,r.items,r.requests,r.started_at,r.finished_at FROM ingest.runs r JOIN ingest.indexing_runs x ON x.run_id=r.id WHERE r.id=$1")
            .bind(run).fetch_optional(&mut *tx).await?.ok_or(Error::NotFound)?;
        let queue: QueueCounts = sqlx::query_as("SELECT count(*) FILTER(WHERE state='ready' AND available_at<=now()) AS ready,count(*) FILTER(WHERE state='ready' AND available_at>now()) AS delayed,count(*) FILTER(WHERE state='leased') AS leased,count(*) FILTER(WHERE state='failed') AS failed FROM queue.jobs WHERE scope_id=$1").bind(run).fetch_one(&mut *tx).await?;
        let candidates: CandidateCounts = sqlx::query_as("SELECT count(*) FILTER(WHERE state='selected') AS selected,count(*) FILTER(WHERE state='checked') AS checked,count(*) FILTER(WHERE state='skipped_ready') AS skipped_ready,count(*) FILTER(WHERE state='staged') AS staged,count(*) FILTER(WHERE state='unavailable') AS unavailable,count(*) FILTER(WHERE state='failed') AS failed FROM ingest.maintenance_candidates WHERE run_id=$1").bind(run).fetch_one(&mut *tx).await?;
        let active: bool = sqlx::query_scalar("SELECT p.enabled AND s.state='enabled' AND p.version=r.policy_version FROM ingest.runs r JOIN ingest.policies p ON p.id=r.policy_id JOIN catalog.sources s ON s.id=r.source_id WHERE r.id=$1").bind(run).fetch_one(&mut *tx).await?;
        let as_of = sqlx::query_scalar("SELECT now()")
            .fetch_one(&mut *tx)
            .await?;
        let coverage = ingest::coverage_in(&mut tx, run).await?;
        let (enumeration_complete,active_jobs,failed_jobs,cancelled_jobs):(bool,i64,i64,i64)=sqlx::query_as("SELECT COALESCE(bool_or(kind='ingest' AND state='complete'),false),count(*) FILTER(WHERE state IN ('ready','leased')),count(*) FILTER(WHERE state='failed'),count(*) FILTER(WHERE state='cancelled') FROM queue.jobs WHERE scope_id=$1 AND queue_name='indexing'").bind(run).fetch_one(&mut *tx).await?;
        let work_settled = active_jobs == 0;
        let work_succeeded = enumeration_complete
            && work_settled
            && failed_jobs + cancelled_jobs == 0
            && summary.state == "complete"
            && candidates.selected
                + candidates.checked
                + candidates.failed
                + candidates.unavailable
                == 0
            && coverage.pending_artwork_publications
                + coverage.omissions
                + coverage.unresolved_operations
                + coverage.unresolved_attempts
                == 0;
        let all_complete = work_succeeded;
        let allowed_actions = match summary.state.as_str() {
            "running" => vec!["pause".into(), "cancel".into()],
            "paused" if active => vec!["resume".into(), "cancel".into()],
            "paused" => vec!["cancel".into()],
            _ => vec![],
        };
        let observed_records =
            sqlx::query_scalar("SELECT observed_records FROM ingest.indexing_runs WHERE run_id=$1")
                .bind(run)
                .fetch_optional(&mut *tx)
                .await?;
        let terminal_reason=sqlx::query_scalar("SELECT CASE WHEN coverage->>'terminal_reason'='deadline_expired' THEN 'deadline_expired' END FROM ingest.runs WHERE id=$1").bind(run).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok(RunStatus {
            observed_records,
            terminal_reason,
            run: summary,
            coverage,
            queue,
            candidates,
            allowed_actions,
            all_complete,
            enumeration_complete,
            work_settled,
            work_succeeded,
            as_of,
        })
    }
    pub async fn libraries(
        &self,
        source: Uuid,
        after: Option<&str>,
        limit: u32,
    ) -> Result<Vec<Library>> {
        page_limit(limit)?;
        Ok(sqlx::query_as("SELECT public_id,title FROM catalog.libraries WHERE source_id=$1 AND state='enabled' AND ($2::text IS NULL OR public_id>$2) ORDER BY public_id LIMIT $3").bind(source).bind(after).bind(i64::from(limit)).fetch_all(&self.pool).await?)
    }
    pub async fn candidates(
        &self,
        run: Uuid,
        after: Option<Uuid>,
        limit: u32,
        pending_only: bool,
    ) -> Result<Vec<Candidate>> {
        page_limit(limit)?;
        Ok(sqlx::query_as("SELECT * FROM ingest.maintenance_candidates WHERE run_id=$1 AND ($2::uuid IS NULL OR source_item_id>$2) AND (NOT $4 OR state IN ('selected','checked')) ORDER BY source_item_id LIMIT $3")
            .bind(run).bind(after).bind(i64::from(limit)).bind(pending_only).fetch_all(&self.pool).await?)
    }
    pub async fn poster_is_ready(&self, source: Uuid, item: Uuid) -> Result<bool> {
        Ok(sqlx::query_scalar(&format!(
            "SELECT {} FROM catalog.source_items si WHERE si.id=$1 AND si.source_id=$2",
            poster_ready()
        ))
        .bind(item)
        .bind(source)
        .fetch_optional(&self.pool)
        .await?
        .unwrap_or(false))
    }
}
pub(crate) async fn select_candidates(
    tx: &mut Transaction<'_, Postgres>,
    run: &Run,
    selection: &RepairExecution,
) -> Result<()> {
    let s = &selection.scope;
    let config = crate::indexing::configuration(run)?;
    let stale = config
        .as_ref()
        .is_some_and(|c| c.strategy == crate::indexing::Strategy::Stale);
    let predicate = match config.as_ref().map(|c| c.strategy) {
        Some(crate::indexing::Strategy::Changed) => {
            "EXISTS(SELECT FROM media.item_artwork ia JOIN media.artwork a ON a.id=ia.artwork_id JOIN media.artwork_generations g ON g.artwork_id=a.id AND g.revision=a.revision AND g.state='published' WHERE ia.source_item_id=si.id AND a.kind='poster') AND EXISTS(SELECT FROM jsonb_array_elements(CASE WHEN jsonb_typeof(si.metadata->'provider_artwork_inputs')='array' THEN si.metadata->'provider_artwork_inputs' ELSE '[]'::jsonb END) input WHERE input->>'kind'='Primary' AND NOT EXISTS(SELECT FROM media.item_artwork ia JOIN media.artwork a ON a.id=ia.artwork_id JOIN media.artwork_generations g ON g.artwork_id=a.id AND g.revision=a.revision AND g.state='published' WHERE ia.source_item_id=si.id AND a.kind='poster' AND g.rights->'provider_input'=input))"
        }
        Some(crate::indexing::Strategy::Stale) => {
            "EXISTS(SELECT FROM media.item_artwork ia JOIN media.artwork a ON a.id=ia.artwork_id JOIN media.artwork_generations g ON g.artwork_id=a.id AND g.revision=a.revision AND g.state='published' WHERE ia.source_item_id=si.id AND a.kind='poster') AND COALESCE(COALESCE((SELECT max(c.updated_at) FROM ingest.maintenance_candidates c WHERE c.source_item_id=si.id AND c.state IN ('skipped_ready','staged') AND NOT EXISTS(SELECT FROM queue.jobs j WHERE j.scope_id=c.run_id AND j.state<>'complete')),(SELECT max(g.published_at) FROM media.item_artwork ia JOIN media.artwork a ON a.id=ia.artwork_id JOIN media.artwork_generations g ON g.artwork_id=a.id AND g.revision=a.revision AND g.state='published' WHERE ia.source_item_id=si.id AND a.kind='poster'))<=statement_timestamp()-make_interval(secs=>$7),true)"
        }
        Some(crate::indexing::Strategy::Failed) => {
            "COALESCE((SELECT c.state='failed' FROM ingest.maintenance_candidates c WHERE c.source_item_id=si.id AND c.state NOT IN ('selected','checked') ORDER BY c.updated_at DESC,c.run_id DESC LIMIT 1),false) OR EXISTS(SELECT FROM media.item_artwork ia JOIN media.artwork a ON a.id=ia.artwork_id JOIN media.artwork_generations g ON g.artwork_id=a.id AND g.revision=a.current_generation JOIN queue.jobs j ON j.kind='artwork' AND j.queue_name='indexing' AND j.input->>'artwork_id'=a.id::text AND j.input->>'revision'=g.revision::text AND j.input->'fingerprint'=(SELECT jsonb_agg(get_byte(g.fingerprint,n) ORDER BY n) FROM generate_series(0,31) n) WHERE ia.source_item_id=si.id AND g.kind='poster' AND (a.state='enabled' OR a.revision IS NULL) AND g.state='pending' AND j.state='failed')"
        }
        _ => "true",
    };
    sqlx::query(&format!("INSERT INTO ingest.maintenance_candidates(run_id,source_id,source_item_id,external_id,public_id,kind) SELECT $5,si.source_id,si.id,si.external_id,i.public_id,i.kind {} AND (NOT $6 OR NOT ({})) AND ($7::integer IS NULL OR $7>=0) AND ({predicate}) ON CONFLICT DO NOTHING",scoped_items(),poster_ready()))
        .bind(run.source_id).bind(&s.library_ids).bind(s.kinds()).bind(&s.item_ids).bind(run.id).bind(selection.artwork.posters.skip_existing).bind(config.and_then(|c|c.stale_after_seconds)).execute(&mut **tx).await?;
    if stale {
        sqlx::query("UPDATE ingest.maintenance_candidates selected SET selection_reason=CASE WHEN COALESCE((SELECT max(c.updated_at) FROM ingest.maintenance_candidates c WHERE c.source_item_id=selected.source_item_id AND c.state IN ('skipped_ready','staged') AND NOT EXISTS(SELECT FROM queue.jobs j WHERE j.scope_id=c.run_id AND j.state<>'complete')),(SELECT max(g.published_at) FROM media.item_artwork ia JOIN media.artwork a ON a.id=ia.artwork_id JOIN media.artwork_generations g ON g.artwork_id=a.id AND g.revision=a.revision AND g.state='published' WHERE ia.source_item_id=selected.source_item_id AND a.kind='poster')) IS NULL THEN 'unknown_age' ELSE 'known_stale' END WHERE selected.run_id=$1").bind(run.id).execute(&mut **tx).await?;
    }
    Ok(())
}
fn page_limit(limit: u32) -> Result<()> {
    if !(1..=1000).contains(&limit) {
        Err(Error::Invalid("maintenance page limit"))
    } else {
        Ok(())
    }
}
pub(crate) async fn write_candidates(
    tx: &mut Transaction<'_, Postgres>,
    run: &Run,
    page: &ingest::PageCommit,
    writes: &RepairWrites,
) -> Result<bool> {
    let selection = RepairExecution::from_value(&run.selection_snapshot)?;
    let WorkBudget::PerWorkCommit {
        max_records,
        max_requests,
    } = selection.work_budget;
    if writes.candidates.len() > max_records || writes.outcomes.len() > max_requests as usize {
        return Err(Error::Invalid("maintenance write bounds"));
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut progress = false;
    for write in &writes.candidates {
        if write.disposition == Disposition::SkippedReady
            && !selection.artwork.posters.skip_existing
        {
            return Err(Error::Invalid("refresh policy requires artwork staging"));
        }
        if !seen.insert(write.source_item_id) {
            return Err(Error::Invalid("duplicate maintenance candidate"));
        }
        let candidate: Candidate = sqlx::query_as("SELECT * FROM ingest.maintenance_candidates WHERE run_id=$1 AND source_id=$2 AND source_item_id=$3 FOR UPDATE")
            .bind(run.id).bind(run.source_id).bind(write.source_item_id).fetch_optional(&mut **tx).await?.ok_or(Error::Invalid("candidate outside run"))?;
        let state =
            serde_json::to_value(write.disposition).map_err(|_| Error::Invalid("disposition"))?;
        let state = state.as_str().ok_or(Error::Invalid("disposition"))?;
        let code = write.code.map(|c| json!(c).as_str().unwrap().to_owned());
        if !matches!(candidate.state.as_str(), "selected" | "checked") {
            if candidate.state != state
                || candidate.code != code
                || candidate.source_artwork_id != write.source_artwork_id
            {
                return Err(Error::Invalid("terminal candidate disposition conflict"));
            }
            if page.artwork_inputs.iter().any(|input| {
                input.reference.item_external_id.as_ref() == Some(&candidate.external_id)
            }) {
                return Err(Error::Invalid("terminal candidate cannot stage again"));
            }
            continue;
        }
        if write.disposition == Disposition::SkippedReady {
            let ready: bool = sqlx::query_scalar(&format!(
                "SELECT {} FROM catalog.source_items si WHERE si.id=$1 AND si.source_id=$2",
                poster_ready()
            ))
            .bind(write.source_item_id)
            .bind(run.source_id)
            .fetch_one(&mut **tx)
            .await?;
            if !ready {
                return Err(Error::Invalid("poster is not ready"));
            }
        }
        if write.disposition == Disposition::Staged {
            let art = write
                .source_artwork_id
                .as_ref()
                .ok_or(Error::Invalid("staged candidate artwork"))?;
            if !page.artwork_inputs.iter().any(|input| {
                input.reference.source_artwork_id == *art
                    && input.reference.item_external_id.as_ref() == Some(&candidate.external_id)
                    && input.reference.kind == "poster"
            }) {
                return Err(Error::Invalid("candidate missing atomic artwork input"));
            }
        } else if write.source_artwork_id.is_some() {
            return Err(Error::Invalid("non-staged candidate artwork"));
        }
        if matches!(
            write.disposition,
            Disposition::Failed | Disposition::Unavailable
        ) && write.code.is_none()
        {
            return Err(Error::Invalid("candidate requires safe outcome code"));
        }
        if candidate.state != state {
            sqlx::query("UPDATE ingest.maintenance_candidates SET state=$4,code=$5,source_artwork_id=$6,updated_at=clock_timestamp() WHERE run_id=$1 AND source_id=$2 AND source_item_id=$3")
                .bind(run.id).bind(run.source_id).bind(write.source_item_id).bind(state).bind(code).bind(&write.source_artwork_id).execute(&mut **tx).await?;
            progress |= write.disposition != Disposition::Checked;
        }
    }
    for input in &page.artwork_inputs {
        if input.reference.kind != "poster"
            || input.reference.person_external_id.is_some()
            || !writes.candidates.iter().any(|w| {
                w.disposition == Disposition::Staged
                    && w.source_artwork_id.as_ref() == Some(&input.reference.source_artwork_id)
            })
        {
            return Err(Error::Invalid("artwork outside maintenance candidates"));
        }
        let selected: bool = sqlx::query_scalar("SELECT EXISTS(SELECT FROM ingest.maintenance_candidates WHERE run_id=$1 AND source_id=$2 AND external_id=$3 AND source_artwork_id=$4 AND state='staged')")
            .bind(run.id).bind(run.source_id).bind(&input.reference.item_external_id).bind(&input.reference.source_artwork_id).fetch_one(&mut **tx).await?;
        if !selected {
            return Err(Error::Invalid("artwork item outside staged candidates"));
        }
    }
    if matches!(page.completion, ingest::Completion::Complete { .. }) {
        let pending: bool = sqlx::query_scalar("SELECT EXISTS(SELECT FROM ingest.maintenance_candidates WHERE run_id=$1 AND state IN ('selected','checked'))").bind(run.id).fetch_one(&mut **tx).await?;
        if pending {
            return Err(Error::Invalid("maintenance enumeration incomplete"));
        }
    }
    Ok(progress)
}
