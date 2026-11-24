use crate::{Catalog, status};
use screenroom_backend::{Uuid, catalog::Scope};
use screenroom_proto::{
    catalog::v1::Item,
    common::v1::SourceLibraryScope,
    user::internal::v1::{
        GetContinueWatchingRequest, playback_state_service_client::PlaybackStateServiceClient,
    },
};
use std::time::{Duration, Instant};
use tonic::{Status, metadata::MetadataMap};

pub(crate) struct Deadline(Instant);
fn source_scopes(source_id: String, libraries: &[String]) -> Vec<SourceLibraryScope> {
    let libraries = if libraries.is_empty() {
        vec![String::new()]
    } else {
        libraries.to_vec()
    };
    libraries
        .into_iter()
        .map(|library_id| SourceLibraryScope {
            source_id: source_id.clone(),
            library_id,
        })
        .collect()
}
impl Deadline {
    pub(crate) fn from_metadata(metadata: &MetadataMap) -> Result<Self, Status> {
        let mut timeout = crate::transport::RPC_MAXIMUM;
        if let Some(value) = metadata.get("grpc-timeout") {
            let value = value
                .to_str()
                .map_err(|_| Status::invalid_argument("deadline"))?;
            if value.len() < 2 || value.len() > 9 {
                return Err(Status::invalid_argument("deadline"));
            }
            let (digits, unit) = value.split_at(value.len() - 1);
            if !digits.bytes().all(|b| b.is_ascii_digit()) {
                return Err(Status::invalid_argument("deadline"));
            }
            let n: u64 = digits
                .parse()
                .map_err(|_| Status::invalid_argument("deadline"))?;
            let duration = match unit {
                "H" => Duration::from_secs(n * 3600),
                "M" => Duration::from_secs(n * 60),
                "S" => Duration::from_secs(n),
                "m" => Duration::from_millis(n),
                "u" => Duration::from_micros(n),
                "n" => Duration::from_nanos(n),
                _ => return Err(Status::invalid_argument("deadline")),
            };
            timeout = timeout.min(duration);
        }
        Ok(Self(Instant::now() + timeout))
    }

    fn remaining(&self) -> Result<Duration, Status> {
        self.0
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| Status::deadline_exceeded("deadline exceeded"))
    }
}

pub(crate) fn selected_user(metadata: &MetadataMap) -> Result<Option<String>, Status> {
    let mut values = metadata.get_all("x-screenroom-user-id").iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(Status::invalid_argument("one user selector required"));
    }
    let value = value
        .to_str()
        .map_err(|_| Status::invalid_argument("user selector must be UUID"))?;
    Uuid::parse_str(value).map_err(|_| Status::invalid_argument("user selector must be UUID"))?;
    Ok(Some(value.into()))
}

impl Catalog {
    pub(crate) async fn continue_watching(
        &self,
        user_id: Option<String>,
        scope: &Scope,
        deadline: Deadline,
    ) -> Result<Vec<Item>, Status> {
        let Some(user_id) = user_id else {
            return Ok(vec![]);
        };
        tokio::time::timeout(
            deadline.remaining()?,
            self.continue_watching_selected(user_id, scope, &deadline),
        )
        .await
        .map_err(|_| Status::deadline_exceeded("progress deadline"))?
    }

    async fn continue_watching_selected(
        &self,
        user_id: String,
        scope: &Scope,
        deadline: &Deadline,
    ) -> Result<Vec<Item>, Status> {
        let uri = self
            .user_uri
            .as_ref()
            .ok_or_else(|| Status::unavailable("private progress service is not configured"))?;
        let scopes: Vec<_> = self
            .catalog
            .sources()
            .await
            .map_err(status)?
            .into_iter()
            .filter(|source| {
                source.state == "enabled"
                    && (scope.source_keys.is_empty()
                        || scope.source_keys.contains(&source.source_key))
            })
            .flat_map(|source| source_scopes(source.source_key, &scope.library_ids))
            .collect();
        if scopes.len() > 32 {
            return Err(Status::resource_exhausted("narrow progress source scope"));
        }
        if scopes.is_empty() {
            return Ok(vec![]);
        }
        let requested_sources: std::collections::BTreeSet<_> =
            scopes.iter().map(|scope| scope.source_id.clone()).collect();
        let endpoint = tonic::transport::Endpoint::from_shared(uri.clone())
            .map_err(|_| Status::unavailable("private progress endpoint"))?
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10));
        let channel = self
            .progress_channel
            .get_or_init(|| endpoint.connect_lazy())
            .clone();
        let mut client =
            PlaybackStateServiceClient::new(channel).max_decoding_message_size(256 * 1024);
        let mut token = String::new();
        let mut cursors = std::collections::BTreeSet::new();
        let mut records = std::collections::BTreeSet::new();
        let mut items = vec![];
        let mut progress = vec![];
        let mut seen = std::collections::BTreeSet::new();
        let mut count = 0;
        for page in 0..512 {
            let mut request = tonic::Request::new(GetContinueWatchingRequest {
                user_id: user_id.clone(),
                scopes: scopes.clone(),
                page_size: 50,
                page_token: token,
            });
            request.set_timeout(Duration::from_secs(10).min(deadline.remaining()?));
            let response = client
                .get_continue_watching(request)
                .await
                .map_err(|error| Status::new(error.code(), "private progress request failed"))?
                .into_inner();
            if response.entries.len() > 50
                || response.entries.iter().any(|entry| {
                    entry.progress_percent > 100
                        || !requested_sources.contains(&entry.source_id)
                        || entry.external_item_id.is_empty()
                        || entry.external_item_id.len() > 200
                })
            {
                return Err(Status::unavailable(
                    "incompatible private progress response",
                ));
            }
            count += response.entries.len();
            if count > 10000 {
                return Err(Status::resource_exhausted("progress bound"));
            }
            let (references, percentages): (Vec<_>, Vec<_>) = response
                .entries
                .into_iter()
                .filter_map(|entry| {
                    let key = (entry.source_id, entry.external_item_id);
                    records
                        .insert(key.clone())
                        .then_some((key, entry.progress_percent))
                })
                .unzip();
            for (index, row) in self
                .catalog
                .resolve_references(scope, &references)
                .await
                .map_err(status)?
            {
                if !seen.insert(row.public_id.clone()) {
                    continue;
                }
                items.push(row);
                progress.push(percentages[index]);
                if items.len() == 50 {
                    break;
                }
            }
            if items.len() == 50 || response.next_page_token.is_empty() {
                break;
            }
            if page == 511
                || response.next_page_token.len() > 16384
                || !cursors.insert(response.next_page_token.clone())
            {
                return Err(Status::unavailable("invalid progress continuation"));
            }
            token = response.next_page_token;
        }
        let mut items = self.hydrate_items(scope, items, Some(&user_id)).await?;
        for (item, percent) in items.iter_mut().zip(progress) {
            item.progress_percent = Some(percent);
        }
        Ok(items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_library_is_forwarded_and_empty_means_all() {
        let scopes = source_scopes("local".into(), &["movies".into(), "tv".into()]);
        assert_eq!(
            scopes
                .iter()
                .map(|s| s.library_id.as_str())
                .collect::<Vec<_>>(),
            ["movies", "tv"]
        );
        assert!(scopes.iter().all(|s| s.source_id == "local"));
        assert_eq!(source_scopes("local".into(), &[])[0].library_id, "");
    }
    #[test]
    fn local_selector_is_explicit_and_single() {
        let mut metadata = MetadataMap::new();
        assert_eq!(selected_user(&metadata).unwrap(), None);
        metadata.insert("x-screenroom-user-id", "not-a-uuid".parse().unwrap());
        assert!(selected_user(&metadata).is_err());
        metadata.insert(
            "x-screenroom-user-id",
            "00000000-0000-0000-0000-000000000001".parse().unwrap(),
        );
        assert!(selected_user(&metadata).unwrap().is_some());
        metadata.append(
            "x-screenroom-user-id",
            "00000000-0000-0000-0000-000000000002".parse().unwrap(),
        );
        assert!(selected_user(&metadata).is_err());
    }
}
