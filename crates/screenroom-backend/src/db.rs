use crate::{
    Result,
    config::{DatabaseConfig, ServiceDatabaseConfig},
};
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use std::str::FromStr;

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

// Deliberately omit underlying connection errors, which may contain connection configuration.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StartupError {
    #[error("invalid migration database configuration")]
    MigrationConfiguration,
    #[error("database startup migration failed")]
    Migration,
    #[error("database startup runtime connection failed")]
    Runtime,
}

/// Applies the shared stream with a short-lived migration-capable pool, including on failure.
pub async fn migrate_service_schema(
    config: &ServiceDatabaseConfig,
) -> std::result::Result<(), StartupError> {
    let url = config
        .migration_url
        .as_deref()
        .unwrap_or(&config.runtime.url);
    if url.is_empty() {
        return Err(StartupError::MigrationConfiguration);
    }
    let migration =
        PgConnectOptions::from_str(url).map_err(|_| StartupError::MigrationConfiguration)?;
    let runtime =
        PgConnectOptions::from_str(&config.runtime.url).map_err(|_| StartupError::Runtime)?;
    if migration.get_host() != runtime.get_host()
        || migration.get_port() != runtime.get_port()
        || migration.get_socket() != runtime.get_socket()
        || migration.get_database().unwrap_or(migration.get_username())
            != runtime.get_database().unwrap_or(runtime.get_username())
    {
        return Err(StartupError::MigrationConfiguration);
    }
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .min_connections(0)
        .acquire_timeout(config.runtime.acquire_timeout)
        .connect_lazy(url)
        .map_err(|_| StartupError::MigrationConfiguration)?;
    let result = migrate(&pool).await;
    pool.close().await;
    result.map_err(|_| StartupError::Migration)
}

/// Call before listeners, work, or readiness; the process intentionally has migration capability.
pub async fn connect_service(
    config: &ServiceDatabaseConfig,
) -> std::result::Result<PgPool, StartupError> {
    if config.runtime.max_connections == 0 {
        return Err(StartupError::Runtime);
    }
    migrate_service_schema(config).await?;
    let pool = PgPoolOptions::new()
        .max_connections(config.runtime.max_connections)
        .min_connections(0)
        .acquire_timeout(config.runtime.acquire_timeout)
        .connect_lazy(&config.runtime.url)
        .map_err(|_| StartupError::Runtime)?;
    let result = pool.acquire().await;
    match result {
        Ok(connection) => {
            drop(connection);
            Ok(pool)
        }
        Err(_) => {
            pool.close().await;
            Err(StartupError::Runtime)
        }
    }
}

pub async fn migrate(pool: &PgPool) -> std::result::Result<(), sqlx::migrate::MigrateError> {
    let mut connection = pool.acquire().await?;
    let result = MIGRATOR.run_direct(&mut *connection).await;
    // SQLx retains the session advisory lock on failed migrations. Never pool that session.
    connection.close().await?;
    result
}

pub async fn connect(config: &DatabaseConfig) -> Result<PgPool> {
    Ok(PgPoolOptions::new()
        .max_connections(config.max_connections)
        .acquire_timeout(config.acquire_timeout)
        .connect(&config.url)
        .await?)
}

pub async fn ready(pool: &PgPool) -> Result<()> {
    sqlx::query("SELECT 1").execute(pool).await?;
    Ok(())
}
