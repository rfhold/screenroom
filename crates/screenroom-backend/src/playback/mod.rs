//! Private durable playback authority. No provider I/O takes place in this module.
pub mod secrets;

use crate::{Error, Result, Uuid, catalog::Scope};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Connection, FromRow, PgConnection, PgPool, Postgres, Transaction};

pub const LEASE_SECONDS: i32 = 90;
pub const ABSOLUTE_SECONDS: i32 = 12 * 60 * 60;
pub const PAUSED_SECONDS: i32 = 30 * 60;
const RESOURCE_CAPACITY: usize = 8192;
const ACTIVE: &[&str] = &["preparing", "ready", "playing", "paused", "stopping"];

pub async fn try_gateway_lock(
    connection: &mut PgConnection,
) -> std::result::Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT pg_try_advisory_lock(1935897197,1)")
        .fetch_one(connection)
        .await
}

pub async fn holds_gateway_lock(
    connection: &mut PgConnection,
) -> std::result::Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS(SELECT FROM pg_locks WHERE locktype='advisory' AND pid=pg_backend_pid() AND classid=1935897197 AND objid=1 AND objsubid=2 AND granted)")
        .fetch_one(connection)
        .await
}

async fn begin_gateway(connection: &mut PgConnection) -> Result<Transaction<'_, Postgres>> {
    let mut tx = connection.begin().await?;
    sqlx::query(
        "SELECT set_config('lock_timeout','2s',true),set_config('statement_timeout','10s',true)",
    )
    .execute(&mut *tx)
    .await?;
    if !holds_gateway_lock(&mut tx).await? {
        return Err(Error::Stale);
    }
    Ok(tx)
}

fn resources_fit(current: usize, additional: usize) -> bool {
    current
        .checked_add(additional)
        .is_some_and(|total| total <= RESOURCE_CAPACITY)
}

fn generate_device_id() -> Result<String> {
    Ok(format!(
        "screenroom-playback-{}",
        secrets::Capability::generate()?.expose()
    ))
}

fn conflict() -> Error {
    Error::Invalid("operation_conflict")
}

#[derive(Clone, Copy)]
pub struct Limits {
    pub per_profile: i64,
    pub installation: i64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            per_profile: 2,
            installation: 8,
        }
    }
}

#[derive(Clone, FromRow)]
pub struct ResolvedItem {
    pub user_id: Uuid,
    pub public_item_id: String,
    pub source_item_id: Uuid,
    pub source_id: Uuid,
    pub source_key: String,
    pub external_item_id: String,
    pub kind: String,
    pub connection_id: Uuid,
    pub connection_generation: i64,
    pub binding_revision: String,
    pub remote_profile_id: String,
}

#[derive(Clone, FromRow)]
pub struct Session {
    pub id: Uuid,
    pub user_id: Uuid,
    pub public_item_id: String,
    pub source_item_id: Uuid,
    pub source_id: Uuid,
    pub source_key: String,
    pub scope: Value,
    pub external_item_id: String,
    pub connection_id: Uuid,
    pub connection_generation: i64,
    pub binding_revision: String,
    pub remote_profile_id: String,
    pub device_id: String,
    pub play_session_id: Option<String>,
    pub credential_ref: Uuid,
    pub replay_ref: Uuid,
    pub control_hash: Vec<u8>,
    pub media_hash: Option<Vec<u8>>,
    pub state: String,
    pub transport_revision: i64,
    pub last_sequence: i64,
    pub capabilities: Value,
    pub tracks: Value,
    pub transport: Option<Value>,
    pub lease_until: DateTime<Utc>,
    pub absolute_until: DateTime<Utc>,
    pub paused_at: Option<DateTime<Utc>>,
}

/// Receipt view only: no provider identity, authority hashes, token, or capability material.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReceiptSnapshot {
    pub state: String,
    pub transport_revision: i64,
    pub last_sequence: i64,
    pub tracks: Value,
    pub transport: Option<Value>,
    pub replay_ref: Uuid,
    pub lease_until: DateTime<Utc>,
    pub absolute_until: DateTime<Utc>,
}

impl ReceiptSnapshot {
    pub fn restore_view(self, session: &mut Session) {
        session.state = self.state;
        session.transport_revision = self.transport_revision;
        session.last_sequence = self.last_sequence;
        session.tracks = self.tracks;
        session.transport = self.transport;
        session.replay_ref = self.replay_ref;
        session.lease_until = self.lease_until;
        session.absolute_until = self.absolute_until;
    }
}

#[derive(Clone, FromRow)]
pub struct Operation {
    pub id: Uuid,
    pub session_id: Uuid,
    pub user_id: Uuid,
    pub binding_revision: String,
    pub kind: String,
    pub request_id: Option<Uuid>,
    pub sequence: Option<i64>,
    pub transport_revision: i64,
    pub body: Value,
    pub replay_ref: Option<Uuid>,
    pub status: String,
    pub claim: Uuid,
    pub dispatched: bool,
    pub result: Option<Value>,
}

pub enum Reservation {
    New(Operation),
    Replay(Operation),
    /// Older completed events are ignored, never dispatched again.
    Older {
        last_sequence: u32,
    },
}

pub struct Create<'a> {
    pub user_id: Uuid,
    pub public_item_id: &'a str,
    pub scope: &'a Scope,
    pub request_id: Uuid,
    pub normalized_body: &'a Value,
    pub capabilities: &'a Value,
    pub control_hash: &'a [u8],
}

#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Started,
    Progress,
    Paused,
    Resumed,
    Seeked,
    Heartbeat,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventBody {
    pub kind: EventKind,
    pub position_ms: String,
    pub paused: bool,
    pub muted: bool,
    pub volume: Option<u32>,
}

impl EventBody {
    pub fn normalized(&self) -> Result<Value> {
        if self.position_ms.is_empty()
            || !self.position_ms.bytes().all(|b| b.is_ascii_digit())
            || self.position_ms.parse::<u64>().is_err()
            || self.volume.is_some_and(|volume| volume > 100)
        {
            return Err(Error::Invalid("playback event"));
        }
        let mut normalized = self.clone();
        normalized.position_ms = self.position_ms.parse::<u64>().unwrap().to_string();
        serde_json::to_value(normalized).map_err(|_| Error::Invalid("playback event"))
    }
}

pub struct Created {
    pub session: Session,
    pub operation: Operation,
    pub replay: bool,
}

pub struct SessionView {
    pub session: Session,
    pub latest_operation: Option<Operation>,
}

pub struct Authority<'a> {
    pub user_id: Uuid,
    pub session_id: Uuid,
    pub control_capability: &'a str,
}

#[derive(Clone, Copy)]
pub enum Mutation {
    Update {
        request_id: Uuid,
    },
    Event {
        sequence: u32,
    },
    Stop {
        request_id: Uuid,
        final_sequence: u32,
    },
}

impl Mutation {
    fn parts(self) -> (&'static str, Option<Uuid>, Option<i64>) {
        match self {
            Self::Update { request_id } => ("update", Some(request_id), None),
            Self::Event { sequence } => ("event", None, Some(i64::from(sequence))),
            Self::Stop { request_id, .. } => ("stop", Some(request_id), None),
        }
    }
}

pub enum Outcome<'a> {
    Confirmed(&'a Value),
    /// Only use when evidence establishes that no remote effect occurred.
    NotSent(&'a Value),
    Uncertain,
}

/// Reads, revocation, timeout, grace periods and exhausted retry budgets are not evidence.
pub enum CompletionEvidence {
    RemoteCompletion,
    ControlledQuiescence,
}

#[derive(Clone, FromRow)]
pub struct Resource {
    pub id: Uuid,
    pub session_id: Uuid,
    pub transport_revision: i64,
    pub source_item_id: Uuid,
    pub media_source_id: String,
    pub private_mapping: Value,
}

pub struct Transport<'a> {
    pub play_session_id: &'a str,
    pub media_hash: &'a [u8],
    pub tracks: &'a Value,
    /// Cap-free descriptor only: omit capability-bearing URLs. Reconstruct those
    /// transiently from replay material. Upstream locators belong in resources.
    pub descriptor: &'a Value,
    pub resources: &'a [Resource],
}

#[derive(Clone, FromRow)]
pub struct Cleanup {
    pub id: Uuid,
    pub session_id: Uuid,
    pub device_id: String,
    pub play_session_id: Option<String>,
    pub credential_ref: Uuid,
    pub revoke_device: bool,
    pub attempts: i32,
    pub claim: Option<Uuid>,
}

#[derive(Clone)]
pub struct PlaybackRepository {
    pool: PgPool,
    limits: Limits,
}

async fn lock_user(tx: &mut Transaction<'_, Postgres>, user: Uuid) -> Result<String> {
    let revision: Option<String> = sqlx::query_scalar("SELECT playback.lock_user($1)")
        .bind(user)
        .fetch_one(&mut **tx)
        .await?;
    revision.ok_or(Error::Stale)
}

async fn locked_session(tx: &mut Transaction<'_, Postgres>, id: Uuid) -> Result<Session> {
    sqlx::query_as("SELECT * FROM playback.sessions WHERE id=$1 FOR UPDATE")
        .bind(id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or(Error::NotFound)
}

fn bounded_json(value: &Value, max: usize) -> Result<()> {
    if serde_json::to_vec(value)
        .map_err(|_| Error::Invalid("playback input"))?
        .len()
        > max
    {
        return Err(Error::Invalid("playback input"));
    }
    Ok(())
}

fn valid_live(session: &Session, revision: &str, now: DateTime<Utc>) -> bool {
    session.binding_revision == revision
        && ACTIVE.contains(&session.state.as_str())
        && session.state != "stopping"
        && lifetime_valid(
            session.lease_until,
            session.absolute_until,
            session.paused_at,
            now,
        )
}

fn lifetime_valid(
    lease: DateTime<Utc>,
    absolute: DateTime<Utc>,
    paused: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> bool {
    lease > now
        && absolute > now
        && paused.is_none_or(|t| t + chrono::Duration::seconds(i64::from(PAUSED_SECONDS)) > now)
}

fn check_control(session: &Session, authority: &Authority<'_>) -> Result<()> {
    if session.user_id != authority.user_id
        || !constant_eq(
            &session.control_hash,
            &secrets::capability_hash(authority.control_capability),
        )
    {
        return Err(Error::NotFound);
    }
    Ok(())
}

fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b)
        .fold(0u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

#[derive(sqlx::FromRow)]
pub struct PlaybackTelemetry {
    pub active: i64,
    pub pending: i64,
    pub claimed: i64,
    pub failed: i64,
    pub blocked_device: i64,
    pub oldest_failed_seconds: f64,
    pub identity_changed: i64,
    pub attempts_exhausted: i64,
    pub upstream_unavailable: i64,
}

impl PlaybackRepository {
    pub async fn telemetry_counts(&self) -> Result<PlaybackTelemetry> {
        sqlx::query_as(r#"
            SELECT (SELECT count(*) FROM playback.sessions
                WHERE state IN ('preparing','ready','playing','paused')
                AND lease_until>statement_timestamp() AND absolute_until>statement_timestamp()) AS active,
                count(*) FILTER(WHERE c.state='pending' AND
                    (NOT c.revoke_device OR s.state NOT IN ('preparing','ready','playing','paused'))) AS pending,
                count(*) FILTER(WHERE c.state='claimed') AS claimed,
                count(*) FILTER(WHERE c.state='failed') AS failed,
                count(*) FILTER(WHERE c.state='pending' AND c.revoke_device AND EXISTS(
                    SELECT FROM playback.cleanup older WHERE older.session_id=c.session_id
                        AND NOT older.revoke_device AND older.state='failed')) AS blocked_device,
                COALESCE(EXTRACT(epoch FROM statement_timestamp()-min(c.created_at)
                    FILTER(WHERE c.state='failed')),0)::float8 AS oldest_failed_seconds,
                count(*) FILTER(WHERE c.state='failed' AND c.error_code='identity_changed') AS identity_changed,
                count(*) FILTER(WHERE c.state='failed' AND c.error_code='attempts_exhausted') AS attempts_exhausted,
                count(*) FILTER(WHERE c.state='failed' AND c.error_code='upstream_unavailable') AS upstream_unavailable
            FROM playback.cleanup c JOIN playback.sessions s ON s.id=c.session_id
        "#)
            .fetch_one(&self.pool).await.map_err(Into::into)
    }

    pub async fn cleanup_status(&self, session: Uuid) -> Result<(bool, bool)> {
        sqlx::query_as("SELECT EXISTS(SELECT FROM playback.cleanup c JOIN playback.sessions s ON s.id=c.session_id WHERE c.session_id=$1 AND c.state<>'complete' AND (NOT c.revoke_device OR s.state NOT IN ('preparing','ready','playing','paused'))),EXISTS(SELECT FROM playback.cleanup WHERE session_id=$1 AND state='failed')")
            .bind(session).fetch_one(&self.pool).await.map_err(Into::into)
    }
    pub async fn operation_receipt(&self, operation: &Operation) -> Result<Operation> {
        sqlx::query_as("SELECT * FROM playback.operations WHERE id=$1 AND claim=$2")
            .bind(operation.id)
            .bind(operation.claim)
            .fetch_one(&self.pool)
            .await
            .map_err(Into::into)
    }

    /// Gateway-only receipt variant, preserving the legacy heartbeat API.
    pub async fn heartbeat_recorded(
        &self,
        authority: &Authority<'_>,
        sequence: u32,
        revision: u32,
        event: &EventBody,
    ) -> Result<Reservation> {
        if event.kind != EventKind::Heartbeat {
            return Err(Error::Invalid("heartbeat"));
        }
        let mut reservation = self
            .reserve_event(authority, sequence, revision, event)
            .await?;
        if let Reservation::New(operation) = &mut reservation {
            self.local_heartbeat(operation, event.paused, true).await?;
            *operation = self.operation_receipt(operation).await?;
        }
        Ok(reservation)
    }

    /// Finalizes a task that returned, panicked, or reached its local deadline.
    /// The durable dispatch bit, never cancellation itself, determines uncertainty.
    pub async fn finalize_abandoned(&self, operation: &Operation) -> Result<()> {
        let current: Operation = sqlx::query_as("SELECT * FROM playback.operations WHERE id=$1")
            .bind(operation.id)
            .fetch_one(&self.pool)
            .await?;
        if current.claim != operation.claim || current.status != "pending" {
            return Ok(());
        }
        self.finish_failure_recorded(operation, 12).await
    }

    /// Safe failure metadata does not constitute evidence or resolve an unknown effect.
    pub async fn finish_failure_recorded(&self, operation: &Operation, code: i32) -> Result<()> {
        if !(1..=12).contains(&code) {
            return Err(Error::Invalid("playback failure"));
        }
        let current = self.operation_receipt(operation).await?;
        let result = serde_json::json!({"failure":code});
        if current.dispatched {
            self.finish_internal(operation, Outcome::Uncertain, "failed", true, Some(&result))
                .await
        } else {
            self.finish_internal(operation, Outcome::NotSent(&result), "failed", true, None)
                .await
        }
    }

    /// Private orchestration facts; receipts remain the authority across transport replacement.
    pub async fn orchestration_history(&self, operation: &Operation) -> Result<(bool, bool)> {
        sqlx::query_as("SELECT EXISTS(SELECT FROM playback.operations WHERE session_id=$1 AND id<>$2 AND kind='event' AND body->>'kind'='started' AND status IN ('confirmed','uncertain')),EXISTS(SELECT FROM playback.operations WHERE session_id=$1 AND id<>$2 AND kind='update' AND body->>'compatible_transcode_fallback'='true')")
            .bind(operation.session_id).bind(operation.id).fetch_one(&self.pool).await.map_err(Into::into)
    }

    /// Constructs private transport configuration only; dispatch guards own all network I/O.
    pub async fn provider_configuration(
        &self,
        session: &Session,
        credentials: &crate::connections::CredentialDirectory,
    ) -> Result<(screenroom_catalog_adapter::jellyfin::LiveJellyfin, String)> {
        use screenroom_catalog_adapter::jellyfin::{ApiKey, ConnectionConfig, LiveJellyfin};
        let row: Option<(String, String, Uuid)> = sqlx::query_as("SELECT j.base_url,j.server_id,j.credential_ref FROM connections.jellyfin j JOIN catalog.sources s ON s.id=j.source_id WHERE j.id=$1 AND j.source_id=$2 AND j.generation=$3 AND s.source_key=$4 AND s.configuration->>'connection_id'=j.id::text AND s.configuration->>'connection_generation'=j.generation::text AND s.configuration->>'locator_revision'=j.revision")
            .bind(session.connection_id).bind(session.source_id).bind(session.connection_generation).bind(&session.source_key).fetch_optional(&self.pool).await?;
        let (url, server, reference) = row.ok_or(Error::Stale)?;
        let key = credentials.read(reference)?;
        if url.contains(&key) {
            return Err(Error::Connection("invalid_connection"));
        }
        let provider = LiveJellyfin::from_api_key(
            ConnectionConfig {
                source_key: session.source_key.clone(),
                base_url: url,
                timeout_ms: 3000,
                max_response_bytes: 1_048_576,
            },
            ApiKey::new(key).map_err(|_| Error::Connection("invalid_credentials"))?,
        )
        .map_err(|_| Error::Connection("invalid_connection"))?;
        Ok((provider, server))
    }

    /// Registers adapter-validated children under the parent's exact transport scope.
    /// The caller retains the typed reference privately; client input cannot mint mappings.
    pub async fn register_media_child(
        &self,
        session_id: Uuid,
        revision: u32,
        capability: &str,
        parent_id: Uuid,
        child_id: Uuid,
        private_mapping: &Value,
    ) -> Result<()> {
        bounded_json(private_mapping, 16384)?;
        let user: Uuid = sqlx::query_scalar("SELECT user_id FROM playback.sessions WHERE id=$1")
            .bind(session_id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(Error::NotFound)?;
        let mut tx = self.pool.begin().await?;
        let binding = lock_user(&mut tx, user).await?;
        let session = locked_session(&mut tx, session_id).await?;
        require_fences(&mut tx, &session, &binding).await?;
        if !["ready", "playing", "paused"].contains(&session.state.as_str())
            || session.transport_revision != i64::from(revision)
            || !session
                .media_hash
                .as_ref()
                .is_some_and(|h| constant_eq(h, &secrets::capability_hash(capability)))
        {
            return Err(Error::Stale);
        }
        let source: Option<String> = sqlx::query_scalar("SELECT media_source_id FROM playback.resources WHERE id=$1 AND session_id=$2 AND transport_revision=$3 AND source_item_id=$4")
            .bind(parent_id).bind(session_id).bind(i64::from(revision)).bind(session.source_item_id).fetch_optional(&mut *tx).await?;
        let source = source.ok_or(Error::NotFound)?;
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM playback.resources WHERE session_id=$1 AND transport_revision=$2",
        )
        .bind(session_id)
        .bind(i64::from(revision))
        .fetch_one(&mut *tx)
        .await?;
        let count = usize::try_from(count).map_err(|_| Error::Invalid("resource capacity"))?;
        if !resources_fit(count, 1) {
            return Err(Error::Invalid("resource capacity"));
        }
        sqlx::query("INSERT INTO playback.resources(id,session_id,transport_revision,source_item_id,media_source_id,private_mapping) VALUES($1,$2,$3,$4,$5,$6)")
            .bind(child_id).bind(session_id).bind(i64::from(revision)).bind(session.source_item_id).bind(source).bind(private_mapping).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(())
    }

    pub fn new(pool: PgPool, limits: Limits) -> Result<Self> {
        if limits.per_profile < 1 || limits.installation < 1 || limits.installation > 1000 {
            return Err(Error::Invalid("playback capacity"));
        }
        Ok(Self { pool, limits })
    }

    /// Unlike catalog precedence, ambiguity is rejected: playback never switches sources.
    pub async fn resolve(
        &self,
        user: Uuid,
        public_id: &str,
        scope: &Scope,
    ) -> Result<ResolvedItem> {
        let mut tx = self.pool.begin().await?;
        lock_user(&mut tx, user).await?;
        let item = resolve_in(&mut tx, user, public_id, scope).await?;
        tx.commit().await?;
        Ok(item)
    }

    /// Commit the random device identity, private references and cleanup intent first.
    /// Only then publish replay material / Quick Connect secret or authorize the device.
    pub async fn create(&self, input: Create<'_>) -> Result<Created> {
        self.create_in(self.pool.begin().await?, input).await
    }

    pub async fn create_owned(
        &self,
        connection: &mut PgConnection,
        input: Create<'_>,
    ) -> Result<Created> {
        self.create_in(begin_gateway(connection).await?, input)
            .await
    }

    async fn create_in(
        &self,
        mut tx: Transaction<'_, Postgres>,
        input: Create<'_>,
    ) -> Result<Created> {
        input.scope.validate()?;
        bounded_json(input.normalized_body, 65536)?;
        bounded_json(input.capabilities, 16384)?;
        if input.control_hash.len() != 32 {
            return Err(Error::Invalid("capability hash"));
        }
        let mut scope = input.scope.clone();
        for values in [
            &mut scope.source_keys,
            &mut scope.library_ids,
            &mut scope.providers,
            &mut scope.kinds,
        ] {
            values.sort();
            values.dedup();
        }
        let body = serde_json::json!({"item_id":input.public_item_id,"scope":scope,"capabilities":input.capabilities,"request":input.normalized_body});
        bounded_json(&body, 65536)?;
        let revision = lock_user(&mut tx, input.user_id).await?;
        if let Some(operation) = sqlx::query_as::<_, Operation>("SELECT * FROM playback.operations WHERE user_id=$1 AND binding_revision=$2 AND kind='create' AND request_id=$3")
            .bind(input.user_id).bind(&revision).bind(input.request_id).fetch_optional(&mut *tx).await? {
            if operation.body != body { return Err(conflict()); }
            let session = locked_session(&mut tx, operation.session_id).await?;
            tx.commit().await?;
            return Ok(Created { session, operation, replay: true });
        }
        let item = resolve_in(&mut tx, input.user_id, input.public_item_id, input.scope).await?;
        // One installation-wide lock makes profile and installation capacity atomic.
        sqlx::query("SELECT pg_advisory_xact_lock(1935897197,2)")
            .execute(&mut *tx)
            .await?;
        let (total, profile, duplicate): (i64,i64,bool) = sqlx::query_as("SELECT count(*),count(*) FILTER(WHERE connection_id=$1 AND remote_profile_id=$2),COALESCE(bool_or(connection_id=$1 AND remote_profile_id=$2 AND external_item_id=$3),false) FROM playback.sessions WHERE state IN ('preparing','ready','playing','paused','stopping')")
            .bind(item.connection_id).bind(&item.remote_profile_id).bind(&item.external_item_id).fetch_one(&mut *tx).await?;
        if duplicate {
            return Err(Error::Invalid("playback_conflict"));
        }
        if total >= self.limits.installation || profile >= self.limits.per_profile {
            return Err(Error::Invalid("capacity_exhausted"));
        }
        let id = Uuid::new_v4();
        let device = generate_device_id()?;
        let credential = Uuid::new_v4();
        let replay = Uuid::new_v4();
        let mut capabilities = input.capabilities.clone();
        let object = capabilities
            .as_object_mut()
            .ok_or(Error::Invalid("capabilities"))?;
        // Private orchestration metadata is not part of the normalized request.
        object.insert(
            "_quick_connect_ref".into(),
            serde_json::json!(Uuid::new_v4()),
        );
        bounded_json(&capabilities, 16384)?;
        let scope = serde_json::to_value(&scope).map_err(|_| Error::Invalid("scope"))?;
        sqlx::query("INSERT INTO playback.sessions(id,user_id,public_item_id,source_item_id,source_id,source_key,scope,external_item_id,connection_id,connection_generation,binding_revision,remote_profile_id,device_id,credential_ref,replay_ref,control_hash,state,capabilities) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,'preparing',$17)")
            .bind(id).bind(item.user_id).bind(&item.public_item_id).bind(item.source_item_id).bind(item.source_id).bind(&item.source_key).bind(scope).bind(&item.external_item_id).bind(item.connection_id).bind(item.connection_generation).bind(&item.binding_revision).bind(&item.remote_profile_id).bind(&device).bind(credential).bind(replay).bind(input.control_hash).bind(capabilities).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO playback.cleanup(id,session_id,device_id,credential_ref,revoke_device,available_at) VALUES($1,$2,$3,$4,true,'infinity')")
            .bind(Uuid::new_v4()).bind(id).bind(&device).bind(credential).execute(&mut *tx).await?;
        let operation = insert_operation(
            &mut tx,
            id,
            item.user_id,
            &revision,
            "create",
            Some(input.request_id),
            None,
            0,
            &body,
            Some(replay),
        )
        .await?;
        let session = locked_session(&mut tx, id).await?;
        tx.commit().await?;
        Ok(Created {
            session,
            operation,
            replay: false,
        })
    }

    pub async fn get(&self, authority: &Authority<'_>) -> Result<Session> {
        let mut tx = self.pool.begin().await?;
        let revision = lock_user(&mut tx, authority.user_id).await?;
        let mut session = locked_session(&mut tx, authority.session_id).await?;
        check_control(&session, authority)?;
        require_binding(&mut tx, &session, &revision).await?;
        if ACTIVE.contains(&session.state.as_str())
            && !lifetime_valid(
                session.lease_until,
                session.absolute_until,
                session.paused_at,
                Utc::now(),
            )
        {
            abandon_operations(&mut tx, session.id).await?;
            revoke_in(&mut tx, session.id, "expired").await?;
            session = locked_session(&mut tx, session.id).await?;
        }
        tx.commit().await?;
        Ok(session)
    }

    pub async fn view(&self, authority: &Authority<'_>) -> Result<SessionView> {
        let session = self.get(authority).await?;
        let latest_operation=sqlx::query_as("SELECT * FROM playback.operations WHERE session_id=$1 ORDER BY created_at DESC,id DESC LIMIT 1")
            .bind(session.id).fetch_optional(&self.pool).await?;
        Ok(SessionView {
            session,
            latest_operation,
        })
    }

    pub async fn reserve_event(
        &self,
        authority: &Authority<'_>,
        sequence: u32,
        revision: u32,
        event: &EventBody,
    ) -> Result<Reservation> {
        self.reserve(
            authority,
            Mutation::Event { sequence },
            revision,
            &event.normalized()?,
            event.kind != EventKind::Heartbeat,
        )
        .await
    }

    /// Pure local heartbeat: no provider report and no personal-state effect.
    /// Use reserve_event + before_io instead when a transcode ping is necessary.
    pub async fn heartbeat(
        &self,
        authority: &Authority<'_>,
        sequence: u32,
        revision: u32,
        event: &EventBody,
    ) -> Result<Reservation> {
        if event.kind != EventKind::Heartbeat {
            return Err(Error::Invalid("heartbeat"));
        }
        let mut reservation = self
            .reserve_event(authority, sequence, revision, event)
            .await?;
        if let Reservation::New(operation) = &mut reservation {
            self.complete_local_heartbeat(operation, event.paused)
                .await?;
            operation.status = "confirmed".into();
            operation.result = Some(serde_json::json!({}));
        }
        Ok(reservation)
    }

    async fn complete_local_heartbeat(&self, operation: &Operation, paused: bool) -> Result<()> {
        self.local_heartbeat(operation, paused, false).await
    }

    async fn local_heartbeat(
        &self,
        operation: &Operation,
        paused: bool,
        recorded: bool,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let revision = lock_user(&mut tx, operation.user_id).await?;
        let session = locked_session(&mut tx, operation.session_id).await?;
        require_fences(&mut tx, &session, &revision).await?;
        let updated=sqlx::query("UPDATE playback.operations SET status='confirmed',result='{}',finished_at=clock_timestamp() WHERE id=$1 AND claim=$2 AND NOT dispatched AND status='pending'")
            .bind(operation.id).bind(operation.claim).execute(&mut *tx).await?.rows_affected();
        if updated != 1 {
            return Err(Error::Stale);
        }
        sqlx::query("UPDATE playback.sessions SET lease_until=LEAST(absolute_until,clock_timestamp()+make_interval(secs=>$2::int)),state=$3,paused_at=CASE WHEN $3='paused' THEN COALESCE(paused_at,clock_timestamp()) ELSE NULL END,updated_at=clock_timestamp() WHERE id=$1")
            .bind(session.id).bind(LEASE_SECONDS).bind(if paused {"paused"} else {"playing"}).execute(&mut *tx).await?;
        if recorded {
            record_snapshot(&mut tx, operation, &serde_json::json!({})).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Reserve before any provider I/O. `personal_effect` is false only for pure
    /// negotiation, local heartbeat, or transcode ping (never a start/report/stop).
    pub async fn reserve(
        &self,
        authority: &Authority<'_>,
        mutation: Mutation,
        transport_revision: u32,
        normalized_body: &Value,
        personal_effect: bool,
    ) -> Result<Reservation> {
        bounded_json(normalized_body, 65536)?;
        let (kind, request, sequence) = mutation.parts();
        let personal_effect = match mutation {
            Mutation::Stop { .. } => true,
            Mutation::Event { .. } => {
                normalized_body.get("kind").and_then(Value::as_str) != Some("heartbeat")
            }
            Mutation::Update { .. } => personal_effect,
        };
        let stop_body;
        let normalized_body = if let Mutation::Stop { final_sequence, .. } = mutation {
            stop_body =
                serde_json::json!({"input":normalized_body,"final_sequence":final_sequence});
            &stop_body
        } else {
            normalized_body
        };
        if sequence == Some(0) {
            return Err(Error::Invalid("event sequence"));
        }
        let mut tx = self.pool.begin().await?;
        let revision = lock_user(&mut tx, authority.user_id).await?;
        let session = locked_session(&mut tx, authority.session_id).await?;
        check_control(&session, authority)?;
        let old: Option<Operation> = if let Some(request) = request {
            sqlx::query_as("SELECT * FROM playback.operations WHERE user_id=$1 AND binding_revision=$2 AND kind=$3 AND request_id=$4")
                .bind(session.user_id).bind(&session.binding_revision).bind(kind).bind(request).fetch_optional(&mut *tx).await?
        } else {
            sqlx::query_as("SELECT * FROM playback.operations WHERE session_id=$1 AND sequence=$2")
                .bind(session.id)
                .bind(sequence)
                .fetch_optional(&mut *tx)
                .await?
        };
        if let Some(old) = old {
            if old.session_id != session.id
                || old.body != *normalized_body
                || old.transport_revision != i64::from(transport_revision)
            {
                return Err(conflict());
            }
            tx.commit().await?;
            return Ok(Reservation::Replay(old));
        }
        require_fences(&mut tx, &session, &revision).await?;
        if session.transport_revision != i64::from(transport_revision) {
            return Err(Error::Stale);
        }
        if sequence.is_some_and(|s| s <= session.last_sequence) {
            tx.commit().await?;
            return Ok(Reservation::Older {
                last_sequence: session.last_sequence as u32,
            });
        }
        if let Mutation::Stop { final_sequence, .. } = mutation
            && i64::from(final_sequence) < session.last_sequence
        {
            return Err(Error::Stale);
        }
        let pending: bool = sqlx::query_scalar("SELECT EXISTS(SELECT FROM playback.operations WHERE session_id=$1 AND status IN ('pending','uncertain'))")
            .bind(session.id).fetch_one(&mut *tx).await?;
        if pending {
            return Err(Error::Invalid("operation_pending"));
        }
        let operation = insert_operation(
            &mut tx,
            session.id,
            session.user_id,
            &session.binding_revision,
            kind,
            request,
            sequence,
            i64::from(transport_revision),
            normalized_body,
            if kind == "update" {
                Some(Uuid::new_v4())
            } else {
                None
            },
        )
        .await?;
        if personal_effect {
            reserve_effect(&mut tx, &session, operation.id).await?;
        }
        if let Some(sequence) = sequence {
            sqlx::query("UPDATE playback.sessions SET last_sequence=$2 WHERE id=$1")
                .bind(session.id)
                .bind(sequence)
                .execute(&mut *tx)
                .await?;
        }
        if kind == "stop" {
            if let Mutation::Stop { final_sequence, .. } = mutation {
                sqlx::query("UPDATE playback.sessions SET last_sequence=$2 WHERE id=$1")
                    .bind(session.id)
                    .bind(i64::from(final_sequence))
                    .execute(&mut *tx)
                    .await?;
            }
            revoke_in(&mut tx, session.id, "stopping").await?;
        }
        tx.commit().await?;
        Ok(Reservation::New(operation))
    }

    /// Call immediately before dispatch. A claim is single-use even across processes.
    /// The user lock serializes this local decision with profile mutation; an already
    /// dispatched request cannot be retracted, so stale results are discarded.
    pub async fn before_io(&self, operation: &Operation) -> Result<Session> {
        self.io_authority(operation, true).await
    }

    /// A multi-step authorization/negotiation flow rechecks authority before each
    /// next outbound step. Never use this to retry an ambiguous provider mutation.
    pub async fn revalidate_io(&self, operation: &Operation) -> Result<Session> {
        self.io_authority(operation, false).await
    }

    async fn io_authority(&self, operation: &Operation, first_dispatch: bool) -> Result<Session> {
        let mut tx = self.pool.begin().await?;
        let revision = lock_user(&mut tx, operation.user_id).await?;
        let session = locked_session(&mut tx, operation.session_id).await?;
        if session.user_id != operation.user_id
            || session.binding_revision != operation.binding_revision
        {
            return Err(Error::Stale);
        }
        if operation.kind == "stop" {
            require_binding(&mut tx, &session, &revision).await?;
            if session.state != "stopping"
                || !lifetime_valid(
                    session.lease_until,
                    session.absolute_until,
                    session.paused_at,
                    Utc::now(),
                )
            {
                return Err(Error::Stale);
            }
        } else {
            require_fences(&mut tx, &session, &revision).await?;
        }
        let changed = sqlx::query("UPDATE playback.operations SET dispatched=true WHERE id=$1 AND claim=$2 AND status='pending' AND dispatched=$3 AND session_id=$4 AND user_id=$5 AND binding_revision=$6 AND kind=$7 AND transport_revision=$8")
            .bind(operation.id).bind(operation.claim).bind(!first_dispatch).bind(session.id).bind(session.user_id).bind(&session.binding_revision).bind(&operation.kind).bind(operation.transport_revision).execute(&mut *tx).await?.rows_affected();
        if changed != 1 {
            return Err(Error::Stale);
        }
        tx.commit().await?;
        Ok(session)
    }

    /// Results must omit capabilities and capability-bearing URLs; immutable
    /// private replay files supply that material when constructing responses.
    /// Complete only the matching claim. Uncertain operations are terminal locally,
    /// but their personal freshness fence survives session termination and cleanup.
    pub async fn finish(
        &self,
        operation: &Operation,
        outcome: Outcome<'_>,
        state: &str,
    ) -> Result<()> {
        self.finish_internal(operation, outcome, state, false, None)
            .await
    }

    /// Atomically captures the capability-free session view in the completed receipt.
    pub async fn finish_recorded(
        &self,
        operation: &Operation,
        outcome: Outcome<'_>,
        state: &str,
    ) -> Result<()> {
        self.finish_internal(operation, outcome, state, true, None)
            .await
    }

    pub async fn reject_unpublished_create(
        &self,
        connection: Option<&mut PgConnection>,
        operation: &Operation,
        result: &Value,
    ) -> Result<()> {
        let tx = match connection {
            Some(connection) => begin_gateway(connection).await?,
            None => self.pool.begin().await?,
        };
        self.finish_in(
            tx,
            operation,
            Outcome::NotSent(result),
            "failed",
            true,
            None,
        )
        .await
    }

    async fn finish_internal(
        &self,
        operation: &Operation,
        outcome: Outcome<'_>,
        state: &str,
        recorded: bool,
        uncertain_metadata: Option<&Value>,
    ) -> Result<()> {
        self.finish_in(
            self.pool.begin().await?,
            operation,
            outcome,
            state,
            recorded,
            uncertain_metadata,
        )
        .await
    }

    async fn finish_in(
        &self,
        mut tx: Transaction<'_, Postgres>,
        operation: &Operation,
        outcome: Outcome<'_>,
        state: &str,
        recorded: bool,
        uncertain_metadata: Option<&Value>,
    ) -> Result<()> {
        if ![
            "preparing",
            "ready",
            "playing",
            "paused",
            "stopping",
            "stopped",
            "failed",
        ]
        .contains(&state)
        {
            return Err(Error::Invalid("playback state"));
        }
        // Disabled users still need uncertainty and cleanup evidence recorded.
        let revision: Option<String> = sqlx::query_scalar("SELECT playback.lock_user($1)")
            .bind(operation.user_id)
            .fetch_one(&mut *tx)
            .await?;
        let session = locked_session(&mut tx, operation.session_id).await?;
        let current: Operation =
            sqlx::query_as("SELECT * FROM playback.operations WHERE id=$1 FOR UPDATE")
                .bind(operation.id)
                .fetch_one(&mut *tx)
                .await?;
        if session.user_id != operation.user_id
            || session.binding_revision != operation.binding_revision
        {
            return Err(Error::Stale);
        }
        if current.claim != operation.claim
            || current.status != "pending"
            || current.session_id != session.id
            || current.user_id != session.user_id
            || current.binding_revision != session.binding_revision
            || current.kind != operation.kind
            || current.transport_revision != operation.transport_revision
        {
            return Err(Error::Stale);
        }
        let stale_binding = match revision {
            Some(revision) => match require_binding(&mut tx, &session, &revision).await {
                Ok(()) => false,
                Err(Error::Stale) => true,
                Err(error) => return Err(error),
            },
            None => true,
        };
        let now = Utc::now();
        let stale = stale_binding
            || !ACTIVE.contains(&session.state.as_str())
            || !lifetime_valid(
                session.lease_until,
                session.absolute_until,
                session.paused_at,
                now,
            );
        let (status, result, evidence, effect, new_state) = match outcome {
            Outcome::Confirmed(result) => {
                bounded_json(result, 65536)?;
                if !current.dispatched {
                    return Err(Error::Invalid("operation not dispatched"));
                }
                (
                    "confirmed",
                    Some(result),
                    "remote_completion",
                    "completed",
                    if stale { "stopping" } else { state },
                )
            }
            Outcome::NotSent(result) => {
                bounded_json(result, 65536)?;
                if current.dispatched {
                    return Err(Error::Invalid("dispatched outcome requires evidence"));
                }
                (
                    "failed",
                    Some(result),
                    "not_dispatched",
                    "no_effect",
                    if stale { "stopping" } else { state },
                )
            }
            Outcome::Uncertain => ("uncertain", uncertain_metadata, "", "unknown", "uncertain"),
        };
        sqlx::query("UPDATE playback.operations SET status=$2,result=$3,finished_at=clock_timestamp() WHERE id=$1")
            .bind(operation.id).bind(status).bind(if stale { None } else { result }).execute(&mut *tx).await?;
        let updated = sqlx::query("UPDATE playback.effects SET status=$2,evidence=NULLIF($3,''),updated_at=clock_timestamp() WHERE operation_id=$1")
            .bind(operation.id).bind(effect).bind(evidence).execute(&mut *tx).await?.rows_affected();
        if updated > 0 && effect != "unknown" {
            advance_epoch(&mut tx, &session).await?;
        }
        sqlx::query("UPDATE playback.sessions SET state=$2,lease_until=LEAST(absolute_until,clock_timestamp()+make_interval(secs=>$3::int)),paused_at=CASE WHEN $2='paused' THEN COALESCE(paused_at,clock_timestamp()) ELSE NULL END,updated_at=clock_timestamp() WHERE id=$1")
            .bind(session.id).bind(new_state).bind(LEASE_SECONDS).execute(&mut *tx).await?;
        if ["stopping", "stopped", "failed", "uncertain"].contains(&new_state) {
            revoke_in(&mut tx, session.id, new_state).await?;
        }
        if recorded
            && !stale
            && let Some(result) = result
        {
            record_snapshot(&mut tx, operation, result).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// No read-only reconciliation API releases effects. Call only with separately
    /// established remote completion or controlled-quiescence evidence.
    pub async fn resolve_effect(
        &self,
        operation_id: Uuid,
        evidence: CompletionEvidence,
    ) -> Result<()> {
        let operation: Operation = sqlx::query_as("SELECT * FROM playback.operations WHERE id=$1")
            .bind(operation_id)
            .fetch_one(&self.pool)
            .await?;
        let mut tx = self.pool.begin().await?;
        let _: Option<String> = sqlx::query_scalar("SELECT playback.lock_user($1)")
            .bind(operation.user_id)
            .fetch_one(&mut *tx)
            .await?;
        let session = locked_session(&mut tx, operation.session_id).await?;
        let evidence = match evidence {
            CompletionEvidence::RemoteCompletion => "remote_completion",
            CompletionEvidence::ControlledQuiescence => "controlled_quiescence",
        };
        let updated = sqlx::query("UPDATE playback.effects SET status='completed',evidence=$2,updated_at=clock_timestamp() WHERE operation_id=$1 AND status='unknown'")
            .bind(operation_id).bind(evidence).execute(&mut *tx).await?.rows_affected();
        if updated != 1 {
            return Err(Error::Stale);
        }
        advance_epoch(&mut tx, &session).await?;
        tx.commit().await?;
        Ok(())
    }

    /// Persist negotiated exact identities and mappings before exposing media.
    /// This remains under the pending create/update claim; finish it afterwards.
    pub async fn publish_transport(
        &self,
        operation: &Operation,
        transport: Transport<'_>,
    ) -> Result<u32> {
        if transport.media_hash.len() != 32
            || transport.play_session_id.is_empty()
            || transport.play_session_id.len() > 200
            || !resources_fit(0, transport.resources.len())
        {
            return Err(Error::Invalid("transport"));
        }
        bounded_json(transport.tracks, 65536)?;
        bounded_json(transport.descriptor, 16384)?;
        let mut tx = self.pool.begin().await?;
        let revision = lock_user(&mut tx, operation.user_id).await?;
        let session = locked_session(&mut tx, operation.session_id).await?;
        require_fences(&mut tx, &session, &revision).await?;
        if session.user_id != operation.user_id
            || session.binding_revision != operation.binding_revision
        {
            return Err(Error::Stale);
        }
        let valid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT FROM playback.operations WHERE id=$1 AND claim=$2 AND session_id=$3 AND kind IN ('create','update') AND status='pending' AND dispatched)")
            .bind(operation.id).bind(operation.claim).bind(session.id).fetch_one(&mut *tx).await?;
        if !valid || session.transport_revision != operation.transport_revision {
            return Err(Error::Stale);
        }
        let next = u32::try_from(session.transport_revision)
            .ok()
            .and_then(|v| v.checked_add(1))
            .ok_or(Error::Stale)?;
        if let Some(old) = &session.play_session_id {
            sqlx::query("INSERT INTO playback.cleanup(id,session_id,device_id,play_session_id,credential_ref,revoke_device) VALUES($1,$2,$3,$4,$5,false)")
                .bind(Uuid::new_v4()).bind(session.id).bind(&session.device_id).bind(old).bind(session.credential_ref).execute(&mut *tx).await?;
        }
        sqlx::query("UPDATE playback.sessions SET play_session_id=$2,media_hash=$3,tracks=$4,transport=$5,transport_revision=$6,replay_ref=$7 WHERE id=$1")
            .bind(session.id).bind(transport.play_session_id).bind(transport.media_hash).bind(transport.tracks).bind(transport.descriptor).bind(i64::from(next)).bind(operation.replay_ref.ok_or(Error::Invalid("missing replay reference"))?).execute(&mut *tx).await?;
        sqlx::query(
            "UPDATE playback.cleanup SET play_session_id=$2 WHERE session_id=$1 AND revoke_device",
        )
        .bind(session.id)
        .bind(transport.play_session_id)
        .execute(&mut *tx)
        .await?;
        for resource in transport.resources {
            if resource.session_id != session.id
                || resource.transport_revision != i64::from(next)
                || resource.source_item_id != session.source_item_id
                || resource.media_source_id.is_empty()
                || resource.media_source_id.len() > 200
            {
                return Err(Error::Invalid("resource binding"));
            }
            bounded_json(&resource.private_mapping, 16384)?;
            sqlx::query("INSERT INTO playback.resources(id,session_id,transport_revision,source_item_id,media_source_id,private_mapping) VALUES($1,$2,$3,$4,$5,$6)")
                .bind(resource.id).bind(session.id).bind(i64::from(next)).bind(session.source_item_id).bind(&resource.media_source_id).bind(&resource.private_mapping).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(next)
    }

    /// Reuse at least every five seconds during streaming. Does not renew the lease.
    pub async fn media(
        &self,
        session_id: Uuid,
        revision: u32,
        capability: &str,
        resource: Uuid,
    ) -> Result<(Session, Resource)> {
        let user: Uuid = sqlx::query_scalar("SELECT user_id FROM playback.sessions WHERE id=$1")
            .bind(session_id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(Error::NotFound)?;
        let mut tx = self.pool.begin().await?;
        let binding = lock_user(&mut tx, user).await?;
        let session = locked_session(&mut tx, session_id).await?;
        require_fences(&mut tx, &session, &binding).await?;
        if !["ready", "playing", "paused"].contains(&session.state.as_str())
            || session.transport_revision != i64::from(revision)
            || !session
                .media_hash
                .as_ref()
                .is_some_and(|h| constant_eq(h, &secrets::capability_hash(capability)))
        {
            return Err(Error::Stale);
        }
        let resource = sqlx::query_as("SELECT * FROM playback.resources WHERE id=$1 AND session_id=$2 AND transport_revision=$3 AND source_item_id=$4")
            .bind(resource).bind(session_id).bind(i64::from(revision)).bind(session.source_item_id).fetch_optional(&mut *tx).await?.ok_or(Error::NotFound)?;
        tx.commit().await?;
        Ok((session, resource))
    }

    /// Recover on the lock holder itself. Losing that connection aborts the transaction.
    /// It never resumes provider reports or playback from durable receipts.
    pub async fn recover_abandoned(connection: &mut PgConnection) -> Result<u64> {
        let mut tx = begin_gateway(connection).await?;
        let sessions: Vec<(Uuid,Uuid)> = sqlx::query_as("SELECT s.id,s.user_id FROM playback.sessions s WHERE s.state IN ('preparing','ready','playing','paused','stopping') OR EXISTS(SELECT FROM playback.operations o WHERE o.session_id=s.id AND o.status='pending') ORDER BY s.user_id,s.id")
            .fetch_all(&mut *tx).await?;
        let mut count = 0;
        for (id, user) in sessions {
            let _: Option<String> = sqlx::query_scalar("SELECT playback.lock_user($1)")
                .bind(user)
                .fetch_one(&mut *tx)
                .await?;
            let session = locked_session(&mut tx, id).await?;
            abandon_operations(&mut tx, id).await?;
            if ACTIVE.contains(&session.state.as_str()) {
                revoke_in(&mut tx, id, "expired").await?;
                count += 1;
            }
        }
        sqlx::query("UPDATE playback.cleanup SET available_at=clock_timestamp() WHERE state IN ('pending','claimed') AND revoke_device")
            .execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(count)
    }

    pub async fn transport_retired(&self, session_id: Uuid, revision: u32) -> Result<bool> {
        Ok(sqlx::query_scalar("SELECT NOT EXISTS(SELECT FROM playback.sessions WHERE id=$1 AND transport_revision<=$2 AND state IN ('preparing','ready','playing','paused'))")
            .bind(session_id).bind(i64::from(revision)).fetch_one(&self.pool).await?)
    }

    pub async fn expire(&self) -> Result<u64> {
        let candidates: Vec<(Uuid,Uuid)> = sqlx::query_as("SELECT id,user_id FROM playback.sessions WHERE state IN ('preparing','ready','playing','paused','stopping') AND (lease_until<=clock_timestamp() OR absolute_until<=clock_timestamp() OR paused_at<=clock_timestamp()-make_interval(secs=>$1::int) OR state='stopping') ORDER BY id LIMIT 100")
            .bind(PAUSED_SECONDS).fetch_all(&self.pool).await?;
        let mut count = 0;
        for (id, user) in candidates {
            let mut tx = self.pool.begin().await?;
            let revision: Option<String> = sqlx::query_scalar("SELECT playback.lock_user($1)")
                .bind(user)
                .fetch_one(&mut *tx)
                .await?;
            let session = locked_session(&mut tx, id).await?;
            let pending: bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM playback.operations WHERE session_id=$1 AND status='pending')")
                .bind(id).fetch_one(&mut *tx).await?;
            let now = Utc::now();
            if (session.state == "stopping"
                && (!pending || revision.as_deref() != Some(&session.binding_revision)))
                || session.lease_until <= now
                || session.absolute_until <= now
                || session.paused_at.is_some_and(|t| {
                    t + chrono::Duration::seconds(i64::from(PAUSED_SECONDS)) <= now
                })
            {
                abandon_operations(&mut tx, id).await?;
                revoke_in(&mut tx, id, "expired").await?;
                count += 1;
            }
            tx.commit().await?;
        }
        Ok(count)
    }

    pub async fn claim_cleanup(&self) -> Result<Option<Cleanup>> {
        Self::claim_cleanup_in(self.pool.begin().await?).await
    }

    pub async fn claim_cleanup_owned(connection: &mut PgConnection) -> Result<Option<Cleanup>> {
        Self::claim_cleanup_in(begin_gateway(connection).await?).await
    }

    async fn claim_cleanup_in(mut tx: Transaction<'_, Postgres>) -> Result<Option<Cleanup>> {
        let claim = Uuid::new_v4();
        sqlx::query("UPDATE playback.cleanup SET state='failed',error_code='attempts_exhausted',finished_at=clock_timestamp() WHERE attempts>=5 AND (state='pending' OR (state='claimed' AND claim_until<=clock_timestamp()))")
            .execute(&mut *tx).await?;
        // A device revoke follows every old-encoding cleanup; never revoke a live token.
        let job: Option<Cleanup>=sqlx::query_as("WITH chosen AS (SELECT c.id FROM playback.cleanup c JOIN playback.sessions s ON s.id=c.session_id WHERE c.attempts<5 AND c.available_at<=clock_timestamp() AND (c.state='pending' OR (c.state='claimed' AND c.claim_until<=clock_timestamp())) AND NOT EXISTS(SELECT FROM playback.operations o WHERE o.session_id=c.session_id AND o.status='pending') AND (NOT c.revoke_device OR (s.state NOT IN ('preparing','ready','playing','paused') AND NOT EXISTS(SELECT FROM playback.cleanup old WHERE old.session_id=c.session_id AND NOT old.revoke_device AND old.state<>'complete'))) ORDER BY c.created_at,c.id FOR UPDATE OF c SKIP LOCKED LIMIT 1) UPDATE playback.cleanup c SET state='claimed',claim=$1,claim_until=clock_timestamp()+interval '30 seconds',attempts=attempts+1 FROM chosen WHERE c.id=chosen.id RETURNING c.*")
            .bind(claim).fetch_optional(&mut *tx).await?;
        tx.commit().await?;
        Ok(job)
    }

    pub async fn cleanup_session(&self, job: &Cleanup) -> Result<Session> {
        let session: Session=sqlx::query_as("SELECT s.* FROM playback.sessions s JOIN playback.cleanup c ON c.session_id=s.id JOIN connections.jellyfin j ON j.id=s.connection_id AND j.source_id=s.source_id AND j.generation=s.connection_generation JOIN catalog.sources source ON source.id=s.source_id WHERE c.id=$1 AND c.claim=$2 AND c.state='claimed' AND c.claim_until>clock_timestamp() AND c.device_id=s.device_id AND c.credential_ref=s.credential_ref AND source.source_key=s.source_key AND source.configuration->>'connection_id'=j.id::text AND source.configuration->>'connection_generation'=j.generation::text AND source.configuration->>'locator_revision'=j.revision")
            .bind(job.id).bind(job.claim).fetch_optional(&self.pool).await?.ok_or(Error::Stale)?;
        Ok(session)
    }

    /// Cleanup may run after source/user disablement, but never against a replaced
    /// connection or server. The normal enabled-source resolver stays unchanged.
    pub async fn cleanup_provider(
        &self,
        job: &Cleanup,
        credentials: &crate::connections::CredentialDirectory,
    ) -> Result<screenroom_catalog_adapter::jellyfin::LiveJellyfin> {
        use screenroom_catalog_adapter::jellyfin::{ApiKey, ConnectionConfig, LiveJellyfin};
        let session = self.cleanup_session(job).await?;
        let (url,server,reference):(String,String,Uuid)=sqlx::query_as("SELECT base_url,server_id,credential_ref FROM connections.jellyfin WHERE id=$1 AND generation=$2")
            .bind(session.connection_id).bind(session.connection_generation).fetch_one(&self.pool).await?;
        let key = credentials.read(reference)?;
        if url.contains(&key) {
            return Err(Error::Connection("invalid_connection"));
        }
        let provider = LiveJellyfin::from_api_key(
            ConnectionConfig {
                source_key: session.source_key,
                base_url: url,
                timeout_ms: 10_000,
                max_response_bytes: 1_048_576,
            },
            ApiKey::new(key).map_err(|_| Error::Connection("invalid_credentials"))?,
        )
        .map_err(|_| Error::Connection("invalid_connection"))?;
        let observed = provider
            .server_identity()
            .await
            .map_err(|_| Error::Connection("connection_unavailable"))?;
        if observed.id != server {
            return Err(Error::Connection("server_identity_mismatch"));
        }
        Ok(provider)
    }

    /// The adapter must verify recorded connection generation AND server identity
    /// before exact-device cleanup. A failure stays visible and retains all evidence.
    pub async fn finish_cleanup(
        &self,
        job: &Cleanup,
        success: bool,
        identity_changed: bool,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        let state = if success {
            "complete"
        } else if identity_changed || job.attempts >= 5 {
            "failed"
        } else {
            "pending"
        };
        let changed=sqlx::query("UPDATE playback.cleanup SET state=$3,claim=NULL,claim_until=NULL,available_at=clock_timestamp()+interval '30 seconds',error_code=$4,finished_at=CASE WHEN $3 IN ('complete','failed') THEN clock_timestamp() ELSE NULL END WHERE id=$1 AND claim=$2 AND state='claimed' AND claim_until>clock_timestamp()")
            .bind(job.id).bind(job.claim).bind(state).bind(if success {None} else if identity_changed {Some("identity_changed")} else if job.attempts>=5 {Some("attempts_exhausted")} else {Some("upstream_unavailable")}).execute(&mut *tx).await?.rows_affected();
        if changed != 1 {
            return Err(Error::Stale);
        }
        if success && job.revoke_device {
            sqlx::query("UPDATE playback.sessions SET state='stopped',media_hash=NULL WHERE id=$1 AND state IN ('stopping','expired','failed','uncertain')")
                .bind(job.session_id).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Only complete-cleanup sessions with no unresolved effects are eligible.
    /// Returns private references for a later, separately proven unreferenced GC.
    /// This method deliberately never deletes files after an ambiguous DB commit.
    pub async fn retain(&self, older_than: DateTime<Utc>) -> Result<Vec<(Uuid, Uuid)>> {
        if older_than > Utc::now() - chrono::Duration::days(1) {
            return Err(Error::Invalid("retention horizon"));
        }
        let mut tx = self.pool.begin().await?;
        let ids: Vec<(Uuid,Uuid,Uuid)>=sqlx::query_as("SELECT s.id,s.credential_ref,s.replay_ref FROM playback.sessions s WHERE s.state='stopped' AND s.updated_at<$1 AND NOT EXISTS(SELECT FROM playback.cleanup c WHERE c.session_id=s.id AND c.state<>'complete') AND NOT EXISTS(SELECT FROM playback.operations o JOIN playback.effects e ON e.operation_id=o.id WHERE o.session_id=s.id AND e.status IN ('pending','unknown')) ORDER BY s.id FOR UPDATE OF s SKIP LOCKED LIMIT 100")
            .bind(older_than).fetch_all(&mut *tx).await?;
        for (id, _, _) in &ids {
            sqlx::query("INSERT INTO playback.retired_credentials(reference) SELECT credential_ref FROM playback.sessions WHERE id=$1 UNION SELECT replay_ref FROM playback.sessions WHERE id=$1")
                .bind(id).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO playback.retired_credentials(reference) SELECT replay_ref FROM playback.operations WHERE session_id=$1 AND replay_ref IS NOT NULL ON CONFLICT DO NOTHING")
                .bind(id).execute(&mut *tx).await?;
            sqlx::query("INSERT INTO playback.retired_credentials(reference) SELECT (capabilities->>'_quick_connect_ref')::uuid FROM playback.sessions WHERE id=$1 AND capabilities ? '_quick_connect_ref' ON CONFLICT DO NOTHING")
                .bind(id).execute(&mut *tx).await?;
            sqlx::query("DELETE FROM playback.effects WHERE operation_id IN (SELECT id FROM playback.operations WHERE session_id=$1)").bind(id).execute(&mut *tx).await?;
            for table in ["resources", "cleanup", "operations"] {
                sqlx::query(&format!("DELETE FROM playback.{table} WHERE session_id=$1"))
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
            }
            sqlx::query("DELETE FROM playback.sessions WHERE id=$1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("DELETE FROM playback.freshness f WHERE (f.user_id,f.source_id,f.binding_revision) IN (SELECT old.user_id,old.source_id,old.binding_revision FROM playback.freshness old WHERE old.updated_at<$1 AND NOT EXISTS(SELECT FROM identity.users u WHERE u.id=old.user_id AND u.updated_at::text=old.binding_revision) AND NOT EXISTS(SELECT FROM playback.effects e WHERE e.user_id=old.user_id AND e.source_id=old.source_id AND e.binding_revision=old.binding_revision) AND NOT EXISTS(SELECT FROM playback.sessions s WHERE s.user_id=old.user_id AND s.source_id=old.source_id AND s.binding_revision=old.binding_revision) ORDER BY old.updated_at LIMIT 100)")
            .bind(older_than).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(ids
            .into_iter()
            .map(|(_, credential, replay)| (credential, replay))
            .collect())
    }

    /// Proof is issued only after committed session retirement, never from an
    /// error/timeout while publishing credentials or committing a new reference.
    pub async fn retired_credentials(&self) -> Result<Vec<secrets::RetiredCredential>> {
        let references: Vec<Uuid>=sqlx::query_scalar("SELECT r.reference FROM playback.retired_credentials r WHERE NOT EXISTS(SELECT FROM playback.sessions s WHERE s.credential_ref=r.reference OR s.replay_ref=r.reference OR s.capabilities->>'_quick_connect_ref'=r.reference::text) AND NOT EXISTS(SELECT FROM playback.operations o WHERE o.replay_ref=r.reference) AND NOT EXISTS(SELECT FROM playback.cleanup c WHERE c.credential_ref=r.reference) ORDER BY r.retired_at LIMIT 100")
            .fetch_all(&self.pool).await?;
        Ok(references
            .into_iter()
            .map(|reference| secrets::RetiredCredential { reference })
            .collect())
    }

    /// Call only after remove_retired succeeds (including an already absent file).
    pub async fn finish_credential_gc(&self, retired: &secrets::RetiredCredential) -> Result<()> {
        sqlx::query("DELETE FROM playback.retired_credentials WHERE reference=$1")
            .bind(retired.reference)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

async fn record_snapshot(
    tx: &mut Transaction<'_, Postgres>,
    operation: &Operation,
    result: &Value,
) -> Result<()> {
    let session = locked_session(tx, operation.session_id).await?;
    let (pending,failed):(bool,bool)=sqlx::query_as("SELECT EXISTS(SELECT FROM playback.cleanup c JOIN playback.sessions s ON s.id=c.session_id WHERE c.session_id=$1 AND c.state<>'complete' AND (NOT c.revoke_device OR s.state NOT IN ('preparing','ready','playing','paused'))),EXISTS(SELECT FROM playback.cleanup WHERE session_id=$1 AND state='failed')")
        .bind(session.id).fetch_one(&mut **tx).await?;
    let snapshot = ReceiptSnapshot {
        state: session.state,
        transport_revision: session.transport_revision,
        last_sequence: session.last_sequence,
        tracks: session.tracks,
        transport: session.transport,
        replay_ref: session.replay_ref,
        lease_until: session.lease_until,
        absolute_until: session.absolute_until,
    };
    let result = serde_json::json!({"snapshot":snapshot,"outcome":result,"cleanup_pending":pending,"cleanup_failed":failed});
    bounded_json(&result, 65536)?;
    sqlx::query("UPDATE playback.operations SET result=$2 WHERE id=$1 AND claim=$3")
        .bind(operation.id)
        .bind(result)
        .bind(operation.claim)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

async fn resolve_in(
    tx: &mut Transaction<'_, Postgres>,
    user: Uuid,
    public_id: &str,
    scope: &Scope,
) -> Result<ResolvedItem> {
    scope.validate()?;
    if public_id.is_empty() || public_id.len() > 200 {
        return Err(Error::Invalid("item"));
    }
    let rows: Vec<ResolvedItem>=sqlx::query_as("SELECT u.id AS user_id,i.public_id AS public_item_id,si.id AS source_item_id,s.id AS source_id,s.source_key,si.external_id AS external_item_id,i.kind,c.id AS connection_id,c.generation AS connection_generation,u.updated_at::text AS binding_revision,COALESCE(l.jellyfin_user_id,'') AS remote_profile_id FROM catalog.source_items si JOIN catalog.items i ON i.id=si.item_id JOIN catalog.sources s ON s.id=si.source_id JOIN connections.jellyfin c ON c.source_id=s.id JOIN identity.users u ON u.id=$1 LEFT JOIN identity.jellyfin_user_links l ON l.user_id=u.id AND l.source_id=s.source_key WHERE u.state='enabled' AND s.state='enabled' AND si.state='enabled' AND s.provider='jellyfin' AND i.kind IN ('movie','episode') AND i.public_id=$2 AND s.configuration->>'connection_id'=c.id::text AND s.configuration->>'connection_generation'=c.generation::text AND s.configuration->>'locator_revision'=c.revision AND (cardinality($3::text[])=0 OR s.source_key=ANY($3)) AND (cardinality($4::text[])=0 OR s.provider=ANY($4)) AND (cardinality($5::text[])=0 OR i.kind=ANY($5)) AND EXISTS(SELECT FROM catalog.library_items li JOIN catalog.libraries lib ON lib.id=li.library_id AND lib.source_id=s.id WHERE li.source_item_id=si.id AND li.source_id=s.id AND li.state='enabled' AND lib.state='enabled' AND (cardinality($6::text[])=0 OR lib.public_id=ANY($6))) ORDER BY s.source_key,si.id LIMIT 2")
        .bind(user).bind(public_id).bind(&scope.source_keys).bind(&scope.providers).bind(&scope.kinds).bind(&scope.library_ids).fetch_all(&mut **tx).await?;
    match rows.len() {
        0 => Err(Error::Invalid("item_unavailable")),
        1 => {
            let item = rows.into_iter().next().unwrap();
            if item.remote_profile_id.is_empty() {
                return Err(Error::Invalid("profile_required"));
            }
            Ok(item)
        }
        _ => Err(Error::Invalid("playback_conflict")),
    }
}

async fn require_binding(
    tx: &mut Transaction<'_, Postgres>,
    session: &Session,
    revision: &str,
) -> Result<()> {
    if session.binding_revision != revision {
        return Err(Error::Stale);
    }
    let valid: bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM catalog.sources s JOIN connections.jellyfin c ON c.source_id=s.id JOIN identity.jellyfin_user_links l ON l.source_id=s.source_key AND l.user_id=$1 JOIN catalog.source_items si ON si.id=$2 AND si.source_id=s.id JOIN catalog.items i ON i.id=si.item_id WHERE s.id=$3 AND s.source_key=$10 AND s.provider='jellyfin' AND s.state='enabled' AND si.state='enabled' AND i.kind IN ('movie','episode') AND (jsonb_array_length($9::jsonb->'kinds')=0 OR i.kind IN (SELECT jsonb_array_elements_text($9::jsonb->'kinds'))) AND i.public_id=$4 AND si.external_id=$5 AND c.id=$6 AND c.generation=$7 AND l.jellyfin_user_id=$8 AND s.configuration->>'connection_id'=c.id::text AND s.configuration->>'connection_generation'=c.generation::text AND s.configuration->>'locator_revision'=c.revision AND EXISTS(SELECT FROM catalog.library_items li JOIN catalog.libraries lib ON lib.id=li.library_id AND lib.source_id=s.id WHERE li.source_item_id=si.id AND li.source_id=s.id AND li.state='enabled' AND lib.state='enabled' AND (jsonb_array_length($9::jsonb->'library_ids')=0 OR lib.public_id IN (SELECT jsonb_array_elements_text($9::jsonb->'library_ids')))))")
        .bind(session.user_id).bind(session.source_item_id).bind(session.source_id).bind(&session.public_item_id).bind(&session.external_item_id).bind(session.connection_id).bind(session.connection_generation).bind(&session.remote_profile_id).bind(&session.scope).bind(&session.source_key).fetch_one(&mut **tx).await?;
    if !valid {
        return Err(Error::Stale);
    }
    Ok(())
}

async fn require_fences(
    tx: &mut Transaction<'_, Postgres>,
    session: &Session,
    revision: &str,
) -> Result<()> {
    if !valid_live(session, revision, Utc::now()) {
        return Err(Error::Stale);
    }
    require_binding(tx, session, revision).await
}

#[allow(clippy::too_many_arguments)]
async fn insert_operation(
    tx: &mut Transaction<'_, Postgres>,
    session: Uuid,
    user: Uuid,
    revision: &str,
    kind: &str,
    request: Option<Uuid>,
    sequence: Option<i64>,
    transport: i64,
    body: &Value,
    replay_ref: Option<Uuid>,
) -> Result<Operation> {
    Ok(sqlx::query_as("INSERT INTO playback.operations(id,session_id,user_id,binding_revision,kind,request_id,sequence,transport_revision,body,claim,replay_ref) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11) RETURNING *")
        .bind(Uuid::new_v4()).bind(session).bind(user).bind(revision).bind(kind).bind(request).bind(sequence).bind(transport).bind(body).bind(Uuid::new_v4()).bind(replay_ref).fetch_one(&mut **tx).await?)
}

async fn advance_epoch(tx: &mut Transaction<'_, Postgres>, session: &Session) -> Result<()> {
    // Also invalidate snapshots for other local selectors linked to this same
    // remote personal identity. Binding revisions remain unchanged.
    sqlx::query("INSERT INTO playback.freshness(user_id,source_id,binding_revision,epoch) SELECT u.id,$1,u.updated_at::text,1 FROM identity.users u JOIN identity.jellyfin_user_links l ON l.user_id=u.id JOIN catalog.sources s ON s.source_key=l.source_id WHERE s.id=$1 AND l.jellyfin_user_id=$2 AND (u.id<>$3 OR u.updated_at::text<>$4) ON CONFLICT(user_id,source_id,binding_revision) DO UPDATE SET epoch=playback.freshness.epoch+1,updated_at=clock_timestamp()")
        .bind(session.source_id).bind(&session.remote_profile_id).bind(session.user_id).bind(&session.binding_revision).execute(&mut **tx).await?;
    sqlx::query("INSERT INTO playback.freshness(user_id,source_id,binding_revision,epoch) VALUES($1,$2,$3,1) ON CONFLICT(user_id,source_id,binding_revision) DO UPDATE SET epoch=playback.freshness.epoch+1,updated_at=clock_timestamp()")
        .bind(session.user_id).bind(session.source_id).bind(&session.binding_revision).execute(&mut **tx).await?;
    Ok(())
}

async fn reserve_effect(
    tx: &mut Transaction<'_, Postgres>,
    session: &Session,
    operation: Uuid,
) -> Result<()> {
    // A remote profile stays fenced even after unlink/relink or another local
    // selector links it. Reconciliation reads cannot hide a late remote write.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,1935897197))")
        .bind(format!(
            "{}:{}",
            session.source_id, session.remote_profile_id
        ))
        .execute(&mut **tx)
        .await?;
    let pending: bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM playback.effects WHERE source_id=$1 AND remote_profile_id=$2 AND status IN ('pending','unknown'))")
        .bind(session.source_id).bind(&session.remote_profile_id).fetch_one(&mut **tx).await?;
    if pending {
        return Err(Error::Invalid("personal_state_unknown"));
    }
    advance_epoch(tx, session).await?;
    sqlx::query("INSERT INTO playback.effects(operation_id,user_id,source_id,binding_revision,remote_profile_id) VALUES($1,$2,$3,$4,$5)")
        .bind(operation).bind(session.user_id).bind(session.source_id).bind(&session.binding_revision).bind(&session.remote_profile_id).execute(&mut **tx).await?;
    Ok(())
}

async fn abandon_operations(tx: &mut Transaction<'_, Postgres>, id: Uuid) -> Result<()> {
    // The same session lock guards before_io's durable dispatch claim.
    locked_session(tx, id).await?;
    sqlx::query("WITH abandoned AS (UPDATE playback.operations SET status=CASE WHEN dispatched THEN 'uncertain' ELSE 'failed' END,finished_at=clock_timestamp() WHERE session_id=$1 AND status='pending' RETURNING id,dispatched) UPDATE playback.effects e SET status=CASE WHEN a.dispatched THEN 'unknown' ELSE 'no_effect' END,evidence=CASE WHEN a.dispatched THEN NULL ELSE 'not_dispatched' END,updated_at=clock_timestamp() FROM abandoned a WHERE e.operation_id=a.id AND e.status='pending'")
        .bind(id).execute(&mut **tx).await?;
    Ok(())
}

async fn revoke_in(tx: &mut Transaction<'_, Postgres>, id: Uuid, state: &str) -> Result<()> {
    sqlx::query("UPDATE playback.sessions SET state=$2,media_hash=NULL,updated_at=clock_timestamp() WHERE id=$1")
        .bind(id).bind(state).execute(&mut **tx).await?;
    sqlx::query("UPDATE playback.cleanup SET available_at=clock_timestamp() WHERE session_id=$1 AND revoke_device AND state='pending'")
        .bind(id).execute(&mut **tx).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_resource_capacity_counts_roots_and_children_and_bounds_both_paths() {
        assert!(resources_fit(2, 2859));
        for current in [2048, 2861, 8191] {
            assert!(resources_fit(current, 1));
        }
        assert!(!resources_fit(8192, 1));
        assert!(resources_fit(0, 8192));
        assert!(!resources_fit(0, 8193));
        assert!(!resources_fit(usize::MAX, 1));
    }

    #[test]
    fn gateway_device_ids_have_256_random_bits_and_the_recorded_adapter_shape() {
        let first = generate_device_id().unwrap();
        let second = generate_device_id().unwrap();
        assert_ne!(first, second);
        for device in [first, second] {
            let suffix = device.strip_prefix("screenroom-playback-").unwrap();
            assert_eq!(suffix.len(), 64);
            assert!(
                suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            );
        }
    }

    #[test]
    fn normalized_body_equality_ignores_object_key_order_not_values() {
        let a: Value = serde_json::from_str(r#"{"position_ms":"100","paused":false}"#).unwrap();
        let b: Value = serde_json::from_str(r#"{"paused":false,"position_ms":"100"}"#).unwrap();
        assert_eq!(a, b);
        assert_ne!(a, serde_json::json!({"position_ms":"99","paused":false}));
    }

    #[test]
    fn capability_hashes_are_domain_separated_by_independent_material() {
        let control = secrets::Capability::generate().unwrap();
        let media = secrets::Capability::generate().unwrap();
        assert!(!constant_eq(&control.hash(), &media.hash()));
        assert!(constant_eq(
            &control.hash(),
            &secrets::capability_hash(control.expose())
        ));
        assert!(!constant_eq(&[], &control.hash()));
    }

    #[test]
    fn events_follow_sequence_not_position() {
        assert_eq!(
            Mutation::Event { sequence: u32::MAX }.parts().2,
            Some(i64::from(u32::MAX))
        );
        let before = serde_json::json!({"sequence":1,"position_ms":"10000"});
        let seek = serde_json::json!({"sequence":2,"position_ms":"100"});
        assert!(before["sequence"].as_u64() < seek["sequence"].as_u64());
        assert!(before["position_ms"].as_str() > seek["position_ms"].as_str());
        assert!(bounded_json(&seek, 65536).is_ok());
    }

    #[test]
    fn lease_absolute_and_paused_deadlines_are_independent_and_strict() {
        let now = Utc::now();
        let future = now + chrono::Duration::seconds(90);
        let absolute = now + chrono::Duration::hours(12);
        assert!(lifetime_valid(future, absolute, None, now));
        assert!(!lifetime_valid(now, absolute, None, now));
        assert!(!lifetime_valid(future, now, None, now));
        assert!(!lifetime_valid(
            future,
            absolute,
            Some(now - chrono::Duration::minutes(30)),
            now
        ));
        assert!(lifetime_valid(
            future,
            absolute,
            Some(now - chrono::Duration::minutes(29)),
            now
        ));
    }

    #[test]
    fn events_normalize_decimal_times_without_forbidding_backward_seek() {
        let event = EventBody {
            kind: EventKind::Seeked,
            position_ms: "000010".into(),
            paused: false,
            muted: false,
            volume: Some(100),
        };
        assert_eq!(event.normalized().unwrap()["position_ms"], "10");
        for invalid in ["", "-1", "+1", "1.0", "18446744073709551616"] {
            let invalid = EventBody {
                position_ms: invalid.into(),
                ..event.clone()
            };
            assert!(invalid.normalized().is_err());
        }
        assert!(
            EventBody {
                volume: Some(101),
                ..event
            }
            .normalized()
            .is_err()
        );
    }
}
