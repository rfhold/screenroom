//! Bounded Jellyfin catalog, personal-state, and private playback adapters.

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
