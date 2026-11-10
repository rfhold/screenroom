//! Concrete persistence and bounded I/O shared by the trusted-local backend.
pub mod catalog;
pub mod config;
pub mod db;
pub mod embedding;
pub mod ingest;
pub mod media;
pub mod search;
pub mod storage;

pub use chrono;
pub use sqlx;
pub use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database: {0}")]
    Database(#[from] sqlx::Error),
    #[error("invalid input: {0}")]
    Invalid(&'static str),
    #[error("stale lease or disabled policy/source")]
    Stale,
    #[error("source {source_id} has active ingestion run {run_id}")]
    SourceBusy { source_id: Uuid, run_id: Uuid },
    #[error("not found")]
    NotFound,
    #[error("I/O unavailable")]
    Unavailable,
    #[error("incompatible response: {0}")]
    Incompatible(&'static str),
}
pub type Result<T> = std::result::Result<T, Error>;
