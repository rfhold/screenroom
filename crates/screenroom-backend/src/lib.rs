//! Concrete persistence and bounded I/O shared by the trusted-local backend.
pub mod catalog;
pub mod config;
pub mod connections;
pub mod db;
pub mod embedding;
pub mod ingest;
mod queue_admin;
pub mod repair;
pub use queue_admin::{
    JobSummary, QueueBlockers, QueueObservation, QueueSnapshot, RedriveEligibility,
};
pub mod media;
pub mod search;
pub mod storage;
pub mod watched;

pub use chrono;
pub use screenroom_queue as queue;
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
    #[error("connection: {0}")]
    Connection(&'static str),
}
pub type Result<T> = std::result::Result<T, Error>;

impl From<queue::Error> for Error {
    fn from(error: queue::Error) -> Self {
        match error {
            queue::Error::Database(e) => Self::Database(e),
            queue::Error::Stale => Self::Stale,
            queue::Error::Invalid => Self::Invalid("queue input"),
            queue::Error::Conflict => Self::Invalid("queue dedupe conflict"),
        }
    }
}
pub mod indexing;
