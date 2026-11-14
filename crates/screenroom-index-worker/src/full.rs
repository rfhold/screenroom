//! Canonical traversal; bounded work windows and database-backed observations.
use crate::{Worker, live};
use screenroom_backend::{
    Error, Result, Uuid,
    catalog::{CollectionMembershipWrite, ExternalIdWrite, ItemWrite},
    chrono,
    embedding::input_hash,
    ingest::{
        CheckpointWrite, Claim, Completion, Followup, IngestRepository, JobKind, LibraryTouch,
        LibraryWrite, OperationClass, PageCommit, RequestErrorClass, RequestOutcome,
        RequestOutcomeWrite, Run, RunCoverage, TraversalWrites, WorkBudget, selected_library_ids,
    },
    media::{ArtworkInputWrite, ArtworkJob, ArtworkOmission, ArtworkOmissionCode, ValidatedImage},
    sqlx,
    storage::ObjectLocation,
};
use screenroom_catalog_adapter::{
    MediaKind,
    jellyfin::{self, Collection, Library, LibraryType, MemberKind},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

const SCOPE: &str = "full-index-v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FullSelection {
    pub full_index: u32,
    pub library_ids: Vec<String>,
    pub operator_profile_id: String,
    pub work_budget: WorkBudget,
}
impl FullSelection {
    pub fn validate(&self) -> Result<()> {
        if self.full_index != 1
            || self.operator_profile_id.is_empty()
            || self.operator_profile_id.len() > 200
            || !self
                .operator_profile_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        {
            return Err(Error::Invalid("full index selection"));
        }
        selected_library_ids(&json!({"library_ids":self.library_ids}))?;
        let budget = WorkBudget::from_selection(
            &serde_json::to_value(self).map_err(|_| Error::Invalid("selection"))?,
        )?;
        if !matches!(
            budget,
            WorkBudget::PerWorkCommit {
                max_requests: 5..,
                max_records: 3..
            }
        ) {
            return Err(Error::Invalid("full work budget"));
        }
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Root {
    id: String,
    external_id: String,
    name: String,
    kind: String,
}
impl Root {
    fn adapter(&self) -> Library {
        Library {
            id: self.id.clone(),
            external_id: self.external_id.clone(),
            name: self.name.clone(),
            kind: match self.kind.as_str() {
                "movies" => LibraryType::Movies,
                "television" => LibraryType::Television,
                "collections" => LibraryType::Collections,
                other => LibraryType::Unsupported(other.into()),
            },
        }
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Cursor {
    roots: Option<Vec<Root>>,
    phase: u8,
    library: usize,
    offset: u32,
    after: Option<String>,
    container: Option<String>,
    art_kind: usize,
    art_index: usize,
    fingerprint: Option<String>,
    member_fingerprints: Vec<String>,
    operation: String,
    error: Option<String>,
    delta: Option<Value>,
    fallback_reason: Option<String>,
    refresh: Vec<String>,
    refresh_offset: u32,
    delta_next: Option<u32>,
}

pub async fn coverage(pool: sqlx::PgPool, run_id: Uuid) -> Result<Value> {
    let repo = IngestRepository::new(pool.clone());
    let run = repo.run(run_id).await?.ok_or(Error::NotFound)?;
    if run.selection_snapshot.get("full_index").is_none() {
        return Err(Error::Invalid("not a full index run"));
    }
    let c = repo.run_coverage(run_id).await?;
    let traversed = run.state == "complete"
        || repo
            .run_checkpoint(&run, SCOPE)
            .await?
            .and_then(|cursor| cursor["phase"].as_u64())
            .is_some_and(|phase| phase >= 3);
    let mut value = coverage_value(&c, traversed);
    if run.state != "complete" {
        value["all_complete"] = json!(false);
    }
    value["state"] = json!(run.state);
    value["run_id"] = json!(run.id);
    let cancelled: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM queue.jobs WHERE scope_id=$1 AND kind<>'ingest' AND state='cancelled'",
    )
    .bind(run.id)
    .fetch_one(&pool)
    .await?;
    value["followup_cancelled"] = json!(cancelled);
    if cancelled > 0 {
        value["all_complete"] = json!(false);
    }
    Ok(value)
}
fn coverage_value(c: &RunCoverage, traversed: bool) -> Value {
    json!({"metadata_traversed":traversed,"all_complete":traversed && c.followup_ready+c.followup_delayed+c.followup_leased+c.followup_failed+c.unresolved_attempts+c.omissions+c.unresolved_operations+c.pending_artwork_publications==0,
        "primary_unique":c.primary_unique,"followup_ready":c.followup_ready,"followup_delayed":c.followup_delayed,"followup_leased":c.followup_leased,"followup_failed":c.followup_failed,"unknown_attempts":c.unknown_attempts,"retryable_attempts":c.retryable_attempts,"permanent_attempts":c.permanent_attempts,"omissions":c.omissions,"unresolved_operations":c.unresolved_operations,"pending_artwork_publications":c.pending_artwork_publications,"authoritative":false})
}

impl Worker {
    pub(crate) async fn full_process(&self, claim: &Claim, run: &Run, source: &str) -> Result<()> {
        let selection: FullSelection = serde_json::from_value(run.selection_snapshot.clone())
            .map_err(|_| Error::Invalid("full selection"))?;
        selection.validate()?;
        match claim.kind.as_str() {
            "embedding" => self.full_embedding(claim, source).await,
            "artwork" => self.full_artwork(claim, run, source).await,
            "ingest" => {
                let mut cursor: Cursor = serde_json::from_value(
                    self.ingest
                        .run_checkpoint(run, SCOPE)
                        .await?
                        .unwrap_or(json!({})),
                )
                .map_err(|_| Error::Invalid("full checkpoint"))?;
                let previous = cursor.clone();
                match self
                    .full_page(claim, run, source, &selection, &mut cursor)
                    .await
                {
                    Ok(()) => Ok(()),
                    Err(Error::Stale) => Err(Error::Stale),
                    Err(error) => {
                        let operation = cursor.operation;
                        cursor = previous;
                        cursor.operation = operation;
                        cursor.error = Some(match &error {
                            Error::Invalid(code) | Error::Incompatible(code) => (*code).into(),
                            Error::Database(error) => format!(
                                "database_{}",
                                error
                                    .as_database_error()
                                    .and_then(|e| e.code())
                                    .as_deref()
                                    .unwrap_or("unavailable")
                            ),
                            _ => "dependency_unavailable".into(),
                        });
                        let class = if matches!(
                            error,
                            Error::Invalid(_) | Error::Incompatible(_) | Error::NotFound
                        ) {
                            screenroom_backend::queue::FailureClass::Permanent
                        } else {
                            screenroom_backend::queue::FailureClass::Transient
                        };
                        self.ingest
                            .commit_failed_traversal_page(
                                claim,
                                &PageCommit {
                                    libraries: vec![],
                                    items: vec![],
                                    artwork_inputs: vec![],
                                    artwork_omissions: vec![],
                                    followups: vec![],
                                    checkpoint: CheckpointWrite {
                                        scope_key: SCOPE.into(),
                                        cursor: serde_json::to_value(cursor)
                                            .map_err(|_| Error::Invalid("full checkpoint"))?,
                                        watermark: None,
                                    },
                                    requests: 0,
                                    completion: Completion::Continue,
                                },
                                class,
                                if class == screenroom_backend::queue::FailureClass::Permanent {
                                    "invalid_input"
                                } else {
                                    "dependency_unavailable"
                                },
                                None,
                            )
                            .await
                    }
                }
            }
            _ => Err(Error::Invalid("full job kind")),
        }
    }

    async fn full_fetch<T>(
        &self,
        claim: &Claim,
        operation: OperationClass,
        future: impl std::future::Future<Output = jellyfin::Result<T>>,
    ) -> Result<Option<(T, RequestOutcomeWrite)>> {
        let receipt = self.ingest.reserve_work_request(claim, operation).await?;
        let result = tokio::time::timeout(Duration::from_secs(10), future)
            .await
            .unwrap_or(Err(jellyfin::Error::Deadline));
        match result {
            Ok(value) => Ok(Some((
                value,
                RequestOutcomeWrite {
                    receipt_id: receipt.id,
                    outcome: RequestOutcome::Success,
                    error_class: None,
                },
            ))),
            Err(error) => {
                let (class, retry) = provider_failure(&error);
                tracing::warn!(operation=?operation,adapter_error=?error,"full index request failed");
                let outcome = RequestOutcomeWrite {
                    receipt_id: receipt.id,
                    outcome: if retry {
                        RequestOutcome::Retryable
                    } else {
                        RequestOutcome::Permanent
                    },
                    error_class: Some(class),
                };
                if let jellyfin::Error::RateLimited {
                    retry_after_seconds,
                } = error
                {
                    let delay = retry_after_seconds.unwrap_or(60).clamp(1, 86400);
                    self.ingest
                        .defer_work(claim, &outcome, delay as i32)
                        .await?;
                    return Ok(None);
                }
                self.ingest.record_work_outcome(claim, &outcome).await?;
                if retry {
                    self.ingest
                        .fail(
                            claim,
                            screenroom_backend::queue::FailureClass::Transient,
                            "full_request_retry",
                            None,
                        )
                        .await?;
                    Ok(None)
                } else {
                    Err(Error::Incompatible(match error {
                        jellyfin::Error::TooLarge => "provider_response_too_large",
                        jellyfin::Error::Unauthorized => "provider_unauthorized",
                        jellyfin::Error::NotFound => "provider_not_found",
                        _ => "provider_invalid_response",
                    }))
                }
            }
        }
    }

    async fn full_page(
        &self,
        claim: &Claim,
        run: &Run,
        source: &str,
        selection: &FullSelection,
        c: &mut Cursor,
    ) -> Result<()> {
        let Some(adapter) = self.provider(claim, source).await? else {
            return Ok(());
        };
        c.error = None;
        let WorkBudget::PerWorkCommit {
            max_requests,
            max_records,
        } = selection.work_budget;
        let page_limit = 8.min((max_records / 3).min(((max_requests - 2) / 2) as usize)) as u32;
        if c.roots.is_none() {
            c.operation = "discovery".into();
            let Some((libraries, outcome)) = self
                .full_fetch(claim, OperationClass::Discovery, adapter.libraries())
                .await?
            else {
                return Ok(());
            };
            let ids: BTreeSet<_> = libraries.iter().map(|l| l.id.clone()).collect();
            if ids.len() != libraries.len()
                || selection.library_ids.iter().any(|id| !ids.contains(id))
                || libraries
                    .iter()
                    .any(|l| l.id != adapter.public_id(&l.external_id))
            {
                return Err(Error::Invalid("library_selection_changed"));
            }
            let mut roots = vec![];
            let mut writes = vec![];
            for l in libraries
                .into_iter()
                .filter(|l| selection.library_ids.contains(&l.id))
            {
                let kind = match l.kind {
                    LibraryType::Movies => "movies",
                    LibraryType::Television => "television",
                    LibraryType::Collections => "collections",
                    LibraryType::Unsupported(_) => {
                        return Err(Error::Invalid("unsupported_library"));
                    }
                };
                writes.push(LibraryWrite {
                    public_id: l.id.clone(),
                    external_id: l.external_id.clone(),
                    title: l.name.clone(),
                });
                roots.push(Root {
                    id: l.id,
                    external_id: l.external_id,
                    name: l.name,
                    kind: kind.into(),
                });
            }
            if roots.iter().filter(|r| r.kind == "collections").count() > 1 {
                return Err(Error::Invalid("ambiguous_collection_library_scope"));
            }
            roots.sort_by(|a, b| a.id.cmp(&b.id));
            c.roots = Some(roots);
            if serde_json::to_vec(c)
                .map_err(|_| Error::Invalid("checkpoint"))?
                .len()
                > 12000
            {
                return Err(Error::Invalid("discovery_snapshot_capacity"));
            }
            return self
                .full_commit(
                    claim,
                    c,
                    writes,
                    vec![],
                    vec![],
                    vec![],
                    vec![],
                    TraversalWrites {
                        outcomes: vec![outcome],
                        ..Default::default()
                    },
                    Completion::Continue,
                )
                .await;
        }
        // Phase-only transitions do not consume separate queue attempts.
        while c.phase <= 1 {
            let roots = c.roots.as_ref().ok_or(Error::Invalid("roots"))?;
            let next = roots.iter().enumerate().skip(c.library).find(|(_, r)| {
                if c.phase == 0 {
                    r.kind != "collections"
                } else {
                    r.kind == "collections"
                }
            });
            let Some((index, root)) = next.map(|(i, r)| (i, r.clone())) else {
                c.phase += 1;
                c.library = 0;
                c.offset = 0;
                c.fingerprint = None;
                continue;
            };
            c.library = index;
            c.operation = format!("page:{index}:{}", c.offset);
            let mut traversal = TraversalWrites::default();
            let mut items = vec![];
            let mut followups = vec![];
            let refreshing = c.phase == 0 && !c.refresh.is_empty();
            let (ids, next_offset) = if c.phase == 0 {
                let execution = screenroom_backend::indexing::IndexingRepository::new(
                    self.catalog.pool().clone(),
                )
                .execution(run.id)
                .await?;
                let delta = execution.as_ref().is_some_and(|e| e.mode == "incremental")
                    && c.fallback_reason.is_none();
                let fetched = if refreshing {
                    let mut descendants = root.adapter();
                    descendants.external_id = c.refresh[0].clone();
                    descendants.id = adapter.public_id(&descendants.external_id);
                    self.full_fetch(
                        claim,
                        OperationClass::Page,
                        adapter.library_page(&descendants, c.refresh_offset, page_limit),
                    )
                    .await?
                } else if delta {
                    use jellyfin::DeltaError;
                    use screenroom_catalog_adapter::catalog_delta::{
                        Checkpoint, Query, Scope, Timestamp, Window,
                    };
                    let e = execution.as_ref().unwrap();
                    let lower = e
                        .lower_watermark
                        .ok_or(Error::Invalid("delta lower watermark"))?;
                    let window = Window::new(
                        Timestamp::parse(
                            &lower.to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
                        )
                        .map_err(|_| Error::Invalid("delta timestamp"))?,
                        Timestamp::parse(
                            &run.started_at
                                .to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
                        )
                        .map_err(|_| Error::Invalid("delta timestamp"))?,
                    )
                    .map_err(|_| Error::Invalid("delta window"))?;
                    let query = Query::new(
                        Scope::new(source, &root.external_id)
                            .map_err(|_| Error::Invalid("delta scope"))?,
                        window,
                        page_limit,
                        50000,
                        50000,
                    )
                    .map_err(|_| Error::Invalid("delta bounds"))?;
                    let mut checkpoint = if let Some(value) = &c.delta {
                        Checkpoint::restore(
                            &query,
                            &serde_json::to_vec(value)
                                .map_err(|_| Error::Invalid("delta codec"))?,
                        )
                        .map_err(|_| Error::Invalid("delta checkpoint"))?
                    } else {
                        Checkpoint::new(query)
                    };
                    // The adapter performs capability discovery and one filtered page request.
                    let capability = self
                        .ingest
                        .reserve_work_request(claim, OperationClass::Discovery)
                        .await?;
                    let result = self
                        .full_fetch(claim, OperationClass::Page, async {
                            match adapter
                                .library_delta_page(&root.adapter(), &mut checkpoint)
                                .await
                            {
                                Ok(page) => Ok(Ok(page)),
                                Err(DeltaError::FullScanRequired(reason)) => Ok(Err(reason)),
                                Err(DeltaError::Provider(error)) => Err(error),
                                Err(DeltaError::Validation(_)) => {
                                    Err(jellyfin::Error::InvalidResponse)
                                }
                            }
                        })
                        .await?;
                    let Some((result, outcome)) = result else {
                        return Ok(());
                    };
                    traversal.outcomes.push(RequestOutcomeWrite {
                        receipt_id: capability.id,
                        outcome: RequestOutcome::Success,
                        error_class: None,
                    });
                    match result {
                        Ok(page) => {
                            c.refresh = page.refresh_children_external_ids;
                            c.refresh_offset = 0;
                            c.delta_next = page.page.next_offset;
                            c.delta = Some(
                                serde_json::from_slice(
                                    &checkpoint
                                        .serialize()
                                        .map_err(|_| Error::Invalid("delta checkpoint size"))?,
                                )
                                .map_err(|_| Error::Invalid("delta codec"))?,
                            );
                            Some((
                                jellyfin::Page {
                                    items: page.page.items.into_iter().map(|i| i.item).collect(),
                                    offset: page.page.offset,
                                    next_offset: page.page.next_offset,
                                    reported_total: page.page.reported_total,
                                    authoritative: false,
                                },
                                outcome,
                            ))
                        }
                        Err(reason) => {
                            let mut outcome = outcome;
                            if reason!=screenroom_catalog_adapter::catalog_delta::FallbackReason::FilterRejected { outcome.outcome=RequestOutcome::Skipped; }
                            else { outcome.outcome=RequestOutcome::Permanent; outcome.error_class=Some(RequestErrorClass::Invalid); }
                            c.fallback_reason=Some(match reason { screenroom_catalog_adapter::catalog_delta::FallbackReason::FilterRejected=>"filter_rejected",_=>"unsupported_capability" }.into());
                            c.delta = None;
                            c.library = 0;
                            c.offset = 0;
                            c.fingerprint = None;
                            traversal.outcomes.push(outcome);
                            return self
                                .full_commit(
                                    claim,
                                    c,
                                    vec![],
                                    vec![],
                                    vec![],
                                    vec![],
                                    vec![],
                                    traversal,
                                    Completion::Continue,
                                )
                                .await;
                        }
                    }
                } else {
                    self.full_fetch(
                        claim,
                        OperationClass::Page,
                        adapter.library_page(&root.adapter(), c.offset, page_limit),
                    )
                    .await?
                };
                let Some((page, outcome)) = fetched else {
                    return Ok(());
                };
                if page.authoritative
                    || page.offset
                        != if refreshing {
                            c.refresh_offset
                        } else {
                            c.offset
                        }
                {
                    return Err(Error::Invalid("invalid_page"));
                }
                traversal.outcomes.push(outcome);
                let ids = page
                    .items
                    .iter()
                    .map(|i| i.external_id.clone())
                    .collect::<Vec<_>>();
                let mut ordered: BTreeMap<String, jellyfin::Item> = BTreeMap::new();
                let mut order = vec![];
                for item in page.items {
                    let mut chain = vec![item];
                    let mut seen = BTreeSet::new();
                    while let Some(parent) = chain.last().and_then(|i| i.parent_external_id.clone())
                    {
                        if !seen.insert(parent.clone()) || chain.len() > 32 {
                            return Err(Error::Invalid("invalid_hierarchy"));
                        }
                        if let Some(prior) = ordered.get(&parent) {
                            check_parent(
                                chain.last().unwrap().kind,
                                crate::mapping::kind(prior.kind),
                            )?;
                            break;
                        }
                        if self.full_observed(run, &parent, &root.external_id).await? {
                            let kind = self
                                .full_known(run, &parent)
                                .await?
                                .ok_or(Error::Invalid("missing_observed_parent"))?;
                            check_parent(chain.last().unwrap().kind, &kind)?;
                            break;
                        }
                        c.operation = format!(
                            "ancestor:{index}:{}:{}",
                            c.offset,
                            hex(&input_hash(&parent))
                        );
                        let Some((ancestor, outcome)) = self
                            .full_fetch(
                                claim,
                                OperationClass::Ancestor,
                                adapter.item_by_external_id(&parent),
                            )
                            .await?
                        else {
                            return Ok(());
                        };
                        if ancestor.external_id != parent
                            || !matches!(
                                (chain.last().unwrap().kind, ancestor.kind),
                                (MediaKind::Episode, MediaKind::Season)
                                    | (MediaKind::Season, MediaKind::Series)
                            )
                        {
                            return Err(Error::Invalid("invalid_hierarchy"));
                        }
                        traversal.outcomes.push(outcome);
                        chain.push(ancestor);
                    }
                    for item in chain.into_iter().rev() {
                        if !ordered.contains_key(&item.external_id) {
                            order.push(item.external_id.clone());
                            ordered.insert(item.external_id.clone(), item);
                        }
                    }
                }
                for id in order {
                    let item = ordered.remove(&id).ok_or(Error::Invalid("page order"))?;
                    if item.id != adapter.public_id(&item.external_id) {
                        return Err(Error::Invalid("public_identity"));
                    }
                    let mut write = live::map_item(&adapter, &item, &root.external_id)?;
                    write.metadata["provider_artwork_inputs"] = json!(
                        item.artwork
                            .iter()
                            .filter(|a| live::valid_artwork(a))
                            .map(|a| json!({"item_id":a.item_id,"kind":a.kind,"tag":a.tag}))
                            .collect::<Vec<_>>()
                    );
                    write.content_hash = input_hash(&write.metadata.to_string()).to_vec();
                    let embeddings = screenroom_backend::indexing::configuration(run)?
                        .ok_or(Error::Invalid("canonical run required"))?
                        .components
                        .embeddings;
                    let model = match (embeddings, run.embedding_model_id) {
                        (true, None) => {
                            return Err(Error::Invalid("run has no pinned embedding model"));
                        }
                        (_, Some(model)) => model,
                        (false, None) => Uuid::nil(),
                    };
                    let embedding_ready:bool= !embeddings || sqlx::query_scalar("SELECT EXISTS(SELECT FROM search.documents d JOIN catalog.source_items si ON si.id=d.source_item_id JOIN search.embeddings e ON e.document_id=d.id WHERE si.source_id=$1 AND si.external_id=$2 AND e.model_id=$3 AND e.input_hash=$4 AND d.content_hash=$4)").bind(run.source_id).bind(&item.external_id).bind(model).bind(input_hash(&write.document).as_slice()).fetch_one(self.catalog.pool()).await?;
                    self.full_write_or_touch(run, &root, &mut traversal, &mut items, write)
                        .await?;
                    if !embedding_ready {
                        followups.push(Followup {
                            kind: JobKind::Embedding,
                            dedupe_key: format!(
                                "full-embed:{}:{}:{}",
                                run.id, item.external_id, model
                            ),
                            input: json!({"external_id":item.external_id,"model_id":model}),
                        });
                    }
                }
                (ids, page.next_offset)
            } else {
                let Some((page, outcome)) = self
                    .full_fetch(
                        claim,
                        OperationClass::Collection,
                        adapter.collection_page(&root.adapter(), c.offset, page_limit),
                    )
                    .await?
                else {
                    return Ok(());
                };
                if page.authoritative || page.offset != c.offset {
                    return Err(Error::Invalid("invalid_page"));
                }
                traversal.outcomes.push(outcome);
                let ids = page
                    .items
                    .iter()
                    .map(|i| i.external_id.clone())
                    .collect::<Vec<_>>();
                for item in page.items {
                    if item.id != adapter.public_id(&item.external_id) {
                        return Err(Error::Invalid("public_identity"));
                    }
                    self.full_write_or_touch(
                        run,
                        &root,
                        &mut traversal,
                        &mut items,
                        collection_write(&item, &root.external_id),
                    )
                    .await?;
                }
                (ids, page.next_offset)
            };
            let fingerprint = hex(&input_hash(
                &serde_json::to_string(&ids).map_err(|_| Error::Invalid("page"))?,
            ));
            if !ids.is_empty() && c.fingerprint.as_ref() == Some(&fingerprint) {
                return Err(Error::Invalid("repeated_page"));
            }
            traversal.primary_external_ids = ids;
            c.fingerprint = Some(fingerprint);
            let advance = if refreshing {
                if let Some(next) = next_offset {
                    if next <= c.refresh_offset {
                        return Err(Error::Invalid("descendant_cursor_loop"));
                    }
                    c.refresh_offset = next;
                    false
                } else {
                    c.refresh.remove(0);
                    c.refresh_offset = 0;
                    c.fingerprint = None;
                    c.refresh.is_empty()
                }
            } else {
                if !c.refresh.is_empty() {
                    c.fingerprint = None;
                }
                c.refresh.is_empty()
            };
            if advance {
                let next_offset = if refreshing {
                    c.delta_next
                } else {
                    next_offset
                };
                if let Some(next) = next_offset {
                    if next <= c.offset {
                        return Err(Error::Invalid("cursor_loop"));
                    }
                    c.offset = next;
                } else {
                    c.library = index + 1;
                    c.offset = 0;
                    c.fingerprint = None;
                    c.delta = None;
                }
            }
            return self
                .full_commit(
                    claim,
                    c,
                    vec![],
                    items,
                    vec![],
                    vec![],
                    followups,
                    traversal,
                    Completion::Continue,
                )
                .await;
        }
        if c.phase == 2 && c.container.is_none() {
            let next = self
                .ingest
                .observed_items(run.id, "collection", c.after.as_deref(), 1)
                .await?;
            if let Some(item) = next.first() {
                c.container = Some(item.external_id.clone());
                c.offset = 0;
                c.member_fingerprints.clear();
            } else {
                c.phase = 3;
                c.after = None;
                c.offset = 0;
            }
        }
        if c.phase == 2 {
            let container = c
                .container
                .clone()
                .ok_or(Error::Invalid("collection cursor"))?;
            c.operation = format!("members:{}:{}", hex(&input_hash(&container)), c.offset);
            let Some((page, outcome)) = self
                .full_fetch(
                    claim,
                    OperationClass::Collection,
                    adapter.collection_members_page(
                        &selection.operator_profile_id,
                        &container,
                        c.offset,
                        (max_records as u32).min(50),
                    ),
                )
                .await?
            else {
                return Ok(());
            };
            if page.authoritative || page.offset != c.offset {
                return Err(Error::Invalid("invalid_members_page"));
            }
            let mut ids = page.items.iter().map(|item| &item.id).collect::<Vec<_>>();
            ids.sort_unstable();
            let fingerprint = hex(&input_hash(
                &serde_json::to_string(&ids).map_err(|_| Error::Invalid("members page"))?,
            ));
            if !ids.is_empty() && c.member_fingerprints.contains(&fingerprint) {
                return Err(Error::Invalid("repeated_members_page"));
            }
            if ids.is_empty() && page.next_offset.is_some() {
                return Err(Error::Invalid("empty_members_page_with_cursor"));
            }
            if !ids.is_empty() {
                if c.member_fingerprints.len() == 32 {
                    c.member_fingerprints.remove(0);
                }
                c.member_fingerprints.push(fingerprint);
            }
            let mut traversal = TraversalWrites::default();
            let receipt = outcome.receipt_id;
            traversal.outcomes.push(outcome);
            for (ordinal, member) in page.items.iter().enumerate() {
                let known = self.full_known(run, &member.external_id).await?;
                let expected = match &member.kind {
                    MemberKind::Media(kind) => Some(crate::mapping::kind(*kind)),
                    MemberKind::Collection => Some("collection"),
                    MemberKind::Unsupported(_) => None,
                };
                if matches!(member.kind, MemberKind::Unsupported(_))
                    || member.id != adapter.public_id(&member.external_id)
                    || known.as_deref() != expected
                {
                    self.ingest
                        .record_unresolved_operation(
                            claim,
                            OperationClass::Collection,
                            &format!(
                                "member:{}:{}",
                                hex(&input_hash(&container)),
                                hex(&input_hash(&member.external_id))
                            ),
                        )
                        .await?;
                    continue;
                }
                traversal
                    .collection_members
                    .push(CollectionMembershipWrite {
                        container_external_id: container.clone(),
                        member_external_id: member.external_id.clone(),
                        ordinal: Some(
                            i32::try_from(u64::from(c.offset) + ordinal as u64)
                                .map_err(|_| Error::Invalid("member ordinal"))?,
                        ),
                        observation_receipt_id: receipt,
                    });
            }
            if let Some(next) = page.next_offset {
                if next <= c.offset {
                    return Err(Error::Invalid("members_cursor_loop"));
                }
                c.offset = next;
            } else {
                c.after = Some(container);
                c.container = None;
                c.offset = 0;
                c.member_fingerprints.clear();
            }
            return self
                .full_commit(
                    claim,
                    c,
                    vec![],
                    vec![],
                    vec![],
                    vec![],
                    vec![],
                    traversal,
                    Completion::Continue,
                )
                .await;
        }
        if c.phase == 3 {
            if screenroom_backend::indexing::configuration(run)?
                .ok_or(Error::Invalid("canonical run required"))?
                .components
                .artwork
            {
                return self.full_prepare_art(claim, run, &adapter, c).await;
            }
            c.phase = 4;
        }
        self.full_complete(claim, run, c).await
    }

    async fn full_complete(&self, claim: &Claim, run: &Run, c: &Cursor) -> Result<()> {
        let coverage = self.ingest.run_coverage(run.id).await?;
        if coverage.unresolved_operations > 0 {
            return Err(Error::Invalid("unresolved_traversal_operations"));
        }
        let evidence = coverage_value(&coverage, true);
        let completion = Completion::Complete { evidence };
        self.full_commit(
            claim,
            c,
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            TraversalWrites::default(),
            completion,
        )
        .await
    }

    async fn full_observed(&self, run: &Run, id: &str, library: &str) -> Result<bool> {
        Ok(sqlx::query_scalar("SELECT EXISTS(SELECT FROM ingest.item_observations WHERE run_id=$1 AND source_id=$2 AND external_id=$3 AND library_external_id=$4)").bind(run.id).bind(run.source_id).bind(id).bind(library).fetch_one(self.catalog.pool()).await?)
    }
    async fn full_known(&self, run: &Run, id: &str) -> Result<Option<String>> {
        Ok(sqlx::query_scalar("SELECT i.kind FROM catalog.source_items si JOIN catalog.items i ON i.id=si.item_id WHERE si.source_id=$2 AND si.external_id=$3 AND si.state='enabled' AND EXISTS(SELECT FROM ingest.item_observations o WHERE o.run_id=$1 AND o.source_id=si.source_id AND o.external_id=si.external_id)").bind(run.id).bind(run.source_id).bind(id).fetch_optional(self.catalog.pool()).await?)
    }
    async fn full_write_or_touch(
        &self,
        run: &Run,
        root: &Root,
        traversal: &mut TraversalWrites,
        items: &mut Vec<ItemWrite>,
        write: ItemWrite,
    ) -> Result<()> {
        let current:Option<Vec<u8>>=sqlx::query_scalar("SELECT content_hash FROM catalog.source_items WHERE source_id=$1 AND external_id=$2 AND state='enabled'").bind(run.source_id).bind(&write.external_id).fetch_optional(self.catalog.pool()).await?;
        let document_ready=current.as_ref()==Some(&write.content_hash) && (write.kind=="collection" || sqlx::query_scalar::<_,bool>("SELECT EXISTS(SELECT FROM search.documents d JOIN catalog.source_items si ON si.id=d.source_item_id WHERE si.source_id=$1 AND si.external_id=$2 AND d.content_hash=$3 AND d.document_version=$4)").bind(run.source_id).bind(&write.external_id).bind(input_hash(&write.document).as_slice()).bind(write.document_version).fetch_one(self.catalog.pool()).await?);
        if current.as_ref() == Some(&write.content_hash) && document_ready {
            traversal.library_touches.push(LibraryTouch {
                external_id: write.external_id,
                library_external_id: root.external_id.clone(),
            });
        } else {
            items.push(write);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn full_commit(
        &self,
        claim: &Claim,
        c: &Cursor,
        libraries: Vec<LibraryWrite>,
        items: Vec<ItemWrite>,
        artwork_inputs: Vec<ArtworkInputWrite>,
        artwork_omissions: Vec<ArtworkOmission>,
        followups: Vec<Followup>,
        writes: TraversalWrites,
        completion: Completion,
    ) -> Result<()> {
        self.ingest
            .commit_traversal_page(
                claim,
                &PageCommit {
                    libraries,
                    items,
                    artwork_inputs,
                    artwork_omissions,
                    followups,
                    checkpoint: CheckpointWrite {
                        scope_key: SCOPE.into(),
                        cursor: serde_json::to_value(c)
                            .map_err(|_| Error::Invalid("checkpoint"))?,
                        watermark: None,
                    },
                    requests: 0,
                    completion,
                },
                &writes,
            )
            .await
    }

    async fn full_prepare_art(
        &self,
        claim: &Claim,
        run: &Run,
        adapter: &jellyfin::LiveJellyfin,
        c: &mut Cursor,
    ) -> Result<()> {
        let (external_id, inputs, input) = loop {
            let next:Option<(String,Value)>=sqlx::query_as("SELECT si.external_id,si.metadata FROM catalog.source_items si WHERE si.source_id=$1 AND si.state='enabled' AND ($3::text IS NULL OR si.external_id>$3) AND jsonb_typeof(si.metadata->'provider_artwork_inputs')='array' AND jsonb_array_length(si.metadata->'provider_artwork_inputs')>0 AND EXISTS(SELECT FROM ingest.item_observations o WHERE o.run_id=$2 AND o.source_id=si.source_id AND o.external_id=si.external_id) ORDER BY si.external_id LIMIT 1")
            .bind(run.source_id).bind(run.id).bind(&c.after).fetch_optional(self.catalog.pool()).await?;
            let Some((external_id, metadata)) = next else {
                c.phase = 4;
                return self.full_complete(claim, run, c).await;
            };
            let inputs = metadata["provider_artwork_inputs"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let Some(input) = inputs.get(c.art_index).cloned() else {
                c.after = Some(external_id.clone());
                c.art_index = 0;
                continue;
            };
            break (external_id, inputs, input);
        };
        let input = &input;
        let art = live::parse_artwork(&json!({"provider_input":input}))?;
        let key = live::artwork_key(&art);
        c.operation = format!("artwork:{}:{}", hex(&input_hash(&external_id)), c.art_index);
        c.art_index += 1;
        let (ready, mut outcomes) = self
            .full_art_ready(claim, run.source_id, &key, input)
            .await?;
        if ready {
            if c.art_index >= inputs.len() {
                c.after = Some(external_id.clone());
                c.art_index = 0;
            }
            return self
                .full_commit(
                    claim,
                    c,
                    vec![],
                    vec![],
                    vec![],
                    vec![],
                    vec![],
                    TraversalWrites {
                        outcomes,
                        ..Default::default()
                    },
                    Completion::Continue,
                )
                .await;
        }
        let receipt = self
            .ingest
            .reserve_work_request(claim, OperationClass::Artwork)
            .await?;
        let fetched = tokio::time::timeout(Duration::from_secs(10), adapter.artwork_bytes(&art))
            .await
            .unwrap_or(Err(jellyfin::Error::Deadline));
        let (inputs, omissions, outcome) = match fetched {
            Ok(bytes) => match ValidatedImage::raster(bytes) {
                Ok(image) => {
                    if !outcomes.is_empty() {
                        self.full_verified_upload(claim, run.source_id, &image, &mut outcomes)
                            .await?;
                    }
                    (
                        vec![live::artwork_input(&art, &image)],
                        vec![],
                        RequestOutcomeWrite {
                            receipt_id: receipt.id,
                            outcome: RequestOutcome::Success,
                            error_class: None,
                        },
                    )
                }
                Err(_) => (
                    vec![],
                    vec![ArtworkOmission {
                        source_artwork_id: key,
                        code: ArtworkOmissionCode::Invalid,
                    }],
                    RequestOutcomeWrite {
                        receipt_id: receipt.id,
                        outcome: RequestOutcome::Permanent,
                        error_class: Some(RequestErrorClass::Invalid),
                    },
                ),
            },
            Err(jellyfin::Error::RateLimited {
                retry_after_seconds,
            }) => {
                c.art_index -= 1;
                let outcome = RequestOutcomeWrite {
                    receipt_id: receipt.id,
                    outcome: RequestOutcome::Retryable,
                    error_class: Some(RequestErrorClass::Throttled),
                };
                let delay = retry_after_seconds.unwrap_or(60).clamp(1, 86400);
                return self.ingest.defer_work(claim, &outcome, delay as i32).await;
            }
            Err(
                error @ (jellyfin::Error::Transport
                | jellyfin::Error::Deadline
                | jellyfin::Error::Http(500..=599)),
            ) => {
                c.art_index -= 1;
                self.ingest
                    .record_work_outcome(
                        claim,
                        &RequestOutcomeWrite {
                            receipt_id: receipt.id,
                            outcome: RequestOutcome::Retryable,
                            error_class: Some(provider_failure(&error).0),
                        },
                    )
                    .await?;
                return self
                    .ingest
                    .fail(
                        claim,
                        screenroom_backend::queue::FailureClass::Transient,
                        "artwork_fetch_retry",
                        None,
                    )
                    .await;
            }
            Err(jellyfin::Error::Unauthorized) => {
                self.ingest
                    .record_work_outcome(
                        claim,
                        &RequestOutcomeWrite {
                            receipt_id: receipt.id,
                            outcome: RequestOutcome::Permanent,
                            error_class: Some(RequestErrorClass::Unauthorized),
                        },
                    )
                    .await?;
                return Err(Error::Invalid("provider_unauthorized"));
            }
            Err(error) => (
                vec![],
                vec![ArtworkOmission {
                    source_artwork_id: key,
                    code: if matches!(error, jellyfin::Error::NotFound) {
                        ArtworkOmissionCode::Missing
                    } else {
                        ArtworkOmissionCode::Unavailable
                    },
                }],
                RequestOutcomeWrite {
                    receipt_id: receipt.id,
                    outcome: RequestOutcome::Permanent,
                    error_class: Some(RequestErrorClass::Invalid),
                },
            ),
        };
        outcomes.push(outcome);
        if c.art_index >= inputs.len() {
            c.after = Some(external_id);
            c.art_index = 0;
        }
        self.full_commit(
            claim,
            c,
            vec![],
            vec![],
            inputs,
            omissions,
            vec![],
            TraversalWrites {
                outcomes,
                ..Default::default()
            },
            Completion::Continue,
        )
        .await
    }

    pub(crate) async fn full_art_ready(
        &self,
        claim: &Claim,
        source: Uuid,
        key: &str,
        input: &Value,
    ) -> Result<(bool, Vec<RequestOutcomeWrite>)> {
        let rows:Vec<(String,String,Vec<u8>,String,i64)>=sqlx::query_as("SELECT o.bucket,o.object_key,o.sha256,o.mime,o.byte_length FROM media.artwork a JOIN media.artwork_generations g ON g.artwork_id=a.id AND g.revision=a.current_generation JOIN media.variants v ON v.artwork_id=g.artwork_id AND v.revision=g.revision JOIN media.objects o ON o.id=v.object_id WHERE a.source_id=$1 AND a.source_artwork_id=$2 AND g.state='published' AND g.rights->'provider_input'=$3 AND o.state='ready' AND v.variant IN ('original','native')").bind(source).bind(key).bind(input).fetch_all(self.catalog.pool()).await?;
        if rows.len() != 2 {
            return Ok((false, vec![]));
        }
        let mut outcomes = vec![];
        for (bucket, object_key, sha256, mime, byte_length) in rows {
            let receipt = match self
                .ingest
                .reserve_work_request(claim, OperationClass::Artwork)
                .await
            {
                Ok(receipt) => receipt,
                Err(error) => {
                    for outcome in &outcomes {
                        self.ingest.record_work_outcome(claim, outcome).await?;
                    }
                    return Err(error);
                }
            };
            if self
                .transport(claim)
                .storage(&self.storage)
                .head(&ObjectLocation {
                    bucket,
                    object_key,
                    sha256,
                    mime,
                    byte_length,
                })
                .await
                .is_err()
            {
                outcomes.push(RequestOutcomeWrite {
                    receipt_id: receipt.id,
                    outcome: RequestOutcome::Retryable,
                    error_class: Some(RequestErrorClass::Unavailable),
                });
                return Ok((false, outcomes));
            }
            outcomes.push(RequestOutcomeWrite {
                receipt_id: receipt.id,
                outcome: RequestOutcome::Success,
                error_class: None,
            });
        }
        Ok((true, outcomes))
    }

    async fn full_embedding(&self, claim: &Claim, source: &str) -> Result<()> {
        let external = claim.input["external_id"]
            .as_str()
            .ok_or(Error::Invalid("embedding job"))?;
        let model: Uuid = serde_json::from_value(claim.input["model_id"].clone())
            .map_err(|_| Error::Invalid("embedding model"))?;
        let configured_model = self
            .model
            .as_ref()
            .ok_or(Error::Invalid("embedding not configured"))?;
        if model != configured_model.id {
            return Err(Error::Incompatible("worker model revision"));
        }
        let Some(doc) = self.search.document_for_reference(source, external).await? else {
            return self.ingest.complete_job(claim).await;
        };
        if claim.input["input_hash"].as_str() != Some(hex(&doc.content_hash).as_str()) {
            return self.ingest.complete_job(claim).await;
        }
        let ready:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM search.embeddings WHERE document_id=$1 AND model_id=$2 AND input_hash=$3)").bind(doc.id).bind(model).bind(&doc.content_hash).fetch_one(self.catalog.pool()).await?;
        if ready {
            return self.ingest.complete_job(claim).await;
        }
        let receipt = self
            .ingest
            .reserve_work_request(claim, OperationClass::Embedding)
            .await?;
        let embedding = self.transport(claim).embedding(
            self.embedding
                .as_ref()
                .ok_or(Error::Invalid("embedding not configured"))?,
        );
        let batch = match tokio::time::timeout(
            Duration::from_secs(10),
            embedding.embed(std::slice::from_ref(&doc.text)),
        )
        .await
        .unwrap_or(Err(Error::Unavailable))
        {
            Ok(batch) => batch,
            Err(error) => {
                let permanent = matches!(error, Error::Invalid(_) | Error::Incompatible(_));
                self.ingest
                    .record_work_outcome(
                        claim,
                        &RequestOutcomeWrite {
                            receipt_id: receipt.id,
                            outcome: if permanent {
                                RequestOutcome::Permanent
                            } else {
                                RequestOutcome::Retryable
                            },
                            error_class: Some(if permanent {
                                RequestErrorClass::Invalid
                            } else {
                                RequestErrorClass::Unavailable
                            }),
                        },
                    )
                    .await?;
                if permanent {
                    return Err(Error::Incompatible("embedding_invalid_response"));
                }
                return self
                    .ingest
                    .fail(
                        claim,
                        screenroom_backend::queue::FailureClass::Transient,
                        "embedding_retry",
                        None,
                    )
                    .await;
            }
        };
        self.search
            .commit_embedding_with_outcomes(
                claim,
                doc.id,
                model,
                &configured_model.revision,
                &doc.content_hash,
                &batch.vectors[0],
                &[RequestOutcomeWrite {
                    receipt_id: receipt.id,
                    outcome: RequestOutcome::Success,
                    error_class: None,
                }],
            )
            .await
    }

    pub(crate) async fn full_artwork(&self, claim: &Claim, run: &Run, source: &str) -> Result<()> {
        let job: ArtworkJob = serde_json::from_value(claim.input.clone())
            .map_err(|_| Error::Invalid("artwork job"))?;
        if job.source_key != source {
            return Err(Error::Invalid("artwork source"));
        }
        let rights:Value=sqlx::query_scalar("SELECT rights FROM media.artwork_generations WHERE artwork_id=$1 AND revision=$2 AND fingerprint=$3").bind(job.artwork_id).bind(job.revision).bind(job.fingerprint.as_slice()).fetch_one(self.catalog.pool()).await?;
        let art = live::parse_artwork(&rights)?;
        if live::artwork_key(&art) != job.source_artwork_id {
            return Err(Error::Invalid("artwork identity"));
        }
        let Some(adapter) = self.provider(claim, source).await? else {
            return Ok(());
        };
        let Some((bytes, outcome)) = self
            .full_fetch(claim, OperationClass::Artwork, adapter.artwork_bytes(&art))
            .await?
        else {
            return Ok(());
        };
        let image = match ValidatedImage::raster(bytes) {
            Ok(image) if <[u8; 32]>::from(Sha256::digest(image.bytes())) == job.expected_sha256 => {
                image
            }
            _ => {
                self.ingest
                    .record_work_outcome(
                        claim,
                        &RequestOutcomeWrite {
                            receipt_id: outcome.receipt_id,
                            outcome: RequestOutcome::Permanent,
                            error_class: Some(RequestErrorClass::Invalid),
                        },
                    )
                    .await?;
                return Err(Error::Incompatible("artwork_input_invalid_or_changed"));
            }
        };
        let mut outcomes = vec![outcome];
        let verified = self
            .full_verified_upload(claim, run.source_id, &image, &mut outcomes)
            .await?;
        self.media
            .publish_with_outcomes(claim, &verified, &outcomes)
            .await?;
        Ok(())
    }

    pub(crate) async fn full_verified_upload(
        &self,
        claim: &Claim,
        source: Uuid,
        image: &ValidatedImage,
        outcomes: &mut Vec<RequestOutcomeWrite>,
    ) -> Result<screenroom_backend::storage::VerifiedObject> {
        for outcome in outcomes.drain(..) {
            self.ingest.record_work_outcome(claim, &outcome).await?;
        }
        let mut receipts = vec![];
        for _ in 0..3 {
            let receipt = self
                .ingest
                .reserve_work_request(claim, OperationClass::Artwork)
                .await?;
            receipts.push(receipt.id);
        }
        let result = self
            .transport(claim)
            .storage(&self.storage)
            .upload_verified(source, image)
            .await;
        for receipt_id in receipts {
            let outcome = RequestOutcomeWrite {
                receipt_id,
                outcome: if result.is_ok() {
                    RequestOutcome::Success
                } else {
                    RequestOutcome::Retryable
                },
                error_class: result
                    .as_ref()
                    .err()
                    .map(|_| RequestErrorClass::Unavailable),
            };
            self.ingest.record_work_outcome(claim, &outcome).await?;
        }
        result
    }
}

fn collection_write(item: &Collection, library: &str) -> ItemWrite {
    let metadata = json!({"id":item.id,"kind":"collection","title":item.title,"overview":item.overview,"year":item.year,"provider_ids":item.provider_ids,"metadata_rights":null,"title_rights":null,"rating":null,"credits":[],"provider_artwork_inputs":item.artwork.iter().filter(|a|live::valid_artwork(a)).map(|a|json!({"item_id":a.item_id,"kind":a.kind,"tag":a.tag})).collect::<Vec<_>>()});
    ItemWrite {
        public_id: item.id.clone(),
        kind: "collection".into(),
        parent_public_id: None,
        external_id: item.external_id.clone(),
        source_revision: None,
        content_hash: input_hash(&metadata.to_string()).to_vec(),
        metadata,
        provenance: json!({"provider":"jellyfin","license":null}),
        library_external_ids: vec![library.into()],
        external_ids: item
            .provider_ids
            .iter()
            .map(|(provider, id)| ExternalIdWrite {
                provider: provider.clone(),
                entity_kind: "collection".into(),
                external_id: id.clone(),
                evidence: json!({"provider":"jellyfin"}),
            })
            .collect(),
        credits: vec![],
        document_title: String::new(),
        document: String::new(),
        document_version: screenroom_backend::embedding::DOCUMENT_VERSION,
    }
}
pub(crate) fn provider_failure(error: &jellyfin::Error) -> (RequestErrorClass, bool) {
    match error {
        jellyfin::Error::RateLimited { .. } => (RequestErrorClass::Throttled, true),
        jellyfin::Error::Transport => (RequestErrorClass::Transport, true),
        jellyfin::Error::Deadline => (RequestErrorClass::Timeout, true),
        jellyfin::Error::Http(500..=599) => (RequestErrorClass::Unavailable, true),
        jellyfin::Error::Unauthorized => (RequestErrorClass::Unauthorized, false),
        _ => (RequestErrorClass::Invalid, false),
    }
}
fn check_parent(child: MediaKind, parent: &str) -> Result<()> {
    if matches!(
        (child, parent),
        (MediaKind::Episode, "season") | (MediaKind::Season, "series")
    ) {
        Ok(())
    } else {
        Err(Error::Invalid("invalid_hierarchy"))
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
