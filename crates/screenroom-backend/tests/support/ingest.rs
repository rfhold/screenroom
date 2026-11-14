#![allow(dead_code)]
use screenroom_backend::{
    Error, Result, Uuid,
    indexing::{Configuration, ConfigureRequest, Existing, IndexingRepository, StartRequest},
    ingest::{
        Claim, OperationClass, PageCommit, Policy, RequestOutcome, RequestOutcomeWrite, Run,
        TraversalWrites,
    },
    sqlx::{self, PgPool},
};
use serde_json::{Value, json};

#[derive(Debug, Clone)]
pub struct PolicyWrite {
    pub enabled: bool,
    pub selection_json: Value,
    pub max_items: i64,
    pub max_requests: i64,
    pub incremental_seconds: i32,
    pub reconcile_seconds: i32,
    pub enrich_metadata: bool,
    pub enrich_people: bool,
    pub enrich_artwork: bool,
}

pub struct IngestRepository {
    inner: screenroom_backend::ingest::IngestRepository,
    indexing: IndexingRepository,
    pool: PgPool,
}
impl IngestRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            inner: screenroom_backend::ingest::IngestRepository::new(pool.clone()),
            indexing: IndexingRepository::new(pool.clone()),
            pool,
        }
    }
    async fn configuration(&self, source: Uuid, p: &PolicyWrite) -> Result<Configuration> {
        let libraries = match p.selection_json.get("library_ids") {
            Some(value) => value.clone(),
            None => json!(sqlx::query_scalar::<_, String>("SELECT public_id FROM catalog.libraries WHERE source_id=$1 AND state='enabled' ORDER BY public_id").bind(source).fetch_all(&self.pool).await?),
        };
        Configuration::from_value(&json!({
            "indexing":1,"enabled":p.enabled,"automatic_runs":false,
            "cadence_seconds":p.incremental_seconds,"reconcile_seconds":p.reconcile_seconds,
            "mode":"full","components":{"metadata":true,"artwork":p.enrich_artwork,"embeddings":true},
            "strategy":"all","scope":{"library_ids":libraries,"item_kinds":["movie","series"]},
            "operator_profile_id":"fixture",
            "limits":{"work_requests":p.selection_json["work_budget"]["max_requests"].as_i64().unwrap_or(1000),
                "work_records":p.selection_json["work_budget"]["max_records"].as_u64().unwrap_or(1000),
                "lifetime_requests":p.max_requests,"lifetime_records":p.max_items,"deadline_seconds":86400,"work_timeout_seconds":240}
        }))
    }
    pub async fn create_policy(&self, source: Uuid, p: &PolicyWrite) -> Result<Policy> {
        // Isolated catalog fixtures use synthetic locators, never provider credentials.
        sqlx::query("UPDATE catalog.sources SET configuration=configuration || '{\"adapter\":\"jellyfin\"}' WHERE id=$1")
            .bind(source).execute(&self.pool).await?;
        self.indexing
            .enroll_locator(source, &"0".repeat(64))
            .await?;
        self.indexing
            .configure(
                source,
                &ConfigureRequest {
                    request_id: Uuid::new_v4(),
                    existing: None,
                    configuration: self.configuration(source, p).await?,
                },
            )
            .await
    }
    pub async fn update_policy(
        &self,
        id: Uuid,
        p: &PolicyWrite,
        start: bool,
    ) -> Result<(Policy, Option<Run>)> {
        let old = self.inner.policy(id).await?.ok_or(Error::NotFound)?;
        let policy = self
            .indexing
            .configure(
                old.source_id,
                &ConfigureRequest {
                    request_id: Uuid::new_v4(),
                    existing: Some(Existing {
                        policy_id: id,
                        expected_version: old.version,
                    }),
                    configuration: self.configuration(old.source_id, p).await?,
                },
            )
            .await?;
        let run = if start {
            Some(self.start_run(id).await?)
        } else {
            None
        };
        Ok((policy, run))
    }
    pub async fn start_run(&self, id: Uuid) -> Result<Run> {
        let policy = self.inner.policy(id).await?.ok_or(Error::NotFound)?;
        self.indexing
            .start(&StartRequest {
                request_id: Uuid::new_v4(),
                policy_id: id,
                policy_version: policy.version,
                scope: None,
            })
            .await
    }
    pub async fn start_run_for_model(&self, id: Uuid, model: Uuid) -> Result<Run> {
        let run = self.start_run(id).await?;
        if run.embedding_model_id != Some(model) {
            return Err(Error::Stale);
        }
        Ok(run)
    }
    pub async fn claim(&self, owner: &str, seconds: i32) -> Result<Option<Claim>> {
        let capabilities: Vec<String> =
            sqlx::query_scalar("SELECT DISTINCT unnest(requirements) FROM queue.jobs")
                .fetch_all(&self.pool)
                .await?;
        if let Some(claim) = self
            .inner
            .claim_canonical(owner, seconds, &capabilities)
            .await?
        {
            return Ok(Some(claim));
        }
        // Storage fault fixtures deliberately exercise simultaneous effects below the scheduler.
        // Admissions are still canonical; historical runs never enter this transport fixture.
        let mut tx = self.pool.begin().await?;
        let scopes: Vec<Uuid> = sqlx::query_scalar("SELECT r.id FROM ingest.runs r JOIN ingest.indexing_runs ir ON ir.run_id=r.id JOIN ingest.policies p ON p.id=r.policy_id JOIN catalog.sources s ON s.id=r.source_id WHERE r.state IN ('running','complete') AND p.enabled AND p.version=r.policy_version AND s.state='enabled' AND s.configuration=ir.source_configuration AND ingest.indexing_deadline_valid(r.id) AND NOT EXISTS(SELECT FROM ingest.source_cooldowns sc WHERE sc.source_id=s.id AND sc.until_at>clock_timestamp())")
            .fetch_all(&mut *tx).await?;
        let claim = screenroom_backend::queue::claim(
            &mut tx,
            "indexing",
            &scopes,
            &capabilities,
            owner,
            seconds,
        )
        .await?;
        tx.commit().await?;
        Ok(claim.map(Into::into))
    }
    pub async fn claim_capable(
        &self,
        owner: &str,
        seconds: i32,
        capabilities: &[String],
    ) -> Result<Option<Claim>> {
        self.inner
            .claim_canonical(owner, seconds, capabilities)
            .await
    }
    pub async fn retry(&self, c: &Claim, code: &str, delay: i32) -> Result<()> {
        self.inner
            .fail(
                c,
                screenroom_backend::queue::FailureClass::Transient,
                code,
                Some(delay),
            )
            .await
    }
    pub async fn commit_fixture_page(&self, c: &Claim, page: &PageCommit) -> Result<()> {
        let receipt = self
            .inner
            .reserve_work_request(c, OperationClass::Page)
            .await?;
        self.inner
            .commit_traversal_page(
                c,
                page,
                &TraversalWrites {
                    primary_external_ids: page
                        .items
                        .iter()
                        .map(|item| item.external_id.clone())
                        .collect(),
                    outcomes: vec![RequestOutcomeWrite {
                        receipt_id: receipt.id,
                        outcome: RequestOutcome::Success,
                        error_class: None,
                    }],
                    ..Default::default()
                },
            )
            .await
    }
}
impl std::ops::Deref for IngestRepository {
    type Target = screenroom_backend::ingest::IngestRepository;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
