//! Personal observations, separate from shared metadata and resume progress.
use std::{collections::BTreeSet, future::Future, pin::Pin};

pub const MAX_WATCHED_OBSERVATIONS: usize = 50_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserStateScope {
    pub profile_id: String,
    pub library_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct WatchedObservation {
    pub external_item_id: String,
    pub watched: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    Unsupported,
    Unavailable,
    ProfileUnavailable,
    Incomplete,
    Limit,
}

#[derive(Debug, Clone)]
pub struct WatchedSnapshot {
    observations: Vec<WatchedObservation>,
}

impl WatchedSnapshot {
    /// Absence is always unknown, even in a complete snapshot.
    pub fn complete(observations: Vec<WatchedObservation>) -> Result<Self, Error> {
        if observations.len() > MAX_WATCHED_OBSERVATIONS {
            return Err(Error::Limit);
        }
        let mut ids = BTreeSet::new();
        if observations.iter().any(|o| {
            o.external_item_id.is_empty()
                || o.external_item_id.len() > 200
                || o.external_item_id.chars().any(char::is_control)
                || !ids.insert(&o.external_item_id)
        }) {
            return Err(Error::Incomplete);
        }
        Ok(Self { observations })
    }

    pub fn observations(&self) -> &[WatchedObservation] {
        &self.observations
    }
}

pub type Acquisition<'a> =
    Pin<Box<dyn Future<Output = Result<WatchedSnapshot, Error>> + Send + 'a>>;

pub trait UserStateAdapter: Send + Sync {
    fn supports_watched(&self) -> bool;
    fn acquire_watched<'a>(&'a self, scope: &'a UserStateScope) -> Acquisition<'a>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_snapshot_accepts_exact_capacity_and_rejects_overflow() {
        let mut observations: Vec<_> = (0..MAX_WATCHED_OBSERVATIONS)
            .map(|id| WatchedObservation {
                external_item_id: format!("item-{id}"),
                watched: Some(true),
            })
            .collect();
        assert_eq!(
            WatchedSnapshot::complete(observations.clone())
                .unwrap()
                .observations()
                .len(),
            50_000
        );
        observations.push(WatchedObservation {
            external_item_id: "overflow".into(),
            watched: Some(false),
        });
        assert_eq!(
            WatchedSnapshot::complete(observations).unwrap_err(),
            Error::Limit
        );
    }
}
