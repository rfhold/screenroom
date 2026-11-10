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
                    && source.provider == "jellyfin"
                    && (scope.source_keys.is_empty()
                        || scope.source_keys.contains(&source.source_key))
            })
            .map(|source| SourceLibraryScope {
                source_id: source.source_key,
                library_id: scope.library_ids.first().cloned().unwrap_or_default(),
            })
            .collect();
        if scopes.len() > 32 {
            return Err(Status::resource_exhausted("narrow progress source scope"));
        }
        if scopes.is_empty() {
            return Ok(vec![]);
        }
        let requested_sources: std::collections::BTreeSet<_> =
            scopes.iter().map(|scope| scope.source_id.clone()).collect();
        let channel = tonic::transport::Endpoint::from_shared(uri.clone())
            .map_err(|_| Status::unavailable("private progress endpoint"))?
            .connect_timeout(Duration::from_secs(5).min(deadline.remaining()?))
            .timeout(Duration::from_secs(10).min(deadline.remaining()?))
            .connect()
            .await
            .map_err(|_| Status::unavailable("private progress unavailable"))?;
        let mut request = tonic::Request::new(GetContinueWatchingRequest {
            user_id,
            scopes,
            page_size: 50,
            page_token: String::new(),
        });
        request.set_timeout(Duration::from_secs(10).min(deadline.remaining()?));
        let response = PlaybackStateServiceClient::new(channel)
            .max_decoding_message_size(256 * 1024)
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
        let mut items = vec![];
        let mut progress = vec![];
        let mut seen = std::collections::BTreeSet::new();
        for entry in response.entries {
            let Some(reference) = self
                .catalog
                .resolve_reference(scope, &entry.source_id, &entry.external_item_id)
                .await
                .map_err(status)?
            else {
                continue;
            };
            if !seen.insert(reference.public_id.clone()) {
                continue;
            }
            if let Some(row) = self
                .catalog
                .get(scope, &reference.public_id)
                .await
                .map_err(status)?
            {
                items.push(row);
                progress.push(entry.progress_percent);
            }
        }
        let mut items = self.hydrate_items(scope, items).await?;
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
