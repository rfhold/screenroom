use crate::{Catalog, mapping, page, status};
use screenroom_backend::{Uuid, catalog::CreditCursor};
use screenroom_proto::catalog::v1::*;
use tonic::{Request, Response, Status};

#[tonic::async_trait]
impl browse_service_server::BrowseService for Catalog {
    async fn get_home(
        &self,
        request: Request<GetHomeRequest>,
    ) -> Result<Response<GetHomeResponse>, Status> {
        let user = crate::progress::selected_user(request.metadata())?;
        let deadline = crate::progress::Deadline::from_metadata(request.metadata())?;
        let r = request.into_inner();
        let scope = page::scope(&r.source_id, &r.library_id)?;
        let rows = self.catalog.list(&scope, None, 50).await.map_err(status)?;
        let items = self.hydrate_items(&scope, rows).await?;
        let progress = self.continue_watching(user, &scope, deadline).await?;
        Ok(Response::new(GetHomeResponse {
            rows: vec![
                HomeRow {
                    id: "continue-watching".into(),
                    title: "Continue watching".into(),
                    items: progress,
                },
                HomeRow {
                    id: "catalog".into(),
                    title: "Catalog".into(),
                    items,
                },
            ],
        }))
    }
    async fn list_libraries(
        &self,
        request: Request<ListLibrariesRequest>,
    ) -> Result<Response<ListLibrariesResponse>, Status> {
        let r = request.into_inner();
        let scope = page::scope(&r.source_id, "")?;
        let binding = page::binding("libraries", "", &scope);
        let (size, after) = page::decode::<String>(r.page.as_ref(), &binding)?;
        let mut rows = self.catalog.libraries(&scope).await.map_err(status)?;
        rows.sort_by(|a, b| a.public_id.cmp(&b.public_id));
        rows.dedup_by(|a, b| a.public_id == b.public_id);
        rows.retain(|r| after.as_ref().is_none_or(|a| r.public_id > *a));
        let more = rows.len() > size as usize;
        rows.truncate(size as usize);
        let next_page_token = if more {
            page::encode(
                &binding,
                &rows
                    .last()
                    .ok_or_else(|| Status::internal("pagination"))?
                    .public_id,
            )?
        } else {
            String::new()
        };
        Ok(Response::new(ListLibrariesResponse {
            libraries: rows
                .into_iter()
                .map(|r| Library {
                    id: r.public_id,
                    title: r.title,
                })
                .collect(),
            next_page_token,
        }))
    }
    async fn list_library_items(
        &self,
        request: Request<ListLibraryItemsRequest>,
    ) -> Result<Response<ListLibraryItemsResponse>, Status> {
        let r = request.into_inner();
        let scope = page::scope(&r.source_id, &r.library_id)?;
        let binding = page::binding("items", "", &scope);
        let (size, after) = page::decode::<String>(r.page.as_ref(), &binding)?;
        let mut rows = self
            .catalog
            .list_top_level(&scope, after.as_deref(), size + 1)
            .await
            .map_err(status)?;
        let more = rows.len() > size as usize;
        rows.truncate(size as usize);
        let next_page_token = if more {
            page::encode(
                &binding,
                &rows
                    .last()
                    .ok_or_else(|| Status::internal("pagination"))?
                    .public_id,
            )?
        } else {
            String::new()
        };
        Ok(Response::new(ListLibraryItemsResponse {
            items: self.hydrate_items(&scope, rows).await?,
            next_page_token,
        }))
    }
}

#[tonic::async_trait]
impl catalog_service_server::CatalogService for Catalog {
    async fn get_catalog_info(
        &self,
        _: Request<GetCatalogInfoRequest>,
    ) -> Result<Response<GetCatalogInfoResponse>, Status> {
        let sources = self.catalog.sources().await.map_err(status)?;
        let mut notices = Vec::new();
        let mut seen = std::collections::BTreeMap::new();
        for source in sources
            .into_iter()
            .filter(|source| source.state == "enabled")
        {
            let Some(value) = source.configuration.get("notices") else {
                continue;
            };
            let values = value.as_array().ok_or_else(invalid_notice)?;
            for value in values {
                if !value.is_object() {
                    return Err(invalid_notice());
                }
                let field = |key: &str| -> Result<String, Status> {
                    match value.get(key) {
                        None => Ok(String::new()),
                        Some(value) => value.as_str().map(str::to_owned).ok_or_else(invalid_notice),
                    }
                };
                let notice = CatalogNotice {
                    id: field("id")?,
                    section: field("section")?,
                    title: field("title")?,
                    text: field("text")?,
                    link_label: field("link_label")?,
                    link_url: field("link_url")?,
                };
                if notice.id.trim().is_empty() || notice.id.chars().any(char::is_control) {
                    return Err(invalid_notice());
                }
                if let Some(existing) = seen.get(&notice.id) {
                    if existing != &notice {
                        return Err(Status::failed_precondition(
                            "conflicting catalog notice configuration",
                        ));
                    }
                    continue;
                }
                if notices.len() == 32 {
                    return Err(Status::failed_precondition("catalog notice limit exceeded"));
                }
                seen.insert(notice.id.clone(), notice.clone());
                notices.push(notice);
            }
        }
        let branding = self
            .media
            .branding(&screenroom_backend::catalog::Scope::default(), 8)
            .await
            .map_err(status)?
            .into_iter()
            .map(|row| mapping::artwork(row, &self.origin))
            .collect();
        let retrieved_at = self
            .catalog
            .metadata_snapshot(&screenroom_backend::catalog::Scope::default())
            .await
            .map_err(status)?
            .map(|date| {
                date.to_rfc3339_opts(screenroom_backend::chrono::SecondsFormat::AutoSi, true)
            })
            .unwrap_or_default();
        Ok(Response::new(GetCatalogInfoResponse {
            info: Some(CatalogInfo {
                notices,
                retrieved_at,
                branding,
            }),
        }))
    }
    async fn get_item(
        &self,
        request: Request<GetItemRequest>,
    ) -> Result<Response<GetItemResponse>, Status> {
        let r = request.into_inner();
        let scope = page::request_scope(r.scope)?;
        let row = self
            .catalog
            .get(&scope, &r.id)
            .await
            .map_err(status)?
            .ok_or_else(|| Status::not_found("item"))?;
        Ok(Response::new(GetItemResponse {
            item: self.hydrate_items(&scope, vec![row]).await?.pop(),
        }))
    }
    async fn list_children(
        &self,
        request: Request<ListChildrenRequest>,
    ) -> Result<Response<ListChildrenResponse>, Status> {
        let r = request.into_inner();
        let scope = page::request_scope(r.scope)?;
        let binding = page::binding("children", &r.parent_id, &scope);
        let (size, after) = page::decode::<String>(r.page.as_ref(), &binding)?;
        let mut rows = self
            .catalog
            .children(&scope, &r.parent_id, after.as_deref(), size + 1)
            .await
            .map_err(status)?;
        let more = rows.len() > size as usize;
        rows.truncate(size as usize);
        let next_page_token = if more {
            page::encode(
                &binding,
                &rows
                    .last()
                    .ok_or_else(|| Status::internal("pagination"))?
                    .public_id,
            )?
        } else {
            String::new()
        };
        Ok(Response::new(ListChildrenResponse {
            items: self.hydrate_items(&scope, rows).await?,
            next_page_token,
        }))
    }
    async fn list_credits(
        &self,
        request: Request<ListCreditsRequest>,
    ) -> Result<Response<ListCreditsResponse>, Status> {
        let r = request.into_inner();
        let scope = page::request_scope(r.scope)?;
        let binding = page::binding("credits", &r.item_id, &scope);
        let (size, after) = page::decode::<CreditCursor>(r.page.as_ref(), &binding)?;
        let mut rows = self
            .catalog
            .credits(&scope, &r.item_id, after.as_ref(), size + 1)
            .await
            .map_err(status)?;
        let more = rows.len() > size as usize;
        rows.truncate(size as usize);
        let next_page_token = if more {
            page::encode(
                &binding,
                &rows
                    .last()
                    .ok_or_else(|| Status::internal("pagination"))?
                    .cursor(),
            )?
        } else {
            String::new()
        };
        let people: Vec<_> = rows.iter().map(|row| row.person_id).collect();
        let artwork = if people.is_empty() {
            std::collections::BTreeMap::new()
        } else {
            self.media
                .person_artwork(&scope, &people)
                .await
                .map_err(status)?
        };
        Ok(Response::new(ListCreditsResponse {
            credits: rows
                .into_iter()
                .map(|r| Credit {
                    id: r.person_id.to_string(),
                    name: r.name,
                    role: r.role,
                    department: r.department,
                    artwork: artwork
                        .get(&r.person_id)
                        .cloned()
                        .map(|row| mapping::artwork(row, &self.origin)),
                })
                .collect(),
            next_page_token,
        }))
    }
    async fn list_artwork(
        &self,
        request: Request<ListArtworkRequest>,
    ) -> Result<Response<ListArtworkResponse>, Status> {
        let r = request.into_inner();
        let scope = page::request_scope(r.scope)?;
        let binding = page::binding("artwork", "", &scope);
        let (size, after) = page::decode::<Uuid>(r.page.as_ref(), &binding)?;
        let mut rows = self
            .media
            .list(&scope, after, size + 1)
            .await
            .map_err(status)?;
        let more = rows.len() > size as usize;
        rows.truncate(size as usize);
        let next_page_token = if more {
            page::encode(
                &binding,
                &rows
                    .last()
                    .ok_or_else(|| Status::internal("pagination"))?
                    .id,
            )?
        } else {
            String::new()
        };
        Ok(Response::new(ListArtworkResponse {
            artwork: rows
                .into_iter()
                .map(|r| mapping::artwork(r, &self.origin))
                .collect(),
            next_page_token,
        }))
    }
}

fn invalid_notice() -> Status {
    Status::failed_precondition("invalid catalog notice configuration")
}
