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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyWrite {
    pub enabled: bool,
    pub selection_json: Value,
    pub max_items: i64,
    pub max_requests: i64,
    pub incremental_seconds: i32,
    pub reconcile_seconds: i32,
    pub enrich_metadata: bool,
    pub enrich_people: bool,
    pub enrich_artwork: bool,
}
impl PolicyWrite {
    pub(crate) fn validate(&self) -> Result<()> {
        if !(1..=1_000_000).contains(&self.max_items)
            || !(1..=100_000).contains(&self.max_requests)
            || self.incremental_seconds < 1
            || self.reconcile_seconds < 1
            || self.selection_json.to_string().len() > 32768
            || !self.selection_json.is_object()
        {
            return Err(Error::Invalid("policy"));
        }
        if self.selection_json.get("library_ids").is_some() {
            selected_library_ids(&self.selection_json)?;
        } else {
            selected_feed_keys(&self.selection_json)?;
        }
        Ok(())
    }
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
}
pub(crate) async fn insert_policy(
    tx: &mut Transaction<'_, Postgres>,
    source: Uuid,
    p: &PolicyWrite,
) -> Result<Policy> {
    Ok(sqlx::query_as("INSERT INTO ingest.policies(source_id,enabled,selection_json,max_items,max_requests,incremental_seconds,reconcile_seconds,enrich_metadata,enrich_people,enrich_artwork) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) RETURNING *")
        .bind(source).bind(p.enabled).bind(&p.selection_json).bind(p.max_items).bind(p.max_requests).bind(p.incremental_seconds).bind(p.reconcile_seconds).bind(p.enrich_metadata).bind(p.enrich_people).bind(p.enrich_artwork).fetch_one(&mut **tx).await?)
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
pub struct FeedMembershipWrite {
    pub feed_key: String,
    pub external_id: String,
    pub ordinal: i32,
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
    /// Caller supplies only the adapter's successful full selected-library evidence.
    SelectedLibrariesComplete {
        library_ids: Vec<String>,
        evidence: Value,
    },
    /// Replaces only this bounded feed membership; never removes a catalog title.
    FeedWindowComplete {
        feed_keys: Vec<String>,
        evidence: Value,
    },
    Complete {
        evidence: Value,
    },
}
#[derive(Debug, Clone)]
pub struct PageCommit {
    pub libraries: Vec<LibraryWrite>,
    pub items: Vec<ItemWrite>,
    pub feeds: Vec<FeedMembershipWrite>,
    pub artwork_inputs: Vec<crate::media::ArtworkInputWrite>,
    pub artwork_omissions: Vec<crate::media::ArtworkOmission>,
    pub followups: Vec<Followup>,
    pub checkpoint: CheckpointWrite,
    pub requests: i64,
    pub completion: Completion,
}

#[derive(Clone)]
pub struct IngestRepository {
    pool: PgPool,
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
    pub async fn create_policy(&self, source: Uuid, p: &PolicyWrite) -> Result<Policy> {
        p.validate()?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT id FROM catalog.sources WHERE id=$1 FOR UPDATE")
            .bind(source)
            .execute(&mut *tx)
            .await?;
        let policy = insert_policy(&mut tx, source, p).await?;
        tx.commit().await?;
        Ok(policy)
    }
    pub async fn update_policy(
        &self,
        id: Uuid,
        p: &PolicyWrite,
        start: bool,
    ) -> Result<(Policy, Option<Run>)> {
        p.validate()?;
        let mut tx = self.pool.begin().await?;
        lock_policy_source(&mut tx, id).await?;
        let policy: Policy=sqlx::query_as("UPDATE ingest.policies SET version=version+1,enabled=$2,selection_json=$3,max_items=$4,max_requests=$5,incremental_seconds=$6,reconcile_seconds=$7,enrich_metadata=$8,enrich_people=$9,enrich_artwork=$10,next_due_at=now() WHERE id=$1 RETURNING *")
            .bind(id).bind(p.enabled).bind(&p.selection_json).bind(p.max_items).bind(p.max_requests).bind(p.incremental_seconds).bind(p.reconcile_seconds).bind(p.enrich_metadata).bind(p.enrich_people).bind(p.enrich_artwork).fetch_optional(&mut *tx).await?.ok_or(Error::NotFound)?;
        sqlx::query("UPDATE ingest.runs SET state='cancelled',finished_at=now() WHERE policy_id=$1 AND state IN ('running','paused')").bind(id).execute(&mut *tx).await?;
        sqlx::query("UPDATE ingest.jobs j SET state='cancelled',lease_owner=NULL,lease_until=NULL FROM ingest.runs r WHERE j.run_id=r.id AND r.policy_id=$1 AND j.state IN ('ready','leased')").bind(id).execute(&mut *tx).await?;
        let run = if start {
            Some(start_locked(&mut tx, &policy).await?)
        } else {
            None
        };
        tx.commit().await?;
        Ok((policy, run))
    }
    pub async fn start_run(&self, id: Uuid) -> Result<Run> {
        let mut tx = self.pool.begin().await?;
        lock_policy_source(&mut tx, id).await?;
        let policy: Policy = sqlx::query_as("SELECT * FROM ingest.policies WHERE id=$1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(Error::NotFound)?;
        let run = start_locked(&mut tx, &policy).await?;
        tx.commit().await?;
        Ok(run)
    }
    pub async fn due_policies(&self, limit: u32) -> Result<Vec<Policy>> {
        if !(1..=100).contains(&limit) {
            return Err(Error::Invalid("limit"));
        }
        Ok(sqlx::query_as("SELECT p.* FROM ingest.policies p JOIN catalog.sources s ON s.id=p.source_id WHERE p.enabled AND s.state='enabled' AND p.next_due_at<=now() AND NOT EXISTS(SELECT FROM ingest.runs r WHERE r.source_id=p.source_id AND r.state IN ('running','paused')) ORDER BY p.next_due_at,p.id LIMIT $1").bind(i64::from(limit)).fetch_all(&self.pool).await?)
    }
    pub async fn run(&self, id: Uuid) -> Result<Option<Run>> {
        Ok(sqlx::query_as("SELECT * FROM ingest.runs WHERE id=$1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?)
    }
    pub async fn cancel_run(&self, id: Uuid) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE ingest.runs SET state='cancelled',finished_at=now() WHERE id=$1 AND state IN ('running','paused')").bind(id).execute(&mut *tx).await?;
        sqlx::query("UPDATE ingest.jobs SET state='cancelled',lease_owner=NULL,lease_until=NULL WHERE run_id=$1 AND state IN ('ready','leased')").bind(id).execute(&mut *tx).await?;
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
        if run.requests == 0 {
            return Ok(None);
        }
        self.checkpoint(run.policy_id, scope, run.policy_version)
            .await
    }
    pub async fn claim(&self, owner: &str, lease_seconds: i32) -> Result<Option<Claim>> {
        if owner.is_empty() || owner.len() > 200 || !(1..=300).contains(&lease_seconds) {
            return Err(Error::Invalid("lease"));
        }
        self.retire_exhausted().await?;
        Ok(sqlx::query_as("WITH candidate AS (SELECT j.id FROM ingest.jobs j JOIN ingest.runs r ON r.id=j.run_id JOIN ingest.policies p ON p.id=r.policy_id JOIN catalog.sources s ON s.id=p.source_id WHERE ((j.state='ready' AND j.available_at<=clock_timestamp()) OR (j.state='leased' AND j.lease_until<clock_timestamp())) AND j.attempts<j.max_attempts AND p.enabled AND p.version=r.policy_version AND s.state='enabled' AND r.state IN ('running','complete') ORDER BY j.available_at,j.id FOR UPDATE OF j SKIP LOCKED LIMIT 1) UPDATE ingest.jobs j SET state='leased',lease_owner=$1,lease_until=clock_timestamp()+make_interval(secs=>$2),fence=fence+1,attempts=attempts+1 FROM candidate c WHERE j.id=c.id RETURNING j.id,j.run_id,j.kind,j.input,j.lease_owner,j.fence,j.attempts,j.lease_until")
            .bind(owner).bind(lease_seconds).fetch_optional(&self.pool).await?)
    }
    async fn retire_exhausted(&self) -> Result<()> {
        // Bound maintenance per claim; terminal failed runs also repair pre-existing dedupe blockers.
        let candidates: Vec<(Uuid, Uuid, Uuid)> = sqlx::query_as("SELECT r.source_id,r.policy_id,r.id FROM ingest.runs r WHERE r.state IN ('running','complete','failed') AND EXISTS(SELECT FROM ingest.jobs j WHERE j.run_id=r.id AND ((j.state='leased' AND j.lease_until<clock_timestamp() AND j.attempts>=j.max_attempts) OR (r.state='failed' AND j.state IN ('ready','leased')) OR (r.state='running' AND j.kind='ingest' AND j.state='failed'))) ORDER BY r.source_id,r.policy_id,r.id LIMIT 32")
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
            sqlx::query("UPDATE ingest.jobs SET state='failed',lease_owner=NULL,lease_until=NULL,fence=fence+1,error_code='attempts_exhausted' WHERE run_id=$1 AND state='leased' AND lease_until<clock_timestamp() AND attempts>=max_attempts")
                .bind(run).execute(&mut *tx).await?;
            fail_exhausted_run(&mut tx, run).await?;
            tx.commit().await?;
        }
        Ok(())
    }
    pub async fn renew(&self, c: &Claim, seconds: i32) -> Result<()> {
        if !(1..=300).contains(&seconds) {
            return Err(Error::Invalid("lease"));
        }
        let n=sqlx::query("UPDATE ingest.jobs SET lease_until=clock_timestamp()+make_interval(secs=>$4) WHERE id=$1 AND lease_owner=$2 AND fence=$3 AND state='leased' AND lease_until>clock_timestamp()")
            .bind(c.id).bind(&c.lease_owner).bind(c.fence).bind(seconds).execute(&self.pool).await?.rows_affected();
        if n != 1 {
            return Err(Error::Stale);
        }
        Ok(())
    }
    pub async fn retry(&self, c: &Claim, error_code: &str, delay_seconds: i32) -> Result<()> {
        if error_code.is_empty()
            || error_code.len() > 64
            || !error_code
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b == b'_')
            || !(0..=86400).contains(&delay_seconds)
        {
            return Err(Error::Invalid("safe retry"));
        }
        let mut tx = self.pool.begin().await?;
        let _ = guard(&mut tx, c).await?;
        sqlx::query("UPDATE ingest.jobs SET state=CASE WHEN attempts>=max_attempts THEN 'failed' ELSE 'ready' END,available_at=clock_timestamp()+make_interval(secs=>$2),lease_owner=NULL,lease_until=NULL,fence=fence+CASE WHEN attempts>=max_attempts THEN 1 ELSE 0 END,error_code=$3 WHERE id=$1")
            .bind(c.id).bind(delay_seconds).bind(error_code).execute(&mut *tx).await?;
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
        let n=sqlx::query("UPDATE ingest.runs r SET state='running',budget_items=0,budget_requests=0 FROM ingest.policies p,catalog.sources s WHERE r.id=$1 AND r.policy_id=p.id AND p.source_id=s.id AND p.version=r.policy_version AND p.enabled AND s.state='enabled' AND r.state='paused'")
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
    pub async fn commit_page(&self, c: &Claim, page: &PageCommit) -> Result<()> {
        if page.items.len() > 1000
            || page.followups.len() > 2000
            || page.artwork_inputs.len() > 2000
            || page.artwork_omissions.len() > 2000
            || page.requests < 0
            || page.requests > 1000
            || page.checkpoint.scope_key.is_empty()
            || page.checkpoint.scope_key.len() > 300
            || page.checkpoint.cursor.to_string().len() > 32768
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
        if c.kind != "ingest" || run.state != "running" {
            return Err(Error::Stale);
        }
        if run.budget_items + page.items.len() as i64 > p.max_items
            || run.budget_requests + page.requests > p.max_requests
        {
            return Err(Error::Invalid("budget exceeded; pause before request"));
        }
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
        }
        for feed in &page.feeds {
            if !selected_feed_keys(&run.selection_snapshot)?.contains(&feed.feed_key) {
                return Err(Error::Invalid("feed outside policy selection"));
            }
            let n=sqlx::query("INSERT INTO ingest.feed_memberships(source_id,feed_key,source_item_id,ordinal,last_seen_run_id) SELECT $1,$2,id,$4,$5 FROM catalog.source_items WHERE source_id=$1 AND external_id=$3 ON CONFLICT(source_id,feed_key,source_item_id) DO UPDATE SET ordinal=EXCLUDED.ordinal,last_seen_run_id=EXCLUDED.last_seen_run_id")
                .bind(p.source_id).bind(&feed.feed_key).bind(&feed.external_id).bind(feed.ordinal).bind(run.id).execute(&mut *tx).await?.rows_affected();
            if n != 1 {
                return Err(Error::Invalid("unknown feed item"));
            }
        }
        for omission in &page.artwork_omissions {
            crate::media::write_omission(&mut tx, run.id, omission).await?;
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
        sqlx::query("INSERT INTO ingest.checkpoints(source_id,scope_key,policy_version,cursor,watermark) VALUES($1,$2,$3,$4,$5) ON CONFLICT(source_id,scope_key,policy_version) DO UPDATE SET cursor=EXCLUDED.cursor,watermark=EXCLUDED.watermark,updated_at=now()")
            .bind(p.source_id).bind(checkpoint_scope(p.id,&page.checkpoint.scope_key)).bind(p.version).bind(&page.checkpoint.cursor).bind(&page.checkpoint.watermark).execute(&mut *tx).await?;
        let (state, evidence) = match &page.completion {
            Completion::Continue => ("running", json!({})),
            Completion::Pause => ("paused", json!({"budget_paused":true})),
            Completion::SelectedLibrariesComplete {
                library_ids,
                evidence,
            } => {
                let mut selected = selected_library_ids(&run.selection_snapshot)?;
                let mut actual = library_ids.clone();
                selected.sort();
                selected.dedup();
                actual.sort();
                actual.dedup();
                if actual.is_empty() || selected != actual || library_ids.len() != actual.len() {
                    return Err(Error::Invalid(
                        "reconciliation must match exact run selection",
                    ));
                }
                let provider: String =
                    sqlx::query_scalar("SELECT provider FROM catalog.sources WHERE id=$1")
                        .bind(p.source_id)
                        .fetch_one(&mut *tx)
                        .await?;
                if provider != "jellyfin" {
                    return Err(Error::Invalid(
                        "only authoritative library traversal reconciles absence",
                    ));
                }
                sqlx::query("UPDATE catalog.library_items li SET state='absent' FROM catalog.libraries l WHERE li.library_id=l.id AND l.source_id=$1 AND l.public_id=ANY($2) AND li.last_seen_run_id<>$3")
                    .bind(p.source_id).bind(&actual).bind(run.id).execute(&mut *tx).await?;
                ("complete", evidence.clone())
            }
            Completion::FeedWindowComplete {
                feed_keys,
                evidence,
            } => {
                let provider: String =
                    sqlx::query_scalar("SELECT provider FROM catalog.sources WHERE id=$1")
                        .bind(p.source_id)
                        .fetch_one(&mut *tx)
                        .await?;
                let mut configured = selected_feed_keys(&run.selection_snapshot)?;
                let mut actual = feed_keys.clone();
                configured.sort();
                actual.sort();
                actual.dedup();
                if provider != "tmdb" || configured != actual || actual.len() != feed_keys.len() {
                    return Err(Error::Invalid(
                        "feed completion must match exact configured windows",
                    ));
                }
                sqlx::query("DELETE FROM ingest.feed_memberships WHERE source_id=$1 AND feed_key=ANY($2) AND last_seen_run_id<>$3").bind(p.source_id).bind(feed_keys).bind(run.id).execute(&mut *tx).await?;
                ("complete", evidence.clone())
            }
            Completion::Complete { evidence } => ("complete", evidence.clone()),
        };
        sqlx::query("UPDATE ingest.runs SET items=items+$2,requests=requests+$3,budget_items=budget_items+$2,budget_requests=budget_requests+$3,state=$4,coverage=$5,finished_at=CASE WHEN $4='complete' THEN now() ELSE NULL END WHERE id=$1")
            .bind(run.id).bind(page.items.len() as i64).bind(page.requests).bind(state).bind(evidence).execute(&mut *tx).await?;
        if state == "complete" {
            sqlx::query("UPDATE ingest.policies SET next_due_at=now()+make_interval(secs=>incremental_seconds) WHERE id=$1").bind(p.id).execute(&mut *tx).await?;
        }
        if state == "running" {
            let n=sqlx::query("UPDATE ingest.jobs SET state='ready',available_at=now(),lease_owner=NULL,lease_until=NULL,attempts=0 WHERE id=$1 AND lease_owner=$2 AND fence=$3 AND lease_until>clock_timestamp()")
                .bind(c.id).bind(&c.lease_owner).bind(c.fence).execute(&mut *tx).await?.rows_affected();
            if n != 1 {
                return Err(Error::Stale);
            }
        } else {
            finish(&mut tx, c).await?;
        }
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
/// Feed entries are strings or objects with a stable `feed_key` (or unique `feed`) string.
/// Include kind/window/locale in feed_key when configuring multiple windows of one feed.
pub fn selected_feed_keys(selection: &Value) -> Result<Vec<String>> {
    let feeds = selection
        .get("feeds")
        .and_then(Value::as_array)
        .ok_or(Error::Invalid("feed selection"))?;
    let keys = feeds
        .iter()
        .map(|v| {
            v.as_str()
                .or_else(|| v.get("feed_key").and_then(Value::as_str))
                .or_else(|| v.get("feed").and_then(Value::as_str))
                .map(str::to_owned)
                .ok_or(Error::Invalid("feed key"))
        })
        .collect::<Result<Vec<_>>>()?;
    validate_selection_keys(&keys)?;
    Ok(keys)
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

async fn lock_policy_source(tx: &mut Transaction<'_, Postgres>, policy_id: Uuid) -> Result<()> {
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

async fn start_locked(tx: &mut Transaction<'_, Postgres>, p: &Policy) -> Result<Run> {
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
        return Ok(run);
    }
    let snapshot = serde_json::to_value(p).map_err(|_| Error::Invalid("policy"))?;
    let run:Run=sqlx::query_as("INSERT INTO ingest.runs(policy_id,policy_version,policy_snapshot,selection_snapshot,source_id) VALUES($1,$2,$3,$4,$5) RETURNING *")
        .bind(p.id).bind(p.version).bind(snapshot).bind(&p.selection_json).bind(p.source_id).fetch_one(&mut **tx).await?;
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
    sqlx::query("UPDATE ingest.runs r SET state='failed',finished_at=now() WHERE r.id=$1 AND r.state='running' AND EXISTS(SELECT FROM ingest.jobs j WHERE j.run_id=r.id AND j.kind='ingest' AND j.state='failed')")
        .bind(run).execute(&mut **tx).await?;
    sqlx::query("UPDATE ingest.jobs j SET state='cancelled',lease_owner=NULL,lease_until=NULL,fence=fence+1,error_code='run_failed' FROM ingest.runs r WHERE r.id=$1 AND j.run_id=r.id AND r.state='failed' AND j.state IN ('ready','leased')")
        .bind(run).execute(&mut **tx).await?;
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
    sqlx::query("INSERT INTO ingest.jobs(run_id,kind,dedupe_key,input) VALUES($1,$2,$3,$4) ON CONFLICT(kind,dedupe_key) WHERE state IN ('ready','leased') DO NOTHING")
        .bind(run).bind(j.kind.as_str()).bind(&j.dedupe_key).bind(&j.input).execute(&mut **tx).await?;
    Ok(())
}

pub(crate) async fn guard(tx: &mut Transaction<'_, Postgres>, c: &Claim) -> Result<(Run, Policy)> {
    let run_id = c.run_id.ok_or(Error::Stale)?;
    // Source, policy, run, job is the shared operator/worker lock order.
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
    let p:Policy=sqlx::query_as("SELECT p.* FROM ingest.policies p JOIN ingest.runs r ON r.policy_id=p.id WHERE r.id=$1 FOR UPDATE OF p").bind(run_id).fetch_optional(&mut **tx).await?.ok_or(Error::Stale)?;
    let run: Run = sqlx::query_as("SELECT * FROM ingest.runs WHERE id=$1 FOR UPDATE")
        .bind(run_id)
        .fetch_one(&mut **tx)
        .await?;
    if !p.enabled
        || !enabled
        || p.version != run.policy_version
        || !matches!(run.state.as_str(), "running" | "complete")
    {
        return Err(Error::Stale);
    }
    let valid:Option<Uuid>=sqlx::query_scalar("SELECT id FROM ingest.jobs WHERE id=$1 AND run_id=$4 AND kind=$5 AND state='leased' AND lease_owner=$2 AND fence=$3 AND lease_until>clock_timestamp() FOR UPDATE")
        .bind(c.id).bind(&c.lease_owner).bind(c.fence).bind(run_id).bind(&c.kind).fetch_optional(&mut **tx).await?;
    if valid.is_none() {
        return Err(Error::Stale);
    }
    Ok((run, p))
}
pub(crate) async fn finish(tx: &mut Transaction<'_, Postgres>, c: &Claim) -> Result<()> {
    let n=sqlx::query("UPDATE ingest.jobs SET state='complete',lease_owner=NULL,lease_until=NULL,error_code=NULL WHERE id=$1 AND lease_owner=$2 AND fence=$3 AND state='leased' AND lease_until>clock_timestamp()")
        .bind(c.id).bind(&c.lease_owner).bind(c.fence).execute(&mut **tx).await?.rows_affected();
    if n != 1 {
        return Err(Error::Stale);
    }
    Ok(())
}
