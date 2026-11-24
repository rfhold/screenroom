use crate::{Catalog, page, status};
use screenroom_backend::{
    Uuid,
    search::{LexicalMethods, SnapshotWrite, reciprocal_rank_fusion, snapshot_digest},
};
use screenroom_proto::{catalog::v1::*, common::v1::SearchMethods};
use tonic::{Request, Response, Status};
const INDEX_REVISION: &str = "document-v1";
#[derive(serde::Serialize, serde::Deserialize)]
struct Cursor {
    id: Uuid,
    offset: u32,
    model: Option<String>,
    index: String,
}

#[tonic::async_trait]
impl search_service_server::SearchService for Catalog {
    async fn search(
        &self,
        request: Request<SearchRequest>,
    ) -> Result<Response<SearchResponse>, Status> {
        let user = crate::progress::selected_user(request.metadata())?;
        let r = request.into_inner();
        let query = screenroom_backend::embedding::preprocess(&r.query);
        if query.is_empty() || query.chars().count() > 8000 {
            return Err(Status::invalid_argument("query length"));
        }
        let mut scope = page::scope(&r.source_id, &r.library_id)?;
        match r.source.as_str() {
            "" | "all" => {}
            "jellyfin" => scope.providers.push(r.source),
            _ => return Err(Status::invalid_argument("source provider")),
        }
        if let Some(t) = r.types {
            if t.movie {
                scope.kinds.push("movie".into());
            }
            if t.series {
                scope.kinds.push("series".into());
            }
            if t.episode {
                scope.kinds.push("episode".into());
            }
            if scope.kinds.is_empty() {
                return Err(Status::invalid_argument("select at least one search type"));
            }
        } else {
            scope.kinds = vec!["movie".into(), "series".into(), "episode".into()];
        }
        let mut requested = r.methods.unwrap_or(SearchMethods {
            title_only: false,
            text: true,
            fuzzy: true,
            semantic: true,
        });
        if requested.title_only {
            requested.semantic = false;
        }
        let choices = method_names(&requested);
        if !requested.text && !requested.fuzzy && !requested.semantic {
            return Err(Status::invalid_argument(
                "select at least one search method",
            ));
        }
        let legacy = page::binding("search", &query, &scope)
            + &serde_json::to_string(&choices).map_err(|_| Status::internal("methods"))?;
        let binding = compact_binding(&query, &scope, &choices)?;
        let (size, cursor) =
            page::decode_with_legacy::<Cursor>(r.page.as_ref(), &binding, Some(&legacy))?;
        let cursor = if let Some(cursor) = cursor {
            if cursor.index != INDEX_REVISION {
                return Err(Status::invalid_argument("search index token"));
            }
            cursor
        } else {
            let lexical = LexicalMethods {
                title: false,
                full_text: requested.text,
                fuzzy: requested.fuzzy,
            };
            let rankings = if requested.title_only {
                self.search
                    .lexical_titles(&scope, &query, &lexical, 1000)
                    .await
            } else {
                self.search.lexical(&scope, &query, &lexical, 1000).await
            }
            .map_err(status)?;
            let mut rankings = vec![rankings];
            let mut effective = requested;
            let mut reason = "";
            let mut model_identity = None;
            if effective.semantic {
                let model = self.search.active_model().await.map_err(status)?;
                model_identity = model
                    .as_ref()
                    .map(|m| format!("{}:{}:{}", m.id, m.revision, m.document_version));
                reason = match (&self.embedding, model) {
                    (None, _) => "embedding_unavailable",
                    (Some(_), None) => "embedding_coverage_incomplete",
                    (Some(client), Some(model)) => {
                        let config = client.config();
                        if model.name != config.model
                            || model.revision != config.revision
                            || model.dimensions != 1024
                            || model.document_version
                                != screenroom_backend::embedding::DOCUMENT_VERSION
                            || model.distance_metric != "cosine"
                        {
                            "embedding_incompatible"
                        } else {
                            match client.embed(std::slice::from_ref(&query)).await {
                                Ok(mut batch) => match batch.vectors.pop() {
                                    Some(vector) => {
                                        let result = self
                                            .search
                                            .vector_with_coverage(&scope, &vector, model.id, 1000)
                                            .await
                                            .map_err(status)?;
                                        if result.coverage.complete {
                                            rankings.push(result.candidates);
                                            ""
                                        } else {
                                            "embedding_coverage_incomplete"
                                        }
                                    }
                                    None => "embedding_incompatible",
                                },
                                Err(
                                    screenroom_backend::Error::Incompatible(_)
                                    | screenroom_backend::Error::Invalid(_),
                                ) => "embedding_incompatible",
                                Err(_) => "embedding_unavailable",
                            }
                        }
                    }
                };
                effective.semantic = reason.is_empty();
            }
            if !effective.semantic && !effective.text && !effective.fuzzy {
                return Err(Status::unavailable(reason));
            }
            let results = reciprocal_rank_fusion(&rankings, 1000);
            let digest = digest(&query, &scope, &choices, model_identity.as_deref())?;
            let id = self
                .search
                .save_snapshot(&SnapshotWrite {
                    digest: digest.to_vec(),
                    model_revision: model_identity.clone(),
                    index_revision: INDEX_REVISION.into(),
                    results,
                    effective_methods: method_names(&effective),
                    degradation_reason: reason.into(),
                    ttl_seconds: 900,
                })
                .await
                .map_err(status)?;
            Cursor {
                id,
                offset: 0,
                model: model_identity,
                index: INDEX_REVISION.into(),
            }
        };
        let digest = digest(&query, &scope, &choices, cursor.model.as_deref())?;
        let saved = self
            .search
            .snapshot_page(cursor.id, &digest, &scope, cursor.offset, size)
            .await
            .map_err(status)?;
        let ids: Vec<_> = saved.results.into_iter().map(|hit| hit.public_id).collect();
        let items = self
            .catalog
            .get_ranked(&scope, &ids)
            .await
            .map_err(status)?;
        let next_page_token = match saved.next_offset {
            Some(offset) => {
                let cursor = Cursor { offset, ..cursor };
                if !compact_encoding_enabled() {
                    let token = page::encode(&legacy, &cursor)?;
                    if token.len() > 8192 {
                        return Err(Status::invalid_argument(
                            "query exceeds legacy page token budget",
                        ));
                    }
                    token
                } else {
                    page::encode(&binding, &cursor)?
                }
            }
            None => String::new(),
        };
        let methods = &saved.effective_methods;
        Ok(Response::new(SearchResponse {
            items: self.hydrate_items(&scope, items, user.as_deref()).await?,
            next_page_token,
            effective_methods: Some(SearchMethods {
                title_only: methods.iter().any(|s| s == "title_only"),
                text: methods.iter().any(|s| s == "text"),
                fuzzy: methods.iter().any(|s| s == "fuzzy"),
                semantic: methods.iter().any(|s| s == "semantic"),
            }),
            degradation_reason: saved.degradation_reason,
        }))
    }
    async fn get_search_capabilities(
        &self,
        _: Request<GetSearchCapabilitiesRequest>,
    ) -> Result<Response<GetSearchCapabilitiesResponse>, Status> {
        Ok(Response::new(GetSearchCapabilitiesResponse {methods:vec!["title_only".into(),"text".into(),"fuzzy".into(),"semantic".into()],types:vec!["movie".into(),"series".into(),"episode".into()],sources:vec!["all".into(),"jellyfin".into()],semantic_description:"Exact eligible-set cosine retrieval and reciprocal-rank fusion; explicit lexical degradation when embeddings are unavailable, incompatible, or incomplete. No reranking.".into(),maximum_query_length:8000}))
    }
}

fn compact_binding(
    query: &str,
    scope: &screenroom_backend::catalog::Scope,
    methods: &[String],
) -> Result<String, Status> {
    let digest = snapshot_digest(query, scope, methods).map_err(status)?;
    let mut binding = String::from("search-v2:");
    use std::fmt::Write;
    for byte in digest {
        write!(&mut binding, "{byte:02x}").expect("String writes are infallible");
    }
    Ok(binding)
}

fn compact_encoding_enabled() -> bool {
    std::env::var_os("SCREENROOM_SEARCH_COMPACT_CURSOR").is_some_and(|value| value == "1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_cursor_is_bounded_for_maximum_multibyte_query() {
        let query = "猫".repeat(8000);
        let scope = page::scope("source", "library").unwrap();
        let methods = vec!["text".into()];
        let binding = compact_binding(&query, &scope, &methods).unwrap();
        let cursor = Cursor {
            id: Uuid::new_v4(),
            offset: 1,
            model: None,
            index: INDEX_REVISION.into(),
        };
        let token = page::encode(&binding, &cursor).unwrap();
        assert!(token.len() < 512);
        let next = Page { size: 1, token };
        assert_eq!(
            page::decode::<Cursor>(Some(&next), &binding)
                .unwrap()
                .1
                .unwrap()
                .id,
            cursor.id
        );
        let other = page::scope("other", "library").unwrap();
        assert!(
            page::decode::<Cursor>(
                Some(&next),
                &compact_binding(&query, &other, &methods).unwrap()
            )
            .is_err()
        );
        let legacy = page::binding("search", "short", &scope) + "[\"text\"]";
        let old = Page {
            size: 1,
            token: page::encode(&legacy, &cursor).unwrap(),
        };
        assert!(page::decode_with_legacy::<Cursor>(Some(&old), &binding, Some(&legacy)).is_ok());
    }
}
fn digest(
    query: &str,
    scope: &screenroom_backend::catalog::Scope,
    choices: &[String],
    model: Option<&str>,
) -> Result<[u8; 32], Status> {
    let mut identity = choices.to_vec();
    identity.push(format!("index:{INDEX_REVISION}"));
    identity.push(format!("model:{}", model.unwrap_or("none")));
    snapshot_digest(query, scope, &identity).map_err(status)
}
fn method_names(m: &SearchMethods) -> Vec<String> {
    [
        (m.title_only, "title_only"),
        (m.text, "text"),
        (m.fuzzy, "fuzzy"),
        (m.semantic, "semantic"),
    ]
    .into_iter()
    .filter(|(on, _)| *on)
    .map(|(_, s)| s.into())
    .collect()
}
