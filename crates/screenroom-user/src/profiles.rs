use crate::{UserService, local_selector, repository::bounded};
use screenroom_proto::user::v1::*;
use sha2::{Digest, Sha256};
use tonic::{Request, Status};

pub(crate) async fn deadline<T>(
    operation: impl Future<Output = Result<T, Status>>,
) -> Result<T, Status> {
    tokio::time::timeout(std::time::Duration::from_secs(9), operation)
        .await
        .map_err(|_| Status::deadline_exceeded("provider operation deadline exceeded"))?
}

fn page<T: serde::Serialize>(
    items: Vec<T>,
    page: Option<Page>,
    binding: impl serde::Serialize,
) -> Result<(Vec<T>, String), Status> {
    let page = page.unwrap_or_default();
    let size = match page.size {
        0 => 50,
        1..=200 => page.size as usize,
        _ => return Err(Status::invalid_argument("page size exceeds 200")),
    };
    let binding = serde_json::to_vec(&(binding, size, &items))
        .map_err(|_| Status::internal("page encoding failed"))?;
    let cursor = |offset: usize| {
        let mut hash = Sha256::new();
        hash.update(&binding);
        hash.update(offset.to_be_bytes());
        format!("{offset}:{:x}", hash.finalize())
    };
    let start = if page.token.is_empty() {
        0
    } else {
        let start = page
            .token
            .split_once(':')
            .and_then(|(n, _)| n.parse::<usize>().ok())
            .ok_or_else(|| Status::invalid_argument("invalid page token"))?;
        if page.token.len() > 100
            || start == 0
            || start >= items.len()
            || start % size != 0
            || page.token != cursor(start)
        {
            return Err(Status::invalid_argument("invalid or stale page token"));
        }
        start
    };
    let end = (start + size).min(items.len());
    let next = if end < items.len() {
        cursor(end)
    } else {
        String::new()
    };
    Ok((items.into_iter().skip(start).take(size).collect(), next))
}

impl UserService {
    pub(crate) async fn sources(
        &self,
        request: Request<ListJellyfinSourcesRequest>,
    ) -> Result<ListJellyfinSourcesResponse, Status> {
        let id = local_selector(&request)?;
        self.repository.user(id).await?;
        let sources = self
            .connections
            .sources()
            .await
            .map_err(|_| Status::unavailable("provider unavailable"))?;
        let (sources, next_page_token) = page(
            sources,
            request.into_inner().page,
            ("sources", id.to_string()),
        )?;
        Ok(ListJellyfinSourcesResponse {
            sources: sources
                .into_iter()
                .map(|(id, display_name)| JellyfinSource { id, display_name })
                .collect(),
            next_page_token,
        })
    }
    pub(crate) async fn profiles(
        &self,
        request: Request<ListJellyfinProfilesRequest>,
    ) -> Result<ListJellyfinProfilesResponse, Status> {
        let id = local_selector(&request)?;
        self.repository.user(id).await?;
        let request = request.into_inner();
        bounded(&request.source_id)?;
        let provider = self
            .connections
            .connect(&request.source_id)
            .await
            .map_err(|_| Status::unavailable("provider unavailable"))?;
        let mut profiles: Vec<_> = provider
            .profiles()
            .await
            .map_err(|_| Status::unavailable("profile discovery unavailable"))?
            .into_iter()
            .map(|p| (p.id, p.name))
            .collect();
        profiles.sort();
        let (profiles, next_page_token) = page(
            profiles,
            request.page,
            ("profiles", id.to_string(), request.source_id),
        )?;
        Ok(ListJellyfinProfilesResponse {
            profiles: profiles
                .into_iter()
                .map(|(id, display_name)| JellyfinProfile { id, display_name })
                .collect(),
            next_page_token,
        })
    }
    pub(crate) async fn read_link(
        &self,
        request: Request<GetJellyfinLinkRequest>,
    ) -> Result<GetJellyfinLinkResponse, Status> {
        let id = local_selector(&request)?;
        self.repository.user(id).await?;
        let source_id = request.into_inner().source_id;
        bounded(&source_id)?;
        let link = self
            .repository
            .link(id, &source_id)
            .await?
            .map(|jellyfin_user_id| JellyfinLink {
                source_id,
                jellyfin_user_id,
            });
        Ok(GetJellyfinLinkResponse { link })
    }
    pub(crate) async fn select_profile(
        &self,
        request: Request<SetJellyfinLinkRequest>,
    ) -> Result<SetJellyfinLinkResponse, Status> {
        let id = local_selector(&request)?;
        self.repository.user(id).await?;
        let revision = self.repository.revision(id).await?;
        let request = request.into_inner();
        bounded(&request.source_id)?;
        bounded(&request.jellyfin_user_id)?;
        let provider = self
            .connections
            .connect(&request.source_id)
            .await
            .map_err(|_| Status::unavailable("provider unavailable"))?;
        provider
            .validate_profile(&request.jellyfin_user_id)
            .await
            .map_err(|error| match error {
                screenroom_catalog_adapter::jellyfin::Error::InvalidScope => {
                    Status::failed_precondition("profile unavailable")
                }
                _ => Status::unavailable("profile provider unavailable"),
            })?;
        self.repository
            .set_current_link(id, &request.source_id, &request.jellyfin_user_id, &revision)
            .await?;
        let link = self
            .repository
            .link(id, &request.source_id)
            .await?
            .map(|jellyfin_user_id| JellyfinLink {
                source_id: request.source_id,
                jellyfin_user_id,
            });
        Ok(SetJellyfinLinkResponse { link })
    }
    pub(crate) async fn unlink(
        &self,
        request: Request<ClearJellyfinLinkRequest>,
    ) -> Result<ClearJellyfinLinkResponse, Status> {
        let id = local_selector(&request)?;
        self.repository.user(id).await?;
        let source_id = request.into_inner().source_id;
        bounded(&source_id)?;
        self.repository.clear_enabled_link(id, &source_id).await?;
        Ok(ClearJellyfinLinkResponse { source_id })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tokens_bind_operation_user_source_size_and_snapshot() {
        let items = vec!["a", "b", "c"];
        let (_, token) = page(
            items.clone(),
            Some(Page {
                size: 1,
                token: String::new(),
            }),
            ("profiles", "user", "source"),
        )
        .unwrap();
        let next = Some(Page {
            size: 1,
            token: token.clone(),
        });
        assert_eq!(
            page(items.clone(), next.clone(), ("profiles", "user", "source"))
                .unwrap()
                .0,
            vec!["b"]
        );
        assert!(page(items.clone(), next.clone(), ("profiles", "other", "source")).is_err());
        assert!(page(items.clone(), next.clone(), ("sources", "user", "source")).is_err());
        assert!(page(items.clone(), next.clone(), ("profiles", "user", "other")).is_err());
        assert!(page(vec!["a", "b", "d"], next, ("profiles", "user", "source")).is_err());
        assert!(page(items, Some(Page { size: 201, token }), "x").is_err());
    }
}
