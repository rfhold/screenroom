//! Transactional PostgreSQL transport. Callers own scope admission and effect idempotency.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("queue database: {0}")]
    Database(#[from] sqlx::Error),
    #[error("invalid queue input")]
    Invalid,
    #[error("stale queue lease or terminal identity")]
    Stale,
    #[error("dedupe identity has different ownership or payload")]
    Conflict,
}
pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, FromRow)]
pub struct Claim {
    pub id: Uuid,
    pub scope_id: Option<Uuid>,
    pub kind: String,
    pub input: Value,
    pub lease_owner: String,
    pub fence: i64,
    pub attempts: i32,
    pub lease_until: DateTime<Utc>,
}

pub struct Enqueue<'a> {
    pub queue: &'a str,
    pub scope_id: Uuid,
    pub kind: &'a str,
    pub dedupe_key: &'a str,
    pub input: &'a Value,
    pub requirements: &'a [String],
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnqueueResult {
    Inserted(Uuid),
    Existing(Uuid),
}
impl EnqueueResult {
    pub fn id(self) -> Uuid {
        match self {
            Self::Inserted(id) | Self::Existing(id) => id,
        }
    }
}

fn token(s: &str, max: usize) -> bool {
    !s.is_empty()
        && s.len() <= max
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_:-.,/".contains(&b))
}
pub async fn enqueue(tx: &mut Transaction<'_, Postgres>, j: &Enqueue<'_>) -> Result<EnqueueResult> {
    if !token(j.queue, 100)
        || !token(j.kind, 100)
        || j.dedupe_key.is_empty()
        || j.dedupe_key.len() > 1000
        || j.input.to_string().len() > 32768
        || j.requirements.len() > 100
        || j.requirements.iter().any(|r| !token(r, 300))
    {
        return Err(Error::Invalid);
    }
    let inserted: Option<Uuid> = sqlx::query_scalar("INSERT INTO queue.jobs(queue_name,scope_id,kind,dedupe_key,input,requirements) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(queue_name,scope_id,kind,dedupe_key) WHERE state IN ('ready','leased') DO NOTHING RETURNING id")
        .bind(j.queue).bind(j.scope_id).bind(j.kind).bind(j.dedupe_key).bind(j.input).bind(j.requirements).fetch_optional(&mut **tx).await?;
    if let Some(id) = inserted {
        return Ok(EnqueueResult::Inserted(id));
    }
    let existing: Option<Uuid> = sqlx::query_scalar("SELECT id FROM queue.jobs WHERE queue_name=$1 AND scope_id=$2 AND kind=$3 AND dedupe_key=$4 AND state IN ('ready','leased') AND input=$5 AND requirements=$6 FOR SHARE")
        .bind(j.queue).bind(j.scope_id).bind(j.kind).bind(j.dedupe_key).bind(j.input).bind(j.requirements).fetch_optional(&mut **tx).await?;
    existing.map(EnqueueResult::Existing).ok_or(Error::Conflict)
}

/// `scopes` is the caller's admitted set in this transaction. Requirements use exact tokens.
pub async fn claim(
    tx: &mut Transaction<'_, Postgres>,
    queue: &str,
    scopes: &[Uuid],
    capabilities: &[String],
    owner: &str,
    seconds: i32,
) -> Result<Option<Claim>> {
    if owner.is_empty() || owner.len() > 200 || !(1..=300).contains(&seconds) || !token(queue, 100)
    {
        return Err(Error::Invalid);
    }
    let c: Option<Claim> = sqlx::query_as("WITH candidate AS (SELECT id FROM queue.jobs WHERE queue_name=$1 AND scope_id=ANY($2) AND requirements<@$3::text[] AND ((state='ready' AND available_at<=clock_timestamp()) OR (state='leased' AND lease_until<clock_timestamp())) AND attempts<max_attempts ORDER BY available_at,id FOR UPDATE SKIP LOCKED LIMIT 1) UPDATE queue.jobs j SET state='leased',lease_owner=$4,lease_until=clock_timestamp()+make_interval(secs=>$5),fence=fence+1,attempts=attempts+1 FROM candidate c WHERE j.id=c.id RETURNING j.id,j.scope_id,j.kind,j.input,j.lease_owner,j.fence,j.attempts,j.lease_until")
        .bind(queue).bind(scopes).bind(capabilities).bind(owner).bind(seconds).fetch_optional(&mut **tx).await?;
    if let Some(c) = &c {
        sqlx::query("UPDATE queue.attempts SET finished_at=clock_timestamp(),outcome='lease_expired',error_code='lease_expired' WHERE job_id=$1 AND finished_at IS NULL")
            .bind(c.id).execute(&mut **tx).await?;
        sqlx::query("INSERT INTO queue.attempts(job_id,fence,owner,generation) SELECT id,$2,$3,redrive_generation FROM queue.jobs WHERE id=$1")
            .bind(c.id).bind(c.fence).bind(&c.lease_owner).execute(&mut **tx).await?;
    }
    Ok(c)
}

pub async fn guard(tx: &mut Transaction<'_, Postgres>, c: &Claim) -> Result<()> {
    let id: Option<Uuid> = sqlx::query_scalar("SELECT id FROM queue.jobs WHERE id=$1 AND lease_owner=$2 AND fence=$3 AND state='leased' AND lease_until>clock_timestamp() AND scope_id IS NOT DISTINCT FROM $4 AND kind=$5 FOR UPDATE")
        .bind(c.id).bind(&c.lease_owner).bind(c.fence).bind(c.scope_id).bind(&c.kind).fetch_optional(&mut **tx).await?;
    if id.is_none() {
        return Err(Error::Stale);
    }
    Ok(())
}
pub async fn renew(tx: &mut Transaction<'_, Postgres>, c: &Claim, seconds: i32) -> Result<()> {
    if !(1..=300).contains(&seconds) {
        return Err(Error::Invalid);
    }
    guard(tx, c).await?;
    sqlx::query(
        "UPDATE queue.jobs SET lease_until=clock_timestamp()+make_interval(secs=>$2) WHERE id=$1",
    )
    .bind(c.id)
    .bind(seconds)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
async fn end_attempt(
    tx: &mut Transaction<'_, Postgres>,
    c: &Claim,
    outcome: &str,
    code: Option<&str>,
) -> Result<()> {
    sqlx::query("UPDATE queue.attempts SET finished_at=clock_timestamp(),outcome=$3,error_code=$4 WHERE job_id=$1 AND fence=$2 AND finished_at IS NULL")
        .bind(c.id).bind(c.fence).bind(outcome).bind(code).execute(&mut **tx).await?;
    Ok(())
}
pub async fn ack(tx: &mut Transaction<'_, Postgres>, c: &Claim) -> Result<()> {
    guard(tx, c).await?;
    end_attempt(tx, c, "complete", None).await?;
    sqlx::query("UPDATE queue.jobs SET state='complete',lease_owner=NULL,lease_until=NULL,error_code=NULL WHERE id=$1").bind(c.id).execute(&mut **tx).await?;
    Ok(())
}
/// A successful durable work commit may start a fresh consecutive-failure tranche.
pub async fn release(tx: &mut Transaction<'_, Postgres>, c: &Claim, progress: bool) -> Result<()> {
    guard(tx, c).await?;
    end_attempt(tx, c, "released", None).await?;
    sqlx::query("UPDATE queue.jobs SET state='ready',available_at=clock_timestamp(),lease_owner=NULL,lease_until=NULL,fence=fence+1,attempts=CASE WHEN $2 THEN 0 ELSE attempts END,error_code=NULL WHERE id=$1").bind(c.id).bind(progress).execute(&mut **tx).await?;
    Ok(())
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    Permanent,
    Transient,
    Throttled,
    Stale,
}

/// Bounded exponential backoff with stable per-attempt jitter and bounded Retry-After.
pub fn retry_delay(id: Uuid, attempts: i32, retry_after: Option<i32>) -> i32 {
    RetryPolicy::default().delay(id, attempts, retry_after)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RetryPolicy {
    pub max_attempts: i32,
    pub base_seconds: i32,
    pub cap_seconds: i32,
    pub jitter_percent: i32,
}
impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            base_seconds: 5,
            cap_seconds: 3600,
            jitter_percent: 25,
        }
    }
}
impl RetryPolicy {
    pub fn validate(&self) -> Result<()> {
        if !(1..=20).contains(&self.max_attempts)
            || !(1..=300).contains(&self.base_seconds)
            || !(self.base_seconds..=86400).contains(&self.cap_seconds)
            || !(0..=100).contains(&self.jitter_percent)
        {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    pub fn delay(&self, id: Uuid, attempts: i32, retry_after: Option<i32>) -> i32 {
        let exponential = self
            .base_seconds
            .saturating_mul(1_i32 << attempts.saturating_sub(1).clamp(0, 10))
            .min(self.cap_seconds);
        let jitter = (id.as_u128().wrapping_add(attempts.max(0) as u128)
            % ((i64::from(exponential) * i64::from(self.jitter_percent) / 100) as u128 + 1))
            as i32;
        (exponential + jitter)
            .min(self.cap_seconds)
            .max(retry_after.unwrap_or(0).clamp(0, 86400))
            .min(86400)
    }
}
pub async fn configure_retry(
    tx: &mut Transaction<'_, Postgres>,
    job: Uuid,
    policy: RetryPolicy,
) -> Result<()> {
    policy.validate()?;
    let changed = sqlx::query("UPDATE queue.jobs SET max_attempts=$2,retry_base_seconds=$3,retry_cap_seconds=$4,retry_jitter_percent=$5 WHERE id=$1 AND state='ready' AND attempts=0")
        .bind(job).bind(policy.max_attempts).bind(policy.base_seconds).bind(policy.cap_seconds).bind(policy.jitter_percent).execute(&mut **tx).await?.rows_affected();
    if changed != 1 {
        return Err(Error::Stale);
    }
    Ok(())
}
pub async fn fail(
    tx: &mut Transaction<'_, Postgres>,
    c: &Claim,
    class: FailureClass,
    code: &str,
    retry_after: Option<i32>,
) -> Result<()> {
    if !token(code, 64) || !code.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') {
        return Err(Error::Invalid);
    }
    if class == FailureClass::Stale {
        return Err(Error::Stale);
    }
    guard(tx, c).await?;
    let exhausted: bool =
        sqlx::query_scalar("SELECT attempts>=max_attempts FROM queue.jobs WHERE id=$1")
            .bind(c.id)
            .fetch_one(&mut **tx)
            .await?;
    let terminal = class == FailureClass::Permanent || exhausted;
    let (base_seconds,cap_seconds,jitter_percent): (i32,i32,i32) = sqlx::query_as("SELECT retry_base_seconds,retry_cap_seconds,retry_jitter_percent FROM queue.jobs WHERE id=$1").bind(c.id).fetch_one(&mut **tx).await?;
    let policy = RetryPolicy {
        max_attempts: 1,
        base_seconds,
        cap_seconds,
        jitter_percent,
    };
    end_attempt(
        tx,
        c,
        if class == FailureClass::Permanent {
            "permanent"
        } else if exhausted {
            "exhausted"
        } else if class == FailureClass::Throttled {
            "throttled"
        } else {
            "retryable"
        },
        Some(code),
    )
    .await?;
    sqlx::query("UPDATE queue.jobs SET state=CASE WHEN $2 OR attempts>=max_attempts THEN 'failed' ELSE 'ready' END,available_at=clock_timestamp()+make_interval(secs=>$3),lease_owner=NULL,lease_until=NULL,fence=fence+1,error_code=$4 WHERE id=$1")
        .bind(c.id).bind(terminal).bind(policy.delay(c.id,c.attempts,retry_after)).bind(code).execute(&mut **tx).await?;
    Ok(())
}

/// Cancels or revokes leases within a caller-locked scope, preserving attempt history.
pub async fn cancel_scope(
    tx: &mut Transaction<'_, Postgres>,
    queue: &str,
    scope: Uuid,
    pause: bool,
) -> Result<()> {
    let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM queue.jobs WHERE queue_name=$1 AND scope_id=$2 AND state IN ('ready','leased') FOR UPDATE").bind(queue).bind(scope).fetch_all(&mut **tx).await?;
    sqlx::query("UPDATE queue.attempts SET finished_at=clock_timestamp(),outcome=$2,error_code=$2 WHERE job_id=ANY($1) AND finished_at IS NULL").bind(&ids).bind(if pause {"paused"} else {"cancelled"}).execute(&mut **tx).await?;
    sqlx::query("UPDATE queue.jobs SET state=CASE WHEN $2 THEN 'ready' ELSE 'cancelled' END,lease_owner=NULL,lease_until=NULL,fence=fence+1 WHERE id=ANY($1)").bind(ids).bind(pause).execute(&mut **tx).await?;
    Ok(())
}
pub async fn retire_exhausted(
    tx: &mut Transaction<'_, Postgres>,
    queue: &str,
    scope: Uuid,
) -> Result<()> {
    let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM queue.jobs WHERE queue_name=$1 AND scope_id=$2 AND state IN ('ready','leased') AND (state='ready' OR lease_until<clock_timestamp()) AND attempts>=max_attempts FOR UPDATE").bind(queue).bind(scope).fetch_all(&mut **tx).await?;
    sqlx::query("UPDATE queue.attempts SET finished_at=clock_timestamp(),outcome='exhausted',error_code='attempts_exhausted' WHERE job_id=ANY($1) AND finished_at IS NULL").bind(&ids).execute(&mut **tx).await?;
    sqlx::query("UPDATE queue.jobs SET state='failed',lease_owner=NULL,lease_until=NULL,fence=fence+1,error_code='attempts_exhausted' WHERE id=ANY($1)").bind(ids).execute(&mut **tx).await?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct RedriveResult {
    pub request_id: Uuid,
    pub job_id: Uuid,
    pub generation: i32,
}
pub async fn redrive_receipt(
    tx: &mut Transaction<'_, Postgres>,
    job: Uuid,
    request: Uuid,
) -> Result<Option<RedriveResult>> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(request.to_string())
        .execute(&mut **tx)
        .await?;
    let r = sqlx::query_as::<_, RedriveResult>(
        "SELECT request_id,job_id,generation FROM queue.redrives WHERE request_id=$1",
    )
    .bind(request)
    .fetch_optional(&mut **tx)
    .await?;
    if r.as_ref().is_some_and(|r| r.job_id != job) {
        return Err(Error::Conflict);
    }
    Ok(r)
}
/// Caller must validate the current payload and domain scope before calling. Identity is retained.
pub async fn redrive(
    tx: &mut Transaction<'_, Postgres>,
    job: Uuid,
    request: Uuid,
) -> Result<RedriveResult> {
    if let Some(r) = redrive_receipt(tx, job, request).await? {
        return Ok(r);
    }
    let generation: Option<i32> = sqlx::query_scalar("UPDATE queue.jobs SET state='ready',attempts=0,error_code=NULL,available_at=clock_timestamp(),fence=fence+1,redrive_generation=redrive_generation+1 WHERE id=$1 AND state='failed' RETURNING redrive_generation").bind(job).fetch_optional(&mut **tx).await?;
    let generation = generation.ok_or(Error::Stale)?;
    sqlx::query("INSERT INTO queue.redrives(request_id,job_id,generation) VALUES($1,$2,$3)")
        .bind(request)
        .bind(job)
        .bind(generation)
        .execute(&mut **tx)
        .await?;
    Ok(RedriveResult {
        request_id: request,
        job_id: job,
        generation,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Attempt {
    pub job_id: Uuid,
    pub fence: i64,
    pub generation: i32,
    pub owner: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub outcome: String,
    pub error_code: Option<String>,
}
pub async fn attempts(
    tx: &mut Transaction<'_, Postgres>,
    job: Uuid,
    after_fence: i64,
    limit: u32,
) -> Result<Vec<Attempt>> {
    if after_fence < 0 || !(1..=200).contains(&limit) {
        return Err(Error::Invalid);
    }
    Ok(sqlx::query_as(
        "SELECT * FROM queue.attempts WHERE job_id=$1 AND fence>$2 ORDER BY fence LIMIT $3",
    )
    .bind(job)
    .bind(after_fence)
    .bind(i64::from(limit))
    .fetch_all(&mut **tx)
    .await?)
}
pub async fn redrives(
    tx: &mut Transaction<'_, Postgres>,
    job: Uuid,
    after_generation: i32,
    limit: u32,
) -> Result<Vec<RedriveResult>> {
    if after_generation < 0 || !(1..=200).contains(&limit) {
        return Err(Error::Invalid);
    }
    Ok(sqlx::query_as("SELECT request_id,job_id,generation FROM queue.redrives WHERE job_id=$1 AND generation>$2 ORDER BY generation LIMIT $3").bind(job).bind(after_generation).bind(i64::from(limit)).fetch_all(&mut **tx).await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configured_backoff_is_effective_and_validated() {
        let p = RetryPolicy {
            max_attempts: 3,
            base_seconds: 10,
            cap_seconds: 25,
            jitter_percent: 0,
        };
        p.validate().unwrap();
        assert_eq!(
            (
                p.delay(Uuid::nil(), 1, None),
                p.delay(Uuid::nil(), 2, None),
                p.delay(Uuid::nil(), 3, None)
            ),
            (10, 20, 25)
        );
        assert_eq!(p.delay(Uuid::nil(), 100, Some(60)), 60);
        let mut invalid = p;
        invalid.jitter_percent = 101;
        assert!(invalid.validate().is_err());
        invalid = p;
        invalid.cap_seconds = 9;
        assert!(invalid.validate().is_err());
        invalid = p;
        invalid.max_attempts = 21;
        assert!(invalid.validate().is_err());
        let jitter = RetryPolicy {
            jitter_percent: 100,
            ..p
        };
        for i in 0..100 {
            assert!((10..=20).contains(&jitter.delay(Uuid::from_u128(i), 1, None)));
            assert_eq!(jitter.delay(Uuid::from_u128(i), 10, None), 25);
        }
    }
    #[test]
    fn backoff_is_bounded_and_honors_retry_after() {
        let id = Uuid::nil();
        assert!((5..=7).contains(&retry_delay(id, 1, None)));
        assert_eq!(retry_delay(id, 1, Some(600)), 600);
        assert_eq!(retry_delay(id, 100, Some(i32::MAX)), 86400);
        for attempt in -2..100 {
            assert!((5..=4500).contains(&retry_delay(id, attempt, None)));
        }
    }
    #[test]
    fn sanitized_codes_cannot_contain_urls_or_credentials() {
        assert!(token("upstream_unavailable", 64));
        assert!(!token("https://host?token=secret", 64));
        assert!(!token("bad\ninput", 64));
    }
}
