use crate::{Error, Result};
use std::time::Duration;

pub fn required(name: &str) -> Result<String> {
    std::env::var(name)
        .ok()
        .filter(|s| !s.is_empty())
        .ok_or(Error::Invalid("missing environment configuration"))
}

#[derive(Clone)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
    pub acquire_timeout: Duration,
}
impl DatabaseConfig {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            url: required("DATABASE_URL")?,
            max_connections: 8,
            acquire_timeout: Duration::from_secs(5),
        })
    }
}

#[derive(Clone)]
pub struct ServiceDatabaseConfig {
    pub runtime: DatabaseConfig,
    /// Same host/port/socket/database as runtime, different credentials allowed. None uses runtime URL.
    pub migration_url: Option<String>,
}
impl ServiceDatabaseConfig {
    pub fn from_env() -> Result<Self> {
        let migration_url = match std::env::var("SCREENROOM_MIGRATION_DATABASE_URL") {
            Ok(url) if !url.is_empty() => Some(url),
            Err(std::env::VarError::NotPresent) => None,
            _ => return Err(Error::Invalid("invalid migration database configuration")),
        };
        Ok(Self {
            runtime: DatabaseConfig::from_env()?,
            migration_url,
        })
    }
}
