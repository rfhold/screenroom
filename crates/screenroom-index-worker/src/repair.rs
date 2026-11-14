use crate::{Worker, live};
use screenroom_backend::{
    Error, Result,
    ingest::{
        CheckpointWrite, Claim, Completion, OperationClass, PageCommit, RequestErrorClass,
        RequestOutcome, RequestOutcomeWrite, Run, WorkBudget,
    },
    media::{ArtworkInputWrite, ArtworkReferenceInput, ValidatedImage},
    repair::{
        Candidate, CandidateWrite, Disposition, OutcomeCode, RepairExecution, RepairRepository,
        RepairWrites,
    },
    sqlx,
};
use screenroom_catalog_adapter::jellyfin;
use serde_json::{Value, json};
use std::time::Duration;

enum Fetch<T> {
    Value(T),
    Deferred,
    Failed(OutcomeCode),
}

impl Worker {
    pub(crate) async fn repair_process(
        &self,
        claim: &Claim,
        run: &Run,
        source: &str,
    ) -> Result<()> {
        let selection = RepairExecution::from_value(&run.selection_snapshot)?;
        let WorkBudget::PerWorkCommit { max_requests, .. } = selection.work_budget;
        // A repair needs lookup, two HEADs, fetch, and three upload operations.
        if max_requests < 7 {
            return Err(Error::Invalid(
                "maintenance requires seven requests per work commit",
            ));
        }
        match claim.kind.as_str() {
            "artwork" => return self.full_artwork(claim, run, source).await,
            "ingest" => {}
            _ => return Err(Error::Invalid("maintenance job kind")),
        }
        let repo = RepairRepository::new(self.catalog.pool().clone());
        let candidate = repo
            .candidates(run.id, None, 1, true)
            .await?
            .into_iter()
            .next();
        let Some(candidate) = candidate else {
            return self
                .maintenance_commit(
                    claim,
                    None,
                    None,
                    Completion::Complete {
                        evidence: json!({"repair":1,"authoritative":false}),
                    },
                )
                .await;
        };
        if selection.artwork.posters.skip_existing
            && repo
                .poster_is_ready(run.source_id, candidate.source_item_id)
                .await?
        {
            return self
                .maintenance_finish(
                    claim,
                    &candidate,
                    Disposition::SkippedReady,
                    OutcomeCode::Ready,
                    None,
                )
                .await;
        }
        match self
            .maintenance_candidate(claim, run, source, &candidate)
            .await
        {
            Ok(()) => Ok(()),
            Err(Error::Stale) => Err(Error::Stale),
            Err(Error::Database(error)) => Err(Error::Database(error)),
            Err(Error::Unavailable) if self.maintenance_remaining(claim).await? > 0 => {
                self.ingest
                    .fail(
                        claim,
                        screenroom_backend::queue::FailureClass::Transient,
                        "maintenance_storage_retry",
                        None,
                    )
                    .await
            }
            Err(_) => {
                self.maintenance_finish(
                    claim,
                    &candidate,
                    Disposition::Failed,
                    OutcomeCode::Unavailable,
                    None,
                )
                .await
            }
        }
    }

    async fn maintenance_candidate(
        &self,
        claim: &Claim,
        run: &Run,
        source: &str,
        candidate: &Candidate,
    ) -> Result<()> {
        let Some(adapter) = self.provider(claim, source).await? else {
            return Ok(());
        };
        let item = match self
            .maintenance_fetch(claim, adapter.item_by_external_id(&candidate.external_id))
            .await?
        {
            Fetch::Value(item) => item,
            Fetch::Deferred => return Ok(()),
            Fetch::Failed(code) => return self.maintenance_failure(claim, candidate, code).await,
        };
        if item.external_id != candidate.external_id
            || item.id != candidate.public_id
            || crate::mapping::kind(item.kind) != candidate.kind
            || !matches!(candidate.kind.as_str(), "movie" | "series")
        {
            return self
                .maintenance_failure(claim, candidate, OutcomeCode::Invalid)
                .await;
        }
        let Some(art) = item.artwork.iter().find(|art| art.kind == "Primary") else {
            return self
                .maintenance_failure(claim, candidate, OutcomeCode::NoPoster)
                .await;
        };
        if art.item_id != candidate.external_id || !live::valid_artwork(art) {
            return self
                .maintenance_failure(claim, candidate, OutcomeCode::Invalid)
                .await;
        }
        let key = live::artwork_key(art);
        let provider_input = json!({"item_id":art.item_id,"kind":art.kind,"tag":art.tag});
        let (ready, mut outcomes) = self
            .full_art_ready(claim, run.source_id, &key, &provider_input)
            .await?;
        let repair = !outcomes.is_empty();
        for outcome in outcomes.drain(..) {
            self.ingest.record_work_outcome(claim, &outcome).await?;
        }
        let input = if ready {
            let (rights, manifest): (Value, Value) = sqlx::query_as("SELECT g.rights,g.manifest FROM media.artwork a JOIN media.artwork_generations g ON g.artwork_id=a.id AND g.revision=a.current_generation WHERE a.source_id=$1 AND a.source_artwork_id=$2 AND g.state='published' AND g.rights->'provider_input'=$3")
                .bind(run.source_id).bind(&key).bind(&provider_input).fetch_one(self.catalog.pool()).await?;
            ArtworkInputWrite {
                reference: ArtworkReferenceInput {
                    source_artwork_id: key,
                    kind: "poster".into(),
                    rights,
                    item_external_id: Some(candidate.external_id.clone()),
                    person_external_id: None,
                    role: "poster".into(),
                    ordinal: 0,
                },
                manifest: serde_json::from_value(manifest)
                    .map_err(|_| Error::Invalid("published artwork manifest"))?,
            }
        } else {
            let bytes = match self
                .maintenance_fetch(claim, adapter.artwork_bytes(art))
                .await?
            {
                Fetch::Value(bytes) => bytes,
                Fetch::Deferred => return Ok(()),
                Fetch::Failed(code) => {
                    return self.maintenance_failure(claim, candidate, code).await;
                }
            };
            let image = match ValidatedImage::raster(bytes) {
                Ok(image) => image,
                Err(_) => {
                    return self
                        .maintenance_failure(claim, candidate, OutcomeCode::Invalid)
                        .await;
                }
            };
            if repair {
                if self.maintenance_remaining(claim).await? < 3 {
                    return self
                        .maintenance_failure(claim, candidate, OutcomeCode::Unavailable)
                        .await;
                }
                self.full_verified_upload(claim, run.source_id, &image, &mut outcomes)
                    .await?;
            }
            live::artwork_input(art, &image)
        };
        self.maintenance_finish(
            claim,
            candidate,
            Disposition::Staged,
            OutcomeCode::Ready,
            Some(input),
        )
        .await
    }

    async fn maintenance_fetch<T>(
        &self,
        claim: &Claim,
        future: impl std::future::Future<Output = jellyfin::Result<T>>,
    ) -> Result<Fetch<T>> {
        let receipt = self
            .ingest
            .reserve_work_request(claim, OperationClass::Artwork)
            .await?;
        let result = tokio::time::timeout(Duration::from_secs(10), future)
            .await
            .unwrap_or(Err(jellyfin::Error::Deadline));
        let error = match result {
            Ok(value) => {
                self.ingest
                    .record_work_outcome(
                        claim,
                        &RequestOutcomeWrite {
                            receipt_id: receipt.id,
                            outcome: RequestOutcome::Success,
                            error_class: None,
                        },
                    )
                    .await?;
                return Ok(Fetch::Value(value));
            }
            Err(error) => error,
        };
        let (class, code, retry) = match error {
            jellyfin::Error::RateLimited { .. } => {
                (RequestErrorClass::Throttled, OutcomeCode::Unavailable, true)
            }
            jellyfin::Error::Transport => {
                (RequestErrorClass::Transport, OutcomeCode::Transport, true)
            }
            jellyfin::Error::Deadline => (RequestErrorClass::Timeout, OutcomeCode::Timeout, true),
            jellyfin::Error::Http(500..=599) => (
                RequestErrorClass::Unavailable,
                OutcomeCode::Unavailable,
                true,
            ),
            jellyfin::Error::Unauthorized => (
                RequestErrorClass::Unauthorized,
                OutcomeCode::Unauthorized,
                false,
            ),
            jellyfin::Error::NotFound => (RequestErrorClass::Invalid, OutcomeCode::NoPoster, false),
            _ => (RequestErrorClass::Invalid, OutcomeCode::Invalid, false),
        };
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
            return Ok(Fetch::Deferred);
        }
        self.ingest.record_work_outcome(claim, &outcome).await?;
        if retry && self.maintenance_remaining(claim).await? > 0 {
            self.ingest
                .fail(
                    claim,
                    screenroom_backend::queue::FailureClass::Transient,
                    "maintenance_request_retry",
                    None,
                )
                .await?;
            return Ok(Fetch::Deferred);
        }
        Ok(Fetch::Failed(code))
    }

    async fn maintenance_remaining(&self, claim: &Claim) -> Result<i64> {
        Ok(sqlx::query_scalar("SELECT (r.selection_snapshot->'work_budget'->>'max_requests')::bigint-j.window_requests FROM queue.jobs j JOIN ingest.runs r ON r.id=j.scope_id WHERE j.id=$1")
            .bind(claim.id).fetch_one(self.catalog.pool()).await?)
    }

    async fn maintenance_failure(
        &self,
        claim: &Claim,
        candidate: &Candidate,
        code: OutcomeCode,
    ) -> Result<()> {
        let disposition = if matches!(code, OutcomeCode::NoPoster) {
            Disposition::Unavailable
        } else {
            Disposition::Failed
        };
        self.maintenance_finish(claim, candidate, disposition, code, None)
            .await
    }

    async fn maintenance_finish(
        &self,
        claim: &Claim,
        candidate: &Candidate,
        disposition: Disposition,
        code: OutcomeCode,
        input: Option<ArtworkInputWrite>,
    ) -> Result<()> {
        let write = CandidateWrite {
            source_item_id: candidate.source_item_id,
            disposition,
            code: Some(code),
            source_artwork_id: input
                .as_ref()
                .map(|input| input.reference.source_artwork_id.clone()),
        };
        self.maintenance_commit(claim, Some(write), input, Completion::Continue)
            .await
    }

    async fn maintenance_commit(
        &self,
        claim: &Claim,
        candidate: Option<CandidateWrite>,
        input: Option<ArtworkInputWrite>,
        completion: Completion,
    ) -> Result<()> {
        self.ingest
            .commit_repair_page(
                claim,
                &PageCommit {
                    libraries: vec![],
                    items: vec![],
                    artwork_inputs: input.into_iter().collect(),
                    artwork_omissions: vec![],
                    followups: vec![],
                    checkpoint: CheckpointWrite {
                        scope_key: "poster-repair-v1".into(),
                        cursor: json!({}),
                        watermark: None,
                    },
                    requests: 0,
                    completion,
                },
                &RepairWrites {
                    candidates: candidate.into_iter().collect(),
                    outcomes: vec![],
                },
            )
            .await
    }
}
