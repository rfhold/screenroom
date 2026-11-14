use super::{
    Error, JellyfinConfig, Library, LiveJellyfin, Profile, Result, ServerVersion, valid_id,
    validate_config,
};
use serde::Deserialize;

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
        let server = self.server_identity().await?;
        let mut libraries = self.libraries().await?;
        let mut profiles = self.profiles().await?;
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

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct IdentityDto {
    id: Option<String>,
    server_name: Option<String>,
    version: Option<String>,
}

#[cfg(test)]
mod tests;
