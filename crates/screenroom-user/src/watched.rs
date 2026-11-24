//! Separately scheduled acquisition; the Jellyfin mapping bridge stays here.
use crate::UserService;
use screenroom_backend::{
    Uuid,
    chrono::Utc,
    watched::{Binding, WatchedRepository},
};
use screenroom_catalog_adapter::user_state::{Error, UserStateAdapter, UserStateScope};
use std::time::Duration;

pub const REFRESH_SECONDS: u64 = 300;
pub const ACQUISITION_SECONDS: u64 = 180;

pub async fn sync_binding(
    repository: &WatchedRepository,
    binding: &Binding,
    adapter: &dyn UserStateAdapter,
    scope: &UserStateScope,
) -> screenroom_backend::Result<()> {
    if !adapter.supports_watched() {
        repository.invalidate(binding).await?;
        return Ok(());
    }
    let acquisition = repository.begin_acquisition(binding).await?;
    let started = Utc::now();
    match tokio::time::timeout(
        Duration::from_secs(ACQUISITION_SECONDS),
        adapter.acquire_watched(scope),
    )
    .await
    {
        Ok(Ok(snapshot)) => repository.publish(&acquisition, &snapshot, started).await,
        Ok(Err(Error::ProfileUnavailable | Error::Unsupported)) => {
            repository.invalidate(binding).await
        }
        _ => Err(screenroom_backend::Error::Unavailable),
    }
}

#[derive(Clone)]
struct MappedBinding {
    binding: Binding,
    source_key: String,
    profile: String,
}

impl UserService {
    async fn watched_bindings(
        &self,
        after: (Uuid, Uuid),
    ) -> screenroom_backend::Result<Vec<MappedBinding>> {
        // Existing Jellyfin links are compatibility input, not a generic profile API.
        let rows: Vec<(Uuid, Uuid, String, String, String)> = screenroom_backend::sqlx::query_as(
            "SELECT u.id,s.id,u.updated_at::text,s.source_key,l.jellyfin_user_id FROM identity.users u JOIN identity.jellyfin_user_links l ON l.user_id=u.id JOIN catalog.sources s ON s.source_key=l.source_id WHERE u.state='enabled' AND s.state='enabled' AND s.provider='jellyfin' AND (u.id,s.id)>($1,$2) ORDER BY u.id,s.id LIMIT 64")
            .bind(after.0).bind(after.1).fetch_all(self.repository.pool()).await?;
        Ok(rows
            .into_iter()
            .map(
                |(user_id, source_id, revision, source_key, profile)| MappedBinding {
                    binding: Binding {
                        user_id,
                        source_id,
                        revision,
                    },
                    source_key,
                    profile,
                },
            )
            .collect())
    }

    async fn sync_watched_mapping(&self, mapped: &MappedBinding) -> screenroom_backend::Result<()> {
        let repository = WatchedRepository::new(self.repository.pool().clone());
        let provider = self
            .connections
            .connect(&mapped.source_key)
            .await
            .map_err(|_| screenroom_backend::Error::Unavailable)?;
        let libraries: Vec<String> = screenroom_backend::sqlx::query_scalar("SELECT external_id FROM catalog.libraries WHERE source_id=$1 AND state='enabled' ORDER BY external_id LIMIT $2")
            .bind(mapped.binding.source_id).bind((screenroom_backend::connections::MAX_SELECTED_LIBRARIES+1) as i64).fetch_all(self.repository.pool()).await?;
        let scope = UserStateScope {
            profile_id: mapped.profile.clone(),
            library_ids: libraries,
        };
        sync_binding(&repository, &mapped.binding, &provider, &scope).await
    }

    /// One sweep has at most two acquisitions in flight and a 240s wall-clock budget.
    /// The keyset cursor rotates through large installations instead of starving later users.
    pub async fn run_watched_sync(self, mut stop: tokio::sync::watch::Receiver<bool>) {
        let mut interval = tokio::time::interval(Duration::from_secs(REFRESH_SECONDS));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut after = (Uuid::nil(), Uuid::nil());
        loop {
            tokio::select! {
                _ = stop.changed() => return,
                _ = interval.tick() => {}
            }
            if *stop.borrow() {
                return;
            }
            let sweep = async {
                let bindings = self.watched_bindings(after).await?;
                for chunk in bindings.chunks(2) {
                    let first = self.sync_watched_mapping(&chunk[0]);
                    let second = async {
                        match chunk.get(1) {
                            Some(binding) => self.sync_watched_mapping(binding).await,
                            None => Ok(()),
                        }
                    };
                    let (first, second) = tokio::join!(first, second);
                    if first.is_err() || second.is_err() {
                        tracing::warn!(
                            "watched refresh unavailable; retained observations expire normally"
                        );
                    }
                    let last = chunk.last().expect("nonempty chunk");
                    after = (last.binding.user_id, last.binding.source_id);
                }
                if bindings.len() < 64 {
                    after = (Uuid::nil(), Uuid::nil());
                }
                Ok::<(), screenroom_backend::Error>(())
            };
            tokio::select! {
                _ = stop.changed() => return,
                result = tokio::time::timeout(Duration::from_secs(240), sweep) => {
                    let reason = match result {
                        Ok(Ok(())) => continue,
                        Ok(Err(_)) => "binding_list_unavailable",
                        Err(_) => "sweep_timeout",
                    };
                    tracing::warn!(reason, "watched refresh unavailable; retained observations expire normally");
                }
            }
        }
    }
}
