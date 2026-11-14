//! Bounded, read-only Jellyfin catalog and personal-state adapters.

pub mod catalog_delta;
pub mod jellyfin;
pub mod user_state;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaKind {
    Movie,
    Series,
    Season,
    Episode,
}
