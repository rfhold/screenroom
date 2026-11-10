use crate::mapping;
use screenroom_backend::{
    Error, Result, Uuid,
    catalog::CatalogRepository,
    embedding::EmbeddingClient,
    ingest::{
        CheckpointWrite, Claim, Completion, FeedMembershipWrite, Followup, IngestRepository,
        JobKind, LibraryWrite, PageCommit, Policy, Run, selected_library_ids,
    },
    media::{ArtworkJob, MediaRepository},
    search::{Model, SearchRepository},
    sqlx::PgPool,
    storage::ObjectStorage,
};
use screenroom_catalog_adapter::{
    Coverage, Feed, FeedPolicy, MediaKind, MockAdapters, ProviderId, TrendingWindow,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::Duration;

const CHECKPOINT: &str = "traversal-v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Idle,
    Committed,
    Retried,
    Stale,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedSelection {
    pub feed_key: String,
    #[serde(flatten)]
    pub policy: FeedPolicy,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Progress {
    feed: usize,
    cursor: Option<String>,
    traversed: usize,
    #[serde(default)]
    evidence: Vec<Value>,
    #[serde(default)]
    partial: bool,
    failures: u32,
    error_code: Option<String>,
    seen_pages: Vec<String>,
    feed_done: bool,
    children: Vec<ChildTraversal>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChildTraversal {
    parent: ProviderId,
    cursor: Option<String>,
    traversed: usize,
    seen_pages: Vec<String>,
}

pub struct Worker {
    ingest: IngestRepository,
    catalog: CatalogRepository,
    search: SearchRepository,
    media: MediaRepository,
    adapters: MockAdapters,
    embedding: EmbeddingClient,
    storage: ObjectStorage,
    model: Model,
    owner: String,
}

impl Worker {
    pub async fn new(
        pool: PgPool,
        adapters: MockAdapters,
        embedding: EmbeddingClient,
        storage: ObjectStorage,
        owner: String,
    ) -> Result<Self> {
        if owner.is_empty()
            || owner.len() > 200
            || embedding.config().timeout > Duration::from_secs(30)
        {
            return Err(Error::Invalid("worker configuration"));
        }
        let search = SearchRepository::new(pool.clone());
        let c = embedding.config();
        let model = search.register_model(&c.model, &c.revision, 1024).await?;
        Ok(Self {
            ingest: IngestRepository::new(pool.clone()),
            catalog: CatalogRepository::new(pool.clone()),
            media: MediaRepository::new(pool),
            search,
            adapters,
            embedding,
            storage,
            model,
            owner,
        })
    }

    /// Starts only persisted due policies; another policy holding this source is normal.
    pub async fn start_due(&self) -> Result<usize> {
        let mut started = 0;
        for policy in self.ingest.due_policies(100).await? {
            match self.ingest.start_run(policy.id).await {
                Ok(_) => started += 1,
                Err(Error::SourceBusy { .. } | Error::Stale) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(started)
    }

    pub async fn resume(&self, run: Uuid) -> Result<()> {
        self.ingest.resume(run).await
    }

    /// Activation is an explicit operator action, never an incidental end-of-drain side effect.
    pub async fn activate_model(&self) -> Result<()> {
        self.search.activate_model(self.model.id).await
    }

    pub async fn step(&self) -> Result<Step> {
        let Some(claim) = self.ingest.claim(&self.owner, 300).await? else {
            return Ok(Step::Idle);
        };
        self.ingest.renew(&claim, 300).await?;
        let result = tokio::time::timeout(Duration::from_secs(240), self.process(&claim))
            .await
            .unwrap_or(Err(Error::Unavailable));
        match result {
            Ok(()) => Ok(Step::Committed),
            Err(Error::Stale) => Ok(Step::Stale),
            Err(e) => {
                let code = match e {
                    Error::Unavailable => "upstream_unavailable",
                    Error::Incompatible(_) => "incompatible_input",
                    Error::Invalid(_) | Error::NotFound => "invalid_input",
                    Error::Database(_) => "database_unavailable",
                    Error::SourceBusy { .. } => "source_busy",
                    Error::Stale => unreachable!(),
                };
                match self.ingest.retry(&claim, code, 5).await {
                    Ok(()) => Ok(Step::Retried),
                    Err(Error::Stale) => Ok(Step::Stale),
                    Err(e) => Err(e),
                }
            }
        }
    }

    async fn process(&self, claim: &Claim) -> Result<()> {
        let run = self
            .ingest
            .run(claim.run_id.ok_or(Error::Stale)?)
            .await?
            .ok_or(Error::Stale)?;
        let policy: Policy = serde_json::from_value(run.policy_snapshot.clone())
            .map_err(|_| Error::Invalid("policy snapshot"))?;
        let source = self
            .catalog
            .sources()
            .await?
            .into_iter()
            .find(|s| s.id == run.source_id)
            .ok_or(Error::Stale)?;
        if source.configuration.get("adapter").and_then(Value::as_str) != Some("mock") {
            return Err(Error::Invalid("offline mock adapter required"));
        }
        match claim.kind.as_str() {
            "ingest" => {
                self.page(claim, &run, &policy, &source.source_key, &source.provider)
                    .await
            }
            "embedding" => self.embed(claim, &source.source_key).await,
            "artwork" => self.artwork(claim, run.source_id).await,
            _ => Err(Error::Invalid("job kind")),
        }
    }

    async fn page(
        &self,
        claim: &Claim,
        run: &Run,
        policy: &Policy,
        source: &str,
        provider: &str,
    ) -> Result<()> {
        let state = self
            .ingest
            .run_checkpoint(run, CHECKPOINT)
            .await?
            .unwrap_or(json!({}));
        let mut progress: Progress =
            serde_json::from_value(state).map_err(|_| Error::Invalid("checkpoint"))?;
        if let Some(code @ ("cursor_loop" | "repeated_page")) = progress.error_code.as_deref() {
            return self.ingest.retry(claim, code, 0).await;
        }
        let mut feeds: Vec<FeedSelection> = vec![];
        let libraries = if provider == "jellyfin" {
            selected_library_ids(&run.selection_snapshot)?
        } else {
            vec![]
        };
        let size = if provider == "tmdb" {
            feeds = feed_selections(&run.selection_snapshot, policy.max_items)?;
            if feeds.is_empty() || progress.feed >= feeds.len() {
                return Err(Error::Invalid("feed checkpoint"));
            }
            feeds[progress.feed].policy.page_size
        } else if provider == "jellyfin" {
            run.selection_snapshot
                .get("page_size")
                .and_then(Value::as_u64)
                .unwrap_or(policy.max_items.min(25) as u64) as usize
        } else {
            return Err(Error::Invalid("provider"));
        };
        if !(1..=100).contains(&size) {
            return Err(Error::Invalid("page size"));
        }
        // Reserve linked lookups before fetching: no extra adapter request after a tranche is spent.
        let reserve = 1 + if policy.enrich_metadata && provider == "jellyfin" {
            size as i64
        } else {
            0
        };
        if policy.max_items < size as i64 || policy.max_requests < reserve {
            return Err(Error::Invalid("policy budget smaller than one page"));
        }
        if run.budget_items + size as i64 > policy.max_items
            || run.budget_requests + reserve > policy.max_requests
        {
            return self
                .ingest
                .commit_page(
                    claim,
                    &PageCommit {
                        libraries: vec![],
                        items: vec![],
                        feeds: vec![],
                        artwork_inputs: vec![],
                        artwork_omissions: vec![],
                        followups: vec![],
                        checkpoint: checkpoint(&progress)?,
                        requests: 0,
                        completion: Completion::Pause,
                    },
                )
                .await;
        }
        let child_page = provider == "tmdb" && !progress.children.is_empty();
        let (cursor, traversed, seen_pages) = if child_page {
            let child = &progress.children[0];
            (
                child.cursor.clone(),
                child.traversed,
                child.seen_pages.clone(),
            )
        } else {
            (
                progress.cursor.clone(),
                progress.traversed,
                progress.seen_pages.clone(),
            )
        };
        let response = if provider == "jellyfin" {
            self.adapters
                .jellyfin_page(&libraries, size, cursor.as_deref())
        } else if child_page {
            self.adapters
                .tmdb_children(&progress.children[0].parent, size, cursor.as_deref())
        } else {
            self.adapters
                .tmdb_page(&feeds[progress.feed].policy, cursor.as_deref())
        };
        let page = match response {
            Ok(page) => page,
            Err(error) => {
                use screenroom_catalog_adapter::Error as A;
                let (code, retry, delay) = match error {
                    A::RateLimited {
                        retry_after_seconds,
                    } => ("rate_limited", true, retry_after_seconds.min(30)),
                    A::Unavailable => ("adapter_unavailable", true, 1),
                    A::CursorExpired | A::InvalidCursor => ("cursor_invalid", false, 0),
                    _ => ("adapter_invalid", false, 0),
                };
                progress.failures += 1;
                progress.error_code = Some(code.into());
                if delay > 0 {
                    tokio::time::sleep(Duration::from_secs(u64::from(delay))).await;
                }
                // Failed provider requests consume the tranche too. Never advance the cursor.
                return self
                    .ingest
                    .commit_page(
                        claim,
                        &PageCommit {
                            libraries: vec![],
                            items: vec![],
                            feeds: vec![],
                            artwork_inputs: vec![],
                            artwork_omissions: vec![],
                            followups: vec![],
                            checkpoint: checkpoint(&progress)?,
                            requests: 1,
                            completion: if retry && progress.failures < 5 {
                                Completion::Continue
                            } else {
                                Completion::Pause
                            },
                        },
                    )
                    .await;
            }
        };
        progress.failures = 0;
        progress.error_code = None;
        let evidence = json!({"source":source,"scope":page.evidence.scope,"snapshot":page.evidence.snapshot,
            "traversed":page.evidence.traversed,"total_in_scope":page.evidence.total_in_scope});
        let fingerprint = hex(&screenroom_backend::embedding::input_hash(
            &serde_json::to_string(
                &page
                    .items
                    .iter()
                    .map(|entry| &entry.reference.external_id)
                    .collect::<Vec<_>>(),
            )
            .map_err(|_| Error::Invalid("page identity"))?,
        ));
        let corruption = if page.next_cursor.is_some() && page.next_cursor == cursor {
            Some("cursor_loop")
        } else if !page.items.is_empty()
            && (seen_pages.contains(&fingerprint)
                || page.items.iter().any(|entry| entry.ordinal < traversed)
                || page.evidence.traversed < traversed)
        {
            Some("repeated_page")
        } else if page.next_cursor.is_some() && page.evidence.traversed <= traversed {
            Some("cursor_loop")
        } else {
            None
        };
        if let Some(code) = corruption {
            progress.error_code = Some(code.into());
            return self
                .ingest
                .commit_page(
                    claim,
                    &PageCommit {
                        libraries: vec![],
                        items: vec![],
                        feeds: vec![],
                        artwork_inputs: vec![],
                        artwork_omissions: vec![],
                        followups: vec![],
                        checkpoint: checkpoint(&progress)?,
                        requests: 1,
                        completion: Completion::Continue,
                    },
                )
                .await;
        }
        progress.partial |= page.coverage == Coverage::Partial;
        let mut requests = 1;
        let mut items = vec![];
        let mut memberships = vec![];
        let mut refs = vec![];
        let mut omissions = vec![];
        let mut jobs = vec![];
        if provider == "tmdb"
            && progress.feed == 0
            && !child_page
            && progress.cursor.is_none()
            && policy.enrich_artwork
        {
            let (inputs, missing) = mapping::branding(&self.adapters)?;
            refs.extend(inputs);
            omissions.extend(missing);
        }
        let mut discovered = vec![];
        for mut entry in page.items {
            entry.reference.source = source.into();
            if provider == "jellyfin" && policy.enrich_metadata {
                // A typed provider identity, not a display/provenance URL, authorizes lookup.
                if let Some(id) = entry.item.provider_ids.first() {
                    requests += 1;
                    let linked = self
                        .adapters
                        .tmdb_lookup(id)
                        .ok_or(Error::Invalid("linked provider item"))?;
                    if linked.item.id != entry.item.id {
                        return Err(Error::Invalid("linked identity mismatch"));
                    }
                    entry.item = linked.item;
                }
            }
            if provider == "tmdb" && !child_page {
                memberships.push(FeedMembershipWrite {
                    feed_key: feeds[progress.feed].feed_key.clone(),
                    external_id: entry.reference.external_id.clone(),
                    ordinal: entry.ordinal as i32,
                });
            }
            if provider == "tmdb"
                && matches!(entry.item.kind, MediaKind::Series | MediaKind::Season)
            {
                let parent = entry
                    .item
                    .provider_ids
                    .first()
                    .cloned()
                    .ok_or(Error::Invalid("hierarchy identity"))?;
                discovered.push(ChildTraversal {
                    parent,
                    cursor: None,
                    traversed: 0,
                    seen_pages: vec![],
                });
            }
            let mut write = mapping::item(&entry, &page.evidence.snapshot, policy.enrich_people)?;
            write.provenance["notices"] = serde_json::to_value(self.adapters.notices())
                .map_err(|_| Error::Invalid("provider notices"))?;
            let hash = screenroom_backend::embedding::input_hash(&write.document);
            jobs.push(Followup {
                kind: JobKind::Embedding,
                dedupe_key: format!(
                    "embed:{}:{}:{}:{}",
                    run.source_id,
                    entry.reference.external_id,
                    self.model.id,
                    hex(&hash)
                ),
                input: json!({"external_id":entry.reference.external_id,"model_id":self.model.id}),
            });
            if policy.enrich_artwork {
                let (a, missing) = mapping::artwork(&self.adapters, &entry, policy.enrich_people)?;
                refs.extend(a);
                omissions.extend(missing);
            }
            items.push(write);
        }
        items.sort_by_key(|i| match i.kind.as_str() {
            "movie" | "series" => 0,
            "season" => 1,
            _ => 2,
        });
        let terminal = page.next_cursor.is_none();
        let mut seen_pages = seen_pages;
        seen_pages.push(fingerprint);
        if seen_pages.len() > 16 {
            seen_pages.remove(0);
        }
        if child_page {
            if terminal {
                progress.children.remove(0);
            } else {
                let child = &mut progress.children[0];
                child.cursor = page.next_cursor;
                child.traversed = page.evidence.traversed;
                child.seen_pages = seen_pages;
            }
        } else {
            progress.traversed = page.evidence.traversed;
            progress.cursor = page.next_cursor;
            progress.seen_pages = seen_pages;
            if terminal {
                progress.feed_done = true;
                progress.evidence.push(evidence);
            }
        }
        progress.children.splice(0..0, discovered);
        if progress.children.len() > 200 {
            return Err(Error::Invalid("hierarchy frontier limit"));
        }
        let scope_done = if provider == "tmdb" {
            progress.feed_done && progress.children.is_empty()
        } else {
            terminal
        };
        let completion = if scope_done {
            if provider == "tmdb" && progress.feed + 1 < feeds.len() {
                progress.feed += 1;
                progress.cursor = None;
                progress.traversed = 0;
                progress.seen_pages.clear();
                progress.feed_done = false;
                Completion::Continue
            } else if progress.partial {
                Completion::Complete {
                    evidence: json!({"partial":true,"scopes":progress.evidence}),
                }
            } else if provider == "jellyfin" && page.coverage == Coverage::SelectedLibrariesComplete
            {
                Completion::SelectedLibrariesComplete {
                    library_ids: libraries.clone(),
                    evidence: json!({"scopes":progress.evidence}),
                }
            } else if provider == "tmdb" {
                Completion::FeedWindowComplete {
                    feed_keys: feeds.iter().map(|f| f.feed_key.clone()).collect(),
                    evidence: json!({"scopes":progress.evidence}),
                }
            } else {
                Completion::Complete {
                    evidence: json!({"partial":true,"scopes":progress.evidence}),
                }
            }
        } else {
            Completion::Continue
        };
        self.ingest
            .commit_page(
                claim,
                &PageCommit {
                    libraries: self
                        .adapters
                        .libraries()
                        .into_iter()
                        .filter(|l| libraries.contains(&l.id))
                        .map(|l| LibraryWrite {
                            public_id: l.id.clone(),
                            external_id: l.id,
                            title: l.name,
                        })
                        .collect(),
                    items,
                    feeds: memberships,
                    artwork_inputs: refs,
                    artwork_omissions: omissions,
                    followups: jobs,
                    checkpoint: checkpoint(&progress)?,
                    requests,
                    completion,
                },
            )
            .await
    }

    async fn embed(&self, claim: &Claim, source: &str) -> Result<()> {
        let external = claim
            .input
            .get("external_id")
            .and_then(Value::as_str)
            .ok_or(Error::Invalid("embedding job"))?;
        let model: Uuid = serde_json::from_value(
            claim
                .input
                .get("model_id")
                .cloned()
                .ok_or(Error::Invalid("embedding model"))?,
        )
        .map_err(|_| Error::Invalid("embedding model"))?;
        if model != self.model.id {
            return Err(Error::Incompatible("worker model revision"));
        }
        let Some(doc) = self.search.document_for_reference(source, external).await? else {
            return self.ingest.complete_job(claim).await;
        };
        let batch = self
            .embedding
            .embed(std::slice::from_ref(&doc.text))
            .await?;
        self.search
            .commit_embedding(
                claim,
                doc.id,
                self.model.id,
                &self.model.revision,
                &doc.content_hash,
                &batch.vectors[0],
            )
            .await
    }

    async fn artwork(&self, claim: &Claim, source: Uuid) -> Result<()> {
        let job: ArtworkJob = serde_json::from_value(claim.input.clone())
            .map_err(|_| Error::Invalid("artwork job"))?;
        let image = mapping::artwork_image(&self.adapters, &job.source_artwork_id, job.variant)?;
        let actual: [u8; 32] = Sha256::digest(image.bytes()).into();
        if actual != job.expected_sha256 {
            return Err(Error::Incompatible("artwork input changed"));
        }
        let verified = self.storage.upload_verified(source, &image).await?;
        self.media.publish(claim, &verified).await?;
        Ok(())
    }
}

fn checkpoint(progress: &Progress) -> Result<CheckpointWrite> {
    Ok(CheckpointWrite {
        scope_key: CHECKPOINT.into(),
        cursor: serde_json::to_value(progress).map_err(|_| Error::Invalid("checkpoint"))?,
        watermark: None,
    })
}
fn feed_selections(selection: &Value, max_items: i64) -> Result<Vec<FeedSelection>> {
    selection
        .get("feeds")
        .and_then(Value::as_array)
        .ok_or(Error::Invalid("feeds"))?
        .iter()
        .map(|value| {
            if let Some(key) = value.as_str() {
                let feed: Feed =
                    serde_json::from_value(json!(key)).map_err(|_| Error::Invalid("feed name"))?;
                Ok(FeedSelection {
                    feed_key: key.into(),
                    policy: FeedPolicy {
                        feed,
                        max_pages: 1,
                        page_size: max_items.clamp(1, 25) as usize,
                        locale: "en-US".into(),
                        region: None,
                        trending_window: matches!(feed, Feed::MovieTrending | Feed::TvTrending)
                            .then_some(TrendingWindow::Day),
                    },
                })
            } else {
                serde_json::from_value(value.clone()).map_err(|_| Error::Invalid("feed policy"))
            }
        })
        .collect()
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
