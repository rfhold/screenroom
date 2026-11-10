//! Durable offline ingestion. Persistence and verified external I/O belong to backend repositories.
pub mod mapping;
pub mod pipeline;

pub use pipeline::{Step, Worker};
