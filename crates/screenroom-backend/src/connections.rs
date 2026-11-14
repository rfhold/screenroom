//! Trusted-local connection provisioning; credentials never enter database JSON.
mod secrets;
use crate::{
    Error, Result,
    indexing::{self, IndexingRepository},
};
use screenroom_catalog_adapter::jellyfin::{
    ApiKey, ConnectionConfig, ConnectionProbe, LibraryType, LiveJellyfin,
};
pub use secrets::CredentialDirectory;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::path::PathBuf;
use uuid::Uuid;

pub const CREDENTIAL_DIRECTORY_ENV: &str = "SCREENROOM_JELLYFIN_CREDENTIAL_DIRECTORY";
pub const MAX_SELECTED_LIBRARIES: usize = 32;
const TIMEOUT_MS: u64 = 3000;
const RESPONSE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoverRequest {
    pub base_url: String,
    pub api_key: String,
}
impl std::fmt::Debug for DiscoverRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DiscoverRequest([REDACTED])")
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveRequest {
    pub request_id: Uuid,
    pub base_url: String,
    pub api_key: String,
    pub server_id: String,
    pub profile_id: String,
    pub library_ids: Vec<String>,
    pub local_user_id: Uuid,
    #[serde(default)]
    pub refresh: bool,
}
impl std::fmt::Debug for SaveRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SaveRequest([REDACTED])")
    }
}
#[derive(Debug, Serialize)]
pub struct Discovery {
    pub server: DiscoveredServer,
    pub libraries: Vec<DiscoveredLibrary>,
    pub profiles: Vec<DiscoveredProfile>,
}
#[derive(Debug, Serialize)]
pub struct DiscoveredServer {
    pub id: String,
    pub name: String,
    pub version: String,
}
#[derive(Debug, Serialize)]
pub struct DiscoveredLibrary {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub supported: bool,
}
#[derive(Debug, Serialize)]
pub struct DiscoveredProfile {
    pub id: String,
    pub name: String,
    pub hidden: bool,
}
#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct ConnectionSummary {
    pub id: Uuid,
    pub source_id: Uuid,
    pub source_key: String,
    pub display_name: String,
    pub server_version: String,
    pub state: String,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct SavedConnection {
    pub connection: ConnectionSummary,
    pub policy_id: Uuid,
    pub policy_version: i64,
    pub run_id: Option<Uuid>,
}
#[derive(Clone)]
pub struct ConnectionRepository {
    pool: PgPool,
    directory: Option<CredentialDirectory>,
}
fn safe(error: Error) -> Error {
    match error {
        Error::Database(_) => Error::Connection("connection_unavailable"),
        other => other,
    }
}
fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
}
fn prospective(base_url: &str, key: &str, source_key: &str) -> Result<LiveJellyfin> {
    if base_url.is_empty() || base_url.len() > 2048 {
        return Err(Error::Connection("invalid_connection"));
    }
    if !key.is_empty() && base_url.contains(key) {
        return Err(Error::Connection("invalid_connection"));
    }
    LiveJellyfin::from_api_key(
        ConnectionConfig {
            source_key: source_key.into(),
            base_url: base_url.into(),
            timeout_ms: TIMEOUT_MS,
            max_response_bytes: RESPONSE_BYTES,
        },
        ApiKey::new(key.into()).map_err(|_| Error::Connection("invalid_credentials"))?,
    )
    .map_err(|_| Error::Connection("invalid_connection"))
}
fn validate_probe(
    probe: &screenroom_catalog_adapter::jellyfin::ConnectionProbe,
    key: &str,
) -> Result<()> {
    let reflected = probe.server.id.contains(key)
        || probe.server.name.contains(key)
        || format!(
            "{}.{}.{}",
            probe.server.version.major, probe.server.version.minor, probe.server.version.patch
        )
        .contains(key)
        || probe
            .profiles
            .iter()
            .any(|profile| profile.id.contains(key) || profile.name.contains(key))
        || probe.libraries.iter().any(|library| {
            library.external_id.contains(key)
                || library.name.contains(key)
                || matches!(&library.kind,LibraryType::Unsupported(kind) if kind.contains(key))
        });
    if reflected {
        return Err(Error::Connection("invalid_discovery"));
    }
    Ok(())
}
fn provider_error(error: screenroom_catalog_adapter::jellyfin::Error) -> Error {
    use screenroom_catalog_adapter::jellyfin::Error as E;
    match error {
        E::Unauthorized => Error::Connection("credentials_rejected"),
        E::InvalidServerIdentity => Error::Connection("invalid_server_identity"),
        _ => Error::Connection("discovery_unavailable"),
    }
}
fn discovery(probe: ConnectionProbe) -> Discovery {
    Discovery {
        server: DiscoveredServer {
            id: probe.server.id,
            name: probe.server.name,
            version: format!(
                "{}.{}.{}",
                probe.server.version.major, probe.server.version.minor, probe.server.version.patch
            ),
        },
        libraries: probe
            .libraries
            .into_iter()
            .map(|l| {
                let (kind, supported) = match l.kind {
                    LibraryType::Movies => ("movies", true),
                    LibraryType::Television => ("television", true),
                    LibraryType::Collections => ("collections", true),
                    LibraryType::Unsupported(_) => ("unsupported", false),
                };
                let supported = supported && l.external_id.len() <= 74;
                DiscoveredLibrary {
                    id: l.external_id,
                    name: l.name,
                    kind: kind.into(),
                    supported,
                }
            })
            .collect(),
        profiles: probe
            .profiles
            .into_iter()
            .map(|p| DiscoveredProfile {
                id: p.id,
                name: p.name,
                hidden: p.hidden,
            })
            .collect(),
    }
}
impl ConnectionRepository {
    pub fn new(pool: PgPool, directory: Option<CredentialDirectory>) -> Self {
        Self { pool, directory }
    }
    pub fn from_env(pool: PgPool) -> Result<Self> {
        let directory = std::env::var_os(CREDENTIAL_DIRECTORY_ENV)
            .map(|p| CredentialDirectory::new(PathBuf::from(p)))
            .transpose()?;
        Ok(Self::new(pool, directory))
    }
    pub async fn discover(&self, request: &DiscoverRequest) -> Result<Discovery> {
        let provider = prospective(&request.base_url, &request.api_key, "prospective")?;
        let probe = provider.probe_connection().await.map_err(provider_error)?;
        validate_probe(&probe, &request.api_key)?;
        Ok(discovery(probe))
    }
    pub async fn list(&self, after: Option<Uuid>, limit: u32) -> Result<Vec<ConnectionSummary>> {
        if !(1..=200).contains(&limit) {
            return Err(Error::Invalid("page"));
        }
        sqlx::query_as("SELECT c.id,c.source_id,s.source_key,c.server_name AS display_name,c.server_version,s.state FROM connections.jellyfin c JOIN catalog.sources s ON s.id=c.source_id WHERE ($1::uuid IS NULL OR c.id>$1) ORDER BY c.id LIMIT $2")
            .bind(after).bind(i64::from(limit)).fetch_all(&self.pool).await.map_err(|_|Error::Connection("connection_unavailable"))
    }
    pub async fn indexing_profile(&self, source: Uuid) -> Result<String> {
        sqlx::query_scalar("SELECT c.indexing_profile_id FROM connections.jellyfin c JOIN catalog.sources s ON s.id=c.source_id WHERE c.source_id=$1 AND s.state='enabled' AND s.configuration->>'locator_revision'=c.revision AND s.configuration->>'connection_id'=c.id::text")
            .bind(source).fetch_optional(&self.pool).await.map_err(|_|Error::Connection("connection_unavailable"))?.ok_or(Error::Connection("connection_unavailable"))
    }
    pub async fn sources(&self) -> Result<Vec<(String, String)>> {
        sqlx::query_as("SELECT s.source_key,c.server_name FROM connections.jellyfin c JOIN catalog.sources s ON s.id=c.source_id WHERE s.state='enabled' ORDER BY s.source_key")
            .fetch_all(&self.pool).await.map_err(|_|Error::Connection("connection_unavailable"))
    }
    pub async fn save(&self, request: &SaveRequest) -> Result<SavedConnection> {
        self.save_inner(request).await.map_err(safe)
    }
    async fn replay(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        request: &SaveRequest,
        fingerprint: &[u8],
    ) -> Result<Option<SavedConnection>> {
        let prior:Option<(Vec<u8>,Uuid,serde_json::Value,bool)>=sqlx::query_as("SELECT r.fingerprint,c.credential_ref,r.result,r.created_at<clock_timestamp()-interval '24 hours' FROM connections.save_receipts r JOIN connections.jellyfin c ON c.id=r.connection_id WHERE r.request_id=$1")
            .bind(request.request_id).fetch_optional(&mut **tx).await?;
        if let Some((prior, credential, result, expired)) = prior {
            if expired {
                return Err(Error::Connection("request_expired"));
            }
            if prior != fingerprint
                || self
                    .directory
                    .as_ref()
                    .ok_or(Error::Connection("credential_store_unavailable"))?
                    .read(credential)?
                    != request.api_key
            {
                return Err(Error::Connection("request_conflict"));
            }
            return serde_json::from_value(result)
                .map(Some)
                .map_err(|_| Error::Connection("connection_unavailable"));
        }
        Ok(None)
    }
    async fn save_inner(&self, request: &SaveRequest) -> Result<SavedConnection> {
        if request.library_ids.len() > MAX_SELECTED_LIBRARIES {
            return Err(Error::Connection("selection_capacity"));
        }
        if !valid_id(&request.server_id)
            || !valid_id(&request.profile_id)
            || request.library_ids.is_empty()
            || request.library_ids.iter().any(|s| !valid_id(s))
            || request
                .library_ids
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != request.library_ids.len()
        {
            return Err(Error::Connection("invalid_selection"));
        }
        let directory = self
            .directory
            .as_ref()
            .ok_or(Error::Connection("credential_store_unavailable"))?;
        let fingerprint=Sha256::digest(serde_json::to_vec(&json!({"base_url":request.base_url,"server_id":request.server_id,"profile_id":request.profile_id,"library_ids":request.library_ids,"local_user_id":request.local_user_id,"refresh":request.refresh})).map_err(|_|Error::Connection("invalid_selection"))?).to_vec();
        // Replays compare the submitted key against its private file, never a durable key digest.
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("connection-save:{}", request.request_id))
            .execute(&mut *tx)
            .await?;
        if let Some(saved) = self.replay(&mut tx, request, &fingerprint).await? {
            tx.commit().await?;
            return Ok(saved);
        }
        tx.rollback().await?;
        let id = Uuid::new_v4();
        let source_key = format!("jellyfin_{}", id.simple());
        let provider = prospective(&request.base_url, &request.api_key, &source_key)?;
        let normalized_url = reqwest::Url::parse(&request.base_url)
            .map_err(|_| Error::Connection("invalid_connection"))?
            .to_string();
        let normalized_url = normalized_url.trim_end_matches('/');
        let probe = provider.probe_connection().await.map_err(provider_error)?;
        validate_probe(&probe, &request.api_key)?;
        if probe.server.id != request.server_id {
            return Err(Error::Connection("server_identity_mismatch"));
        }
        if !probe.profiles.iter().any(|p| p.id == request.profile_id) {
            return Err(Error::Connection("profile_unavailable"));
        }
        let mut selected = Vec::new();
        for external in &request.library_ids {
            let library = probe
                .libraries
                .iter()
                .find(|l| &l.external_id == external)
                .ok_or(Error::Connection("library_unavailable"))?;
            if matches!(library.kind, LibraryType::Unsupported(_)) || library.id.len() > 200 {
                return Err(Error::Connection("unsupported_library"));
            }
            selected.push(library);
        }
        if selected
            .iter()
            .filter(|library| matches!(library.kind, LibraryType::Collections))
            .count()
            > 1
        {
            return Err(Error::Connection("unsupported_library_scope"));
        }
        let roots:Vec<_>=selected.iter().map(|library|json!({"id":library.id,"external_id":library.external_id,"name":library.name,"kind":match library.kind {LibraryType::Movies=>"movies",LibraryType::Television=>"television",LibraryType::Collections=>"collections",LibraryType::Unsupported(_)=>"unsupported"}})).collect();
        if serde_json::to_vec(&roots)
            .map_err(|_| Error::Connection("invalid_selection"))?
            .len()
            > 10000
        {
            return Err(Error::Connection("selection_capacity"));
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
            .bind(format!("connection-save:{}", request.request_id))
            .execute(&mut *tx)
            .await?;
        if let Some(saved) = self.replay(&mut tx, request, &fingerprint).await? {
            tx.commit().await?;
            return Ok(saved);
        }
        sqlx::query(
            "SELECT pg_advisory_xact_lock(hashtextextended('managed-connection-admission',0))",
        )
        .execute(&mut *tx)
        .await?;
        let duplicate: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT FROM connections.jellyfin WHERE server_id=$1 OR base_url=$2)",
        )
        .bind(&request.server_id)
        .bind(normalized_url)
        .fetch_one(&mut *tx)
        .await?;
        if duplicate {
            return Err(Error::Connection("server_already_connected"));
        }
        let enabled: Option<String> =
            sqlx::query_scalar("SELECT state FROM identity.users WHERE id=$1 FOR UPDATE")
                .bind(request.local_user_id)
                .fetch_optional(&mut *tx)
                .await?;
        if enabled.as_deref() != Some("enabled") {
            return Err(Error::Connection("local_user_unavailable"));
        }
        let credential = Uuid::new_v4();
        let revision = format!(
            "{:x}",
            Sha256::digest(format!(
                "managed-jellyfin-v1:{id}:{credential}:{}:{}",
                request.server_id, normalized_url
            ))
        );
        let source_id:Uuid=sqlx::query_scalar("INSERT INTO catalog.sources(source_key,provider,configuration,capabilities) VALUES($1,'jellyfin',$2,'{}') RETURNING id")
            .bind(&source_key).bind(json!({"adapter":"jellyfin","connection_id":id,"connection_generation":1,"locator_revision":revision})).fetch_one(&mut *tx).await?;
        for library in &selected {
            sqlx::query("INSERT INTO catalog.libraries(source_id,public_id,external_id,title) VALUES($1,$2,$3,$4)").bind(source_id).bind(&library.id).bind(&library.external_id).bind(&library.name).execute(&mut *tx).await?;
        }
        let version = format!(
            "{}.{}.{}",
            probe.server.version.major, probe.server.version.minor, probe.server.version.patch
        );
        sqlx::query("INSERT INTO connections.jellyfin(id,source_id,server_id,server_name,server_version,base_url,credential_ref,revision,indexing_profile_id) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9)")
            .bind(id).bind(source_id).bind(&probe.server.id).bind(&probe.server.name).bind(&version).bind(normalized_url).bind(credential).bind(&revision).bind(&request.profile_id).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO identity.jellyfin_user_links(user_id,source_id,jellyfin_user_id) VALUES($1,$2,$3) ON CONFLICT(user_id,source_id) DO UPDATE SET jellyfin_user_id=EXCLUDED.jellyfin_user_id,updated_at=clock_timestamp()")
            .bind(request.local_user_id).bind(&source_key).bind(&request.profile_id).execute(&mut *tx).await?;
        sqlx::query("UPDATE identity.users SET updated_at=GREATEST(clock_timestamp(),updated_at+interval '1 microsecond') WHERE id=$1").bind(request.local_user_id).execute(&mut *tx).await?;
        let configuration:indexing::Configuration=serde_json::from_value(json!({"indexing":1,"enabled":true,"automatic_runs":false,"cadence_seconds":3600,"mode":"full","components":{"metadata":true,"artwork":true,"embeddings":false},"strategy":"all","scope":{"library_ids":selected.iter().map(|l|l.id.clone()).collect::<Vec<_>>(),"item_kinds":["movie","series"]},"operator_profile_id":request.profile_id,"limits":{"work_requests":100,"work_records":100,"lifetime_requests":10000000,"lifetime_records":10000000,"deadline_seconds":2592000,"work_timeout_seconds":240}})).map_err(|_|Error::Connection("invalid_selection"))?;
        let (policy, next_tx) = IndexingRepository::configure_in(
            tx,
            source_id,
            &indexing::ConfigureRequest {
                request_id: Uuid::new_v4(),
                existing: None,
                configuration,
            },
        )
        .await?;
        tx = next_tx;
        let run_id = if request.refresh {
            let (run, next_tx) = IndexingRepository::start_in(
                tx,
                &indexing::StartRequest {
                    request_id: Uuid::new_v4(),
                    policy_id: policy.id,
                    policy_version: policy.version,
                    scope: None,
                },
            )
            .await?;
            tx = next_tx;
            Some(run.id)
        } else {
            None
        };
        let result = SavedConnection {
            connection: ConnectionSummary {
                id,
                source_id,
                source_key,
                display_name: probe.server.name,
                server_version: version,
                state: "enabled".into(),
            },
            policy_id: policy.id,
            policy_version: policy.version,
            run_id,
        };
        sqlx::query("INSERT INTO connections.save_receipts(request_id,fingerprint,connection_id,result) VALUES($1,$2,$3,$4)").bind(request.request_id).bind(fingerprint).bind(id).bind(json!(result)).execute(&mut *tx).await?;
        directory.publish(credential, &request.api_key)?;
        // A failed/uncertain COMMIT must never remove a file a committed row could reference.
        tx.commit().await?;
        Ok(result)
    }
    pub async fn capabilities(&self) -> Result<Vec<String>> {
        if self.directory.is_none() {
            return Ok(Vec::new());
        }
        let rows:Vec<(String,String)>=sqlx::query_as("SELECT s.source_key,c.revision FROM connections.jellyfin c JOIN catalog.sources s ON s.id=c.source_id WHERE s.state='enabled' AND s.configuration->>'locator_revision'=c.revision AND s.configuration->>'connection_id'=c.id::text AND s.configuration->>'connection_generation'=c.generation::text ORDER BY c.id")
            .fetch_all(&self.pool).await.map_err(|_|Error::Connection("connection_unavailable"))?;
        let mut result = Vec::new();
        if !rows.is_empty() {
            result.push("adapter:jellyfin".into());
        }
        for (source, revision) in rows {
            result.push(format!("source:{source}"));
            result.push(format!("locator:{revision}"));
        }
        Ok(result)
    }
    pub async fn resolve(&self, source: &str) -> Result<ResolvedConnection> {
        let row:Option<(Uuid,String,String,Uuid)>=sqlx::query_as("SELECT c.id,c.base_url,c.server_id,c.credential_ref FROM connections.jellyfin c JOIN catalog.sources s ON s.id=c.source_id WHERE s.source_key=$1 AND s.state='enabled' AND s.configuration->>'locator_revision'=c.revision AND s.configuration->>'connection_id'=c.id::text AND s.configuration->>'connection_generation'=c.generation::text")
            .bind(source).fetch_optional(&self.pool).await.map_err(|_|Error::Connection("connection_unavailable"))?;
        let (id, url, server, credential) =
            row.ok_or(Error::Connection("connection_unavailable"))?;
        let key = self
            .directory
            .as_ref()
            .ok_or(Error::Connection("credential_store_unavailable"))?
            .read(credential)?;
        Ok(ResolvedConnection {
            id,
            provider: prospective(&url, &key, source)?,
            expected_server: server,
        })
    }
    pub async fn connect(&self, source: &str) -> Result<LiveJellyfin> {
        let resolved = self.resolve(source).await?;
        resolved.verify_identity().await?;
        Ok(resolved.provider)
    }
}
pub struct ResolvedConnection {
    id: Uuid,
    pub provider: LiveJellyfin,
    expected_server: String,
}
impl ResolvedConnection {
    pub async fn verify_identity(&self) -> Result<()> {
        let observed = self
            .provider
            .server_identity()
            .await
            .map_err(provider_error)?;
        self.validate_identity(&observed)?;
        Ok(())
    }
    pub fn validate_identity(
        &self,
        observed: &screenroom_catalog_adapter::jellyfin::ServerIdentity,
    ) -> Result<()> {
        if observed.id != self.expected_server {
            return Err(Error::Connection("server_identity_mismatch"));
        }
        Ok(())
    }
    pub fn id(&self) -> Uuid {
        self.id
    }
}
