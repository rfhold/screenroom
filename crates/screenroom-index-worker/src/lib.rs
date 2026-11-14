//! Durable ingestion. Persistence and verified storage I/O belong to backend repositories.
pub mod full;
pub mod live;
pub mod mapping;
pub mod pipeline;
mod repair;

pub use pipeline::{Step, Worker};
