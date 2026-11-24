use screenroom_backend::{
    Error, Result, Uuid,
    catalog::CatalogRepository,
    chrono::{DateTime, Utc},
    embedding::EmbeddingClient,
    ingest::{Claim, IngestRepository},
    media::MediaRepository,
    search::{Model, SearchRepository},
    sqlx::PgPool,
    storage::ObjectStorage,
};
use screenroom_observability::{
    metrics::{
        ErrorClass, JobKind as MetricKind, Outcome, QueueMetrics, Transition, WorkerMetrics,
    },
    transport::TransportObservation,
};
use std::time::Duration;

pub(crate) fn metric_kind(kind: &str) -> MetricKind {
    match kind {
        "ingest" => MetricKind::Ingest,
        "embedding" => MetricKind::Embedding,
        "artwork" => MetricKind::Artwork,
        _ => MetricKind::Other,
    }
}
pub(crate) struct JobTransport(TransportObservation);
impl JobTransport {
    fn new(kind: &str, observer: impl FnOnce(MetricKind) -> TransportObservation) -> Self {
        Self(observer(metric_kind(kind)))
    }
    pub(crate) fn provider(
        &self,
        client: screenroom_catalog_adapter::jellyfin::LiveJellyfin,
    ) -> screenroom_catalog_adapter::jellyfin::LiveJellyfin {
        client.with_transport_observer(self.0.clone())
    }
    pub(crate) fn embedding(&self, client: &EmbeddingClient) -> EmbeddingClient {
        client.clone().with_transport_observer(self.0.clone())
    }
    pub(crate) fn storage(&self, client: &ObjectStorage) -> ObjectStorage {
        client.clone().with_transport_observer(self.0.clone())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Idle,
    Committed,
    Retried,
    Deferred,
    Failed,
    Stale,
}

pub struct Worker {
    pub(crate) pool: PgPool,
    pub(crate) ingest: IngestRepository,
    pub(crate) catalog: CatalogRepository,
    pub(crate) search: SearchRepository,
    pub(crate) media: MediaRepository,
    pub(crate) embedding: Option<EmbeddingClient>,
    pub(crate) storage: ObjectStorage,
    pub(crate) model: Option<Model>,
    pub(crate) connections: screenroom_backend::connections::ConnectionRepository,
    owner: String,
    due_cursor: tokio::sync::Mutex<Option<(DateTime<Utc>, Uuid)>>,
    pub(crate) metrics: Option<(QueueMetrics, WorkerMetrics)>,
}
impl Worker {
    pub(crate) fn transport(&self, claim: &Claim) -> JobTransport {
        JobTransport::new(&claim.kind, |kind| {
            self.metrics
                .as_ref()
                .map_or_else(TransportObservation::default, |(_, m)| {
                    m.transport_observer(kind)
                })
        })
    }
    pub async fn new(
        pool: PgPool,
        embedding: Option<EmbeddingClient>,
        storage: ObjectStorage,
        owner: String,
    ) -> Result<Self> {
        if owner.is_empty()
            || owner.len() > 200
            || embedding
                .as_ref()
                .is_some_and(|e| e.config().timeout > Duration::from_secs(30))
        {
            return Err(Error::Invalid("worker configuration"));
        }
        let search = SearchRepository::new(pool.clone());
        let model = if let Some(client) = &embedding {
            let c = client.config();
            Some(search.register_model(&c.model, &c.revision, 1024).await?)
        } else {
            None
        };
        Ok(Self {
            pool: pool.clone(),
            ingest: IngestRepository::new(pool.clone()),
            catalog: CatalogRepository::new(pool.clone()),
            media: MediaRepository::new(pool.clone()),
            search,
            embedding,
            storage,
            model,
            connections: screenroom_backend::connections::ConnectionRepository::from_env(
                pool.clone(),
            )?,
            owner,
            due_cursor: tokio::sync::Mutex::new(None),
            metrics: None,
        })
    }
    /// Returns admissions and whether this bounded call finished the scheduling sweep.
    pub async fn start_due(&self) -> Result<(usize, bool)> {
        let mut started = 0;
        // Continue past blocked pages without loading unbounded policy sets.
        let mut cursor = self.due_cursor.lock().await;
        let policies = self.ingest.due_policies_after(100, *cursor).await?;
        let page_complete = policies.len() < 100;
        for policy in policies {
            *cursor = Some((policy.next_due_at, policy.id));
            if policy.selection_json.get("indexing").is_none() {
                continue;
            }
            let configuration = match screenroom_backend::indexing::Configuration::from_value(
                &policy.selection_json,
            ) {
                Ok(configuration) => configuration,
                Err(Error::Invalid(_)) => {
                    self.defer_invalid_policy(&policy).await?;
                    continue;
                }
                Err(error) => return Err(error),
            };
            if configuration.components.embeddings && self.model.is_none() {
                continue;
            }
            let request = screenroom_backend::indexing::StartRequest {
                request_id: Uuid::new_v4(),
                policy_id: policy.id,
                policy_version: policy.version,
                scope: None,
            };
            match screenroom_backend::indexing::IndexingRepository::new(self.pool.clone())
                .start_automatic(&request)
                .await
            {
                Ok(_) => started += 1,
                Err(Error::SourceBusy { .. } | Error::Stale) => {}
                Err(Error::Invalid(_)) => self.defer_invalid_policy(&policy).await?,
                Err(e) => return Err(e),
            }
        }
        if page_complete {
            *cursor = None;
        }
        Ok((started, page_complete))
    }
    async fn defer_invalid_policy(
        &self,
        policy: &screenroom_backend::ingest::Policy,
    ) -> Result<()> {
        self.ingest.defer_invalid_automatic_policy(policy).await?;
        tracing::warn!(
            code = "automatic_policy_invalid",
            "automatic index start deferred"
        );
        Ok(())
    }
    pub async fn activate_model(&self) -> Result<()> {
        self.search
            .activate_model(
                self.model
                    .as_ref()
                    .ok_or(Error::Invalid("embedding not configured"))?
                    .id,
            )
            .await
    }
    pub async fn step(&self) -> Result<Step> {
        let Some(claim) = self
            .ingest
            .claim_canonical(&self.owner, 300, &self.capabilities().await?)
            .await?
        else {
            return Ok(Step::Idle);
        };
        let kind = metric_kind(&claim.kind);
        if let Some((q, _)) = &self.metrics {
            q.transition(kind, Transition::Claimed, 1);
        }
        let started = std::time::Instant::now();
        if let Err(e) = self.ingest.renew(&claim, 300).await {
            if matches!(e, Error::Invalid("indexing deadline expired")) {
                self.ingest.expire_deadlines().await?;
                return Ok(Step::Failed);
            }
            return if matches!(e, Error::Stale) {
                Ok(Step::Stale)
            } else {
                Err(e)
            };
        }
        let run = self
            .ingest
            .run(claim.run_id.ok_or(Error::Stale)?)
            .await?
            .ok_or(Error::Stale)?;
        let config = screenroom_backend::indexing::configuration(&run)?
            .ok_or(Error::Invalid("canonical indexing run required"))?;
        let result = tokio::time::timeout(
            Duration::from_secs(config.limits.work_timeout_seconds as u64),
            self.process(&claim, &run),
        )
        .await
        .unwrap_or(Err(Error::Unavailable));
        let step = match result {
            Ok(()) => self.settled_step(&claim, false).await,
            Err(Error::Invalid("indexing deadline expired")) => {
                self.ingest.expire_deadlines().await?;
                Ok(Step::Failed)
            }
            Err(Error::Stale) => Ok(Step::Stale),
            Err(e) => {
                if let Some((_, m)) = &self.metrics {
                    m.error(
                        kind,
                        match &e {
                            Error::Database(_) => ErrorClass::Database,
                            Error::Invalid(_) | Error::Incompatible(_) | Error::NotFound => {
                                ErrorClass::InvalidInput
                            }
                            _ => ErrorClass::Other,
                        },
                    );
                }
                use screenroom_backend::queue::FailureClass::{Permanent, Transient};
                let (class, code) = match e {
                    Error::Unavailable => (Transient, "upstream_unavailable"),
                    Error::Incompatible(_) => (Permanent, "incompatible_input"),
                    Error::Invalid(_) | Error::NotFound => (Permanent, "invalid_input"),
                    Error::Database(_) => (Transient, "database_unavailable"),
                    Error::SourceBusy { .. } => (Transient, "source_busy"),
                    Error::Connection("credentials_rejected" | "server_identity_mismatch") => {
                        (Permanent, "connection_identity")
                    }
                    Error::Connection(_) => (Transient, "connection_unavailable"),
                    Error::Stale => unreachable!(),
                };
                match self.ingest.fail(&claim, class, code, None).await {
                    Ok(()) => self.settled_step(&claim, true).await,
                    Err(Error::Stale) => Ok(Step::Stale),
                    Err(e) => Err(e),
                }
            }
        }?;
        if let Some((q, m)) = &self.metrics {
            let (transition, outcome) = match step {
                Step::Committed => (Transition::Committed, Outcome::Committed),
                Step::Deferred => (Transition::Deferred, Outcome::Deferred),
                Step::Retried => (Transition::Retried, Outcome::Retried),
                Step::Failed => (Transition::Failed, Outcome::Failed),
                Step::Stale => (Transition::Stale, Outcome::Stale),
                Step::Idle => unreachable!(),
            };
            if step != Step::Stale {
                q.transition(kind, transition, 1);
            }
            m.processing(kind, outcome, started.elapsed());
        }
        Ok(step)
    }
    async fn settled_step(&self, claim: &Claim, retried: bool) -> Result<Step> {
        let outcome: String = screenroom_backend::sqlx::query_scalar(
            "SELECT outcome FROM queue.attempts WHERE job_id=$1 AND fence=$2",
        )
        .bind(claim.id)
        .bind(claim.fence)
        .fetch_one(&self.pool)
        .await?;
        Ok(match outcome.as_str() {
            "permanent" | "exhausted" => Step::Failed,
            "retryable" | "throttled" => {
                if retried {
                    Step::Retried
                } else {
                    Step::Deferred
                }
            }
            "complete" | "released" => Step::Committed,
            _ => Step::Stale,
        })
    }
    pub fn with_metrics(mut self, queue: QueueMetrics, worker: WorkerMetrics) -> Self {
        self.metrics = Some((queue, worker));
        self
    }
    pub async fn capabilities(&self) -> Result<Vec<String>> {
        let mut capabilities = self.connections.capabilities().await?;
        if let Some(model) = &self.model {
            capabilities.push(format!("model:{}", model.id));
        }
        Ok(capabilities)
    }
    async fn process(&self, claim: &Claim, run: &screenroom_backend::ingest::Run) -> Result<()> {
        let canonical: bool = screenroom_backend::sqlx::query_scalar(
            "SELECT EXISTS(SELECT FROM ingest.indexing_runs WHERE run_id=$1)",
        )
        .bind(run.id)
        .fetch_one(&self.pool)
        .await?;
        if !canonical {
            return Err(Error::Invalid("canonical indexing run required"));
        }
        let source = self
            .catalog
            .sources()
            .await?
            .into_iter()
            .find(|s| s.id == run.source_id)
            .ok_or(Error::Stale)?;
        if source
            .configuration
            .get("adapter")
            .and_then(serde_json::Value::as_str)
            != Some("jellyfin")
        {
            return Err(Error::Invalid("Jellyfin source required"));
        }
        if run.selection_snapshot.get("repair").is_some() {
            return self.repair_process(claim, run, &source.source_key).await;
        }
        self.full_process(claim, run, &source.source_key).await
    }
}

#[cfg(test)]
#[path = "../tests/transport_metrics/worker.rs"]
mod transport_metrics;
