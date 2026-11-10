use crate::status;
use screenroom_backend::{
    Uuid,
    catalog::CatalogRepository,
    ingest::{IngestRepository, Policy, PolicyWrite, Run},
    sqlx::PgPool,
};
use screenroom_proto::catalog::operator::v1::*;
use tonic::{Request, Response, Status};

#[derive(Clone)]
pub struct Operator {
    repo: IngestRepository,
    catalog: CatalogRepository,
}
impl Operator {
    pub fn new(pool: PgPool) -> Self {
        Self {
            repo: IngestRepository::new(pool.clone()),
            catalog: CatalogRepository::new(pool),
        }
    }
}
fn id(s: &str) -> Result<Uuid, Status> {
    Uuid::parse_str(s).map_err(|_| Status::invalid_argument("UUID required"))
}
pub(crate) fn policy(p: Policy) -> IngestionPolicy {
    IngestionPolicy {
        id: p.id.to_string(),
        source_id: p.source_id.to_string(),
        version: p.version as u64,
        enabled: p.enabled,
        selection_json: p.selection_json.to_string(),
        max_items_per_run: p.max_items as u32,
        max_requests_per_run: p.max_requests as u32,
        incremental_seconds: p.incremental_seconds as u32,
        reconcile_seconds: p.reconcile_seconds as u32,
        enrich_metadata: p.enrich_metadata,
        enrich_people: p.enrich_people,
        enrich_artwork: p.enrich_artwork,
    }
}
fn run(r: Run) -> Result<IngestionRun, Status> {
    let p: Policy = serde_json::from_value(r.policy_snapshot)
        .map_err(|_| Status::internal("invalid policy snapshot"))?;
    Ok(IngestionRun {
        id: r.id.to_string(),
        policy: Some(policy(p)),
        state: if r.state == "complete" {
            "completed".into()
        } else {
            r.state
        },
        items_processed: r.items as u64,
        requests_made: r.requests as u64,
        started_at: r.started_at.to_rfc3339(),
        finished_at: r.finished_at.map(|d| d.to_rfc3339()).unwrap_or_default(),
        coverage_json: r.coverage.to_string(),
        error_code: String::new(),
    })
}
pub(crate) fn validate_selection(provider: &str, value: &serde_json::Value) -> Result<(), Status> {
    let invalid = || Status::invalid_argument("provider selection schema");
    let object = value.as_object().ok_or_else(invalid)?;
    match provider {
        "jellyfin" => {
            if object
                .keys()
                .any(|key| !matches!(key.as_str(), "library_ids" | "page_size"))
                || object
                    .get("page_size")
                    .is_some_and(|v| !v.as_u64().is_some_and(|n| (1..=100).contains(&n)))
            {
                return Err(invalid());
            }
            let ids = object
                .get("library_ids")
                .and_then(|v| v.as_array())
                .ok_or_else(invalid)?;
            if ids.is_empty()
                || ids.len() > 32
                || ids.iter().any(|v| {
                    v.as_str()
                        .is_none_or(|s| s.is_empty() || s == "all" || s.len() > 200)
                })
            {
                return Err(invalid());
            }
        }
        "tmdb" => {
            if object.len() != 1 {
                return Err(invalid());
            }
            let feeds = object
                .get("feeds")
                .and_then(|v| v.as_array())
                .ok_or_else(invalid)?;
            if feeds.is_empty() || feeds.len() > 32 {
                return Err(invalid());
            }
            let mut keys = std::collections::BTreeSet::new();
            for feed in feeds {
                if let Some(key) = feed.as_str() {
                    if !known_feed(key) || !keys.insert(key) {
                        return Err(invalid());
                    }
                    continue;
                }
                let feed = feed.as_object().ok_or_else(invalid)?;
                if feed.keys().any(|k| {
                    !matches!(
                        k.as_str(),
                        "feed_key"
                            | "feed"
                            | "max_pages"
                            | "page_size"
                            | "locale"
                            | "region"
                            | "trending_window"
                    )
                }) || !feed
                    .get("feed_key")
                    .and_then(|v| v.as_str())
                    .is_some_and(feed_key)
                {
                    return Err(invalid());
                }
                let key = feed["feed_key"].as_str().ok_or_else(invalid)?;
                if !keys.insert(key) {
                    return Err(invalid());
                }
                let name = feed
                    .get("feed")
                    .and_then(|v| v.as_str())
                    .filter(|s| known_feed(s))
                    .ok_or_else(invalid)?;
                for (key, max) in [("max_pages", 500), ("page_size", 100)] {
                    if !feed
                        .get(key)
                        .and_then(|v| v.as_u64())
                        .is_some_and(|n| n >= 1 && n <= max)
                    {
                        return Err(invalid());
                    }
                }
                if !feed
                    .get("locale")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| {
                        !s.is_empty()
                            && s.len() <= 16
                            && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    })
                {
                    return Err(invalid());
                }
                if let Some(region) = feed.get("region").filter(|v| !v.is_null())
                    && (!matches!(name, "MoviePopular" | "MovieUpcoming" | "MovieNowPlaying")
                        || !region.as_str().is_some_and(|r| {
                            r.len() == 2 && r.bytes().all(|b| b.is_ascii_uppercase())
                        }))
                {
                    return Err(invalid());
                }
                let window = feed.get("trending_window").filter(|v| !v.is_null());
                let trending = matches!(name, "MovieTrending" | "TvTrending");
                if trending != window.is_some()
                    || window.is_some_and(|v| !matches!(v.as_str(), Some("Day" | "Week")))
                {
                    return Err(invalid());
                }
            }
        }
        _ => return Err(invalid()),
    }
    Ok(())
}
fn feed_key(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 100
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'/' | b':'))
}
fn known_feed(s: &str) -> bool {
    matches!(
        s,
        "MoviePopular"
            | "MovieUpcoming"
            | "MovieNowPlaying"
            | "MovieTrending"
            | "TvPopular"
            | "TvOnTheAir"
            | "TvTrending"
    )
}
pub(crate) fn write(p: &IngestionPolicy) -> Result<PolicyWrite, Status> {
    if p.selection_json.len() > 16384
        || p.incremental_seconds == 0
        || p.incremental_seconds > 31_536_000
        || p.reconcile_seconds == 0
        || p.reconcile_seconds > 31_536_000
        || !(1..=1_000_000).contains(&p.max_items_per_run)
        || !(1..=100_000).contains(&p.max_requests_per_run)
    {
        return Err(Status::invalid_argument("policy bounds"));
    }
    Ok(PolicyWrite {
        enabled: p.enabled,
        selection_json: serde_json::from_str(&p.selection_json)
            .map_err(|_| Status::invalid_argument("selection JSON"))?,
        max_items: p.max_items_per_run.into(),
        max_requests: p.max_requests_per_run.into(),
        incremental_seconds: p.incremental_seconds as i32,
        reconcile_seconds: p.reconcile_seconds as i32,
        enrich_metadata: p.enrich_metadata,
        enrich_people: p.enrich_people,
        enrich_artwork: p.enrich_artwork,
    })
}
pub fn validate_policy_write(provider: &str, p: &PolicyWrite) -> Result<(), Status> {
    if p.selection_json.to_string().len() > 16384
        || !(1..=1_000_000).contains(&p.max_items)
        || !(1..=100_000).contains(&p.max_requests)
        || !(1..=31_536_000).contains(&p.incremental_seconds)
        || !(1..=31_536_000).contains(&p.reconcile_seconds)
    {
        return Err(Status::invalid_argument("policy bounds"));
    }
    validate_selection(provider, &p.selection_json)
}
#[tonic::async_trait]
impl ingestion_service_server::IngestionService for Operator {
    async fn get_policy(
        &self,
        r: Request<GetPolicyRequest>,
    ) -> Result<Response<GetPolicyResponse>, Status> {
        let p = self
            .repo
            .policy(id(&r.into_inner().policy_id)?)
            .await
            .map_err(status)?
            .ok_or_else(|| Status::not_found("policy"))?;
        Ok(Response::new(GetPolicyResponse {
            policy: Some(policy(p)),
        }))
    }
    async fn update_policy(
        &self,
        r: Request<UpdatePolicyRequest>,
    ) -> Result<Response<UpdatePolicyResponse>, Status> {
        let p = r
            .into_inner()
            .policy
            .ok_or_else(|| Status::invalid_argument("policy required"))?;
        let existing = self
            .repo
            .policy(id(&p.id)?)
            .await
            .map_err(status)?
            .ok_or_else(|| Status::not_found("policy"))?;
        if id(&p.source_id)? != existing.source_id {
            return Err(Status::invalid_argument("immutable source_id"));
        }
        let source = self
            .catalog
            .sources()
            .await
            .map_err(status)?
            .into_iter()
            .find(|s| s.id == existing.source_id)
            .ok_or_else(|| Status::not_found("source"))?;
        let write = write(&p)?;
        validate_policy_write(&source.provider, &write)?;
        let (p, _) = self
            .repo
            .update_policy(existing.id, &write, write.enabled)
            .await
            .map_err(status)?;
        Ok(Response::new(UpdatePolicyResponse {
            policy: Some(policy(p)),
        }))
    }
    async fn start_run(
        &self,
        r: Request<StartRunRequest>,
    ) -> Result<Response<StartRunResponse>, Status> {
        Ok(Response::new(StartRunResponse {
            run: Some(run(self
                .repo
                .start_run(id(&r.into_inner().policy_id)?)
                .await
                .map_err(status)?)?),
        }))
    }
    async fn get_run(&self, r: Request<GetRunRequest>) -> Result<Response<GetRunResponse>, Status> {
        Ok(Response::new(GetRunResponse {
            run: Some(run(self
                .repo
                .run(id(&r.into_inner().run_id)?)
                .await
                .map_err(status)?
                .ok_or_else(|| Status::not_found("run"))?)?),
        }))
    }
    async fn cancel_run(
        &self,
        r: Request<CancelRunRequest>,
    ) -> Result<Response<CancelRunResponse>, Status> {
        let id = id(&r.into_inner().run_id)?;
        self.repo.cancel_run(id).await.map_err(status)?;
        Ok(Response::new(CancelRunResponse {
            run: Some(run(self
                .repo
                .run(id)
                .await
                .map_err(status)?
                .ok_or_else(|| Status::not_found("run"))?)?),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_worker_instructions_and_provider_urls() {
        assert!(
            validate_selection("jellyfin", &serde_json::json!({"library_ids":["movies"]})).is_ok()
        );
        assert!(
            validate_selection(
                "jellyfin",
                &serde_json::json!({"library_ids":["movies"],"url":"http://evil"})
            )
            .is_err()
        );
        assert!(
            validate_selection(
                "tmdb",
                &serde_json::json!({"feeds":[{"feed_key":"popular","url":"http://evil"}]})
            )
            .is_err()
        );
    }

    #[test]
    fn accepts_worker_feed_schema_and_rejects_incompatible_windows() {
        assert!(
            validate_selection(
                "jellyfin",
                &serde_json::json!({"library_ids":["movies","tv"],"page_size":25})
            )
            .is_ok()
        );
        assert!(
            validate_selection(
                "tmdb",
                &serde_json::json!({"feeds":["MoviePopular","TvOnTheAir"]})
            )
            .is_ok()
        );
        assert!(
            validate_selection("tmdb", &serde_json::json!({"feeds":["provider-wide"]})).is_err()
        );
        let mut selection = serde_json::json!({"feeds":[{"feed_key":"daily-movies","feed":"MovieTrending","max_pages":2,"page_size":25,"locale":"en-US","region":null,"trending_window":"Day"}]});
        assert!(validate_selection("tmdb", &selection).is_ok());
        selection["feeds"][0]["trending_window"] = serde_json::Value::Null;
        assert!(validate_selection("tmdb", &selection).is_err());
        selection["feeds"][0]["feed"] = "MoviePopular".into();
        selection["feeds"][0]["feed_key"] = "MoviePopular:en-US:US:1".into();
        selection["feeds"][0]["region"] = "US".into();
        assert!(validate_selection("tmdb", &selection).is_ok());
        selection["feeds"][0]["page_size"] = 101.into();
        assert!(validate_selection("tmdb", &selection).is_err());
    }
}
