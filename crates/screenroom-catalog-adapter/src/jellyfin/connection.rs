use super::{
    Error, JellyfinConfig, Library, LiveJellyfin, Profile, Result, ServerVersion, valid_id,
    validate_config,
};
use serde::Deserialize;
use std::time::Instant;

/// API keys have no display/serialization surface and never enter URLs.
pub struct ApiKey(String);
impl std::fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApiKey([REDACTED])")
    }
}
impl ApiKey {
    pub fn new(value: String) -> Result<Self> {
        if value.is_empty()
            || value.len() > 4096
            || value
                .bytes()
                .any(|b| !(32..127).contains(&b) || b == b'"' || b == b'\\')
        {
            return Err(Error::Secret);
        }
        Ok(Self(value))
    }
    pub(super) fn value(&self) -> &str {
        &self.0
    }
}

#[derive(Clone)]
pub struct ConnectionConfig {
    pub source_key: String,
    pub base_url: String,
    pub timeout_ms: u64,
    pub max_response_bytes: usize,
}
impl std::fmt::Debug for ConnectionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectionConfig")
            .field("source_key", &self.source_key)
            .field("base_url", &"[REDACTED]")
            .field("timeout_ms", &self.timeout_ms)
            .field("max_response_bytes", &self.max_response_bytes)
            .finish()
    }
}

/// Authenticated remote identity, distinct from the private URL/locator digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerIdentity {
    pub id: String,
    pub name: String,
    pub version: ServerVersion,
}

#[derive(Debug, Clone)]
pub struct ConnectionProbe {
    pub server: ServerIdentity,
    /// Includes explicit Unsupported kinds; callers choose eligible catalog kinds.
    pub libraries: Vec<Library>,
    /// Enabled profiles, including hidden ones, matching existing user-state logic.
    pub profiles: Vec<Profile>,
}

impl LiveJellyfin {
    /// No filesystem IO or network IO; only authenticated probe establishes access.
    /// In-memory instances have no file locator and cannot supply a locator revision.
    pub fn from_api_key(config: ConnectionConfig, key: ApiKey) -> Result<Self> {
        let config = JellyfinConfig {
            source_key: config.source_key,
            base_url: config.base_url,
            secret_file: Default::default(),
            timeout_ms: config.timeout_ms,
            max_response_bytes: config.max_response_bytes,
        };
        let base = validate_config(&config)?;
        Self::build(config, base, key)
    }

    /// One authenticated request; does not discover libraries/profiles or establish eligibility.
    pub async fn server_identity(&self) -> Result<ServerIdentity> {
        let remote: IdentityDto = self.json(&["System", "Info"], &[]).await?;
        let id = remote.id.ok_or(Error::InvalidServerIdentity)?;
        valid_id(&id).map_err(|_| Error::InvalidServerIdentity)?;
        if id.trim() != id
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(Error::InvalidServerIdentity);
        }
        let version = remote
            .version
            .as_deref()
            .and_then(ServerVersion::parse)
            .ok_or(Error::InvalidServerIdentity)?;
        let name: String = remote
            .server_name
            .ok_or(Error::InvalidServerIdentity)?
            .chars()
            .filter(|c| !c.is_control())
            .collect();
        if name.trim().is_empty() || name.len() > 256 {
            return Err(Error::InvalidServerIdentity);
        }
        Ok(ServerIdentity { id, name, version })
    }

    /// All three operations require the supplied API key; public version info is insufficient.
    /// No indexing, persistence, enqueue, or credential return occurs.
    pub async fn probe_connection(&self) -> Result<ConnectionProbe> {
        let server = observe_probe_stage("server_identity", self.server_identity()).await?;
        let libraries = observe_probe_stage("libraries", self.libraries()).await?;
        let profiles = observe_probe_stage("profiles", self.profiles()).await?;
        observe_probe_stage("validate_response", async {
            Self::validate_probe_response(server, libraries, profiles)
        })
        .await
    }

    fn validate_probe_response(
        server: ServerIdentity,
        mut libraries: Vec<Library>,
        mut profiles: Vec<Profile>,
    ) -> Result<ConnectionProbe> {
        let mut ids = std::collections::BTreeSet::new();
        if libraries.len() > 4096 || profiles.len() > 4096 {
            return Err(Error::InvalidResponse);
        }
        for library in &mut libraries {
            if !ids.insert(library.external_id.clone()) {
                return Err(Error::InvalidResponse);
            }
            library.name.retain(|c| !c.is_control());
            if library.name.trim().is_empty() || library.name.len() > 1024 {
                return Err(Error::InvalidResponse);
            }
            if let super::LibraryType::Unsupported(kind) = &mut library.kind {
                kind.retain(|c| !c.is_control());
                if kind.len() > 256 {
                    return Err(Error::InvalidResponse);
                }
            }
        }
        ids.clear();
        for profile in &mut profiles {
            if !ids.insert(profile.id.clone()) {
                return Err(Error::InvalidResponse);
            }
            profile.name.retain(|c| !c.is_control());
            if profile.name.trim().is_empty() || profile.name.len() > 1024 {
                return Err(Error::InvalidResponse);
            }
        }
        Ok(ConnectionProbe {
            server,
            libraries,
            profiles,
        })
    }
}

async fn observe_probe_stage<T>(
    stage: &'static str,
    operation: impl Future<Output = Result<T>>,
) -> Result<T> {
    struct Attempt {
        stage: &'static str,
        started: Instant,
        completed: bool,
    }
    impl Drop for Attempt {
        fn drop(&mut self) {
            if !self.completed {
                tracing::info!(
                    stage = self.stage,
                    outcome = "cancelled",
                    elapsed_ms = self.started.elapsed().as_secs_f64() * 1000.0,
                    "Jellyfin connection probe"
                );
            }
        }
    }

    let mut attempt = Attempt {
        stage,
        started: Instant::now(),
        completed: false,
    };
    let result = operation.await;
    attempt.completed = true;
    let elapsed_ms = attempt.started.elapsed().as_secs_f64() * 1000.0;
    match &result {
        Ok(_) => tracing::debug!(
            stage,
            outcome = "ok",
            elapsed_ms,
            "Jellyfin connection probe"
        ),
        Err(error) => {
            let category = match error {
                Error::InvalidConfig => "invalid_config",
                Error::Secret => "invalid_credentials",
                Error::InvalidScope => "invalid_scope",
                Error::Transport => "transport",
                Error::Resolution => "dns_resolution",
                Error::DestinationRejected => "destination_rejected",
                Error::Deadline => "deadline",
                Error::TooLarge => "response_too_large",
                Error::Unauthorized => "unauthorized",
                Error::NotFound => "not_found",
                Error::RateLimited { .. } => "rate_limited",
                Error::Http(_) => "http_status",
                Error::InvalidResponse => "invalid_response",
                Error::InvalidServerIdentity => "invalid_server_identity",
                Error::UnsupportedCollection => "unsupported_collection",
                Error::Hierarchy => "invalid_hierarchy",
            };
            let http_status = match error {
                Error::Http(status) => Some(*status),
                _ => None,
            };
            tracing::warn!(
                stage,
                outcome = "failed",
                category,
                http_status,
                elapsed_ms,
                "Jellyfin connection probe"
            );
        }
    }
    result
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct IdentityDto {
    id: Option<String>,
    server_name: Option<String>,
    version: Option<String>,
}

#[cfg(test)]
mod tests;
