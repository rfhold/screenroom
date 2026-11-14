use screenroom_backend::{
    Error, Uuid,
    db::MIGRATOR,
    ingest::OperationClass,
    queue::{self, FailureClass},
    search::SearchRepository,
    sqlx,
};
use serde_json::json;
use sqlx::{PgPool, migrate::Migrator, postgres::PgPoolOptions};
use std::borrow::Cow;
mod support;
use support::{CatalogRepository, IngestRepository, PolicyWrite};

fn source_capabilities(source: &str) -> Vec<String> {
    vec![
        "adapter:jellyfin".into(),
        format!("source:{source}"),
        format!("locator:{}", "0".repeat(64)),
    ]
}

async fn database() -> (PgPool, PgPool, String) {
    let url = std::env::var("SCREENROOM_TEST_DATABASE_URL").unwrap();
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .unwrap();
    let name = format!("queue_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    let options = admin.connect_options().as_ref().clone().database(&name);
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect_with(options)
        .await
        .unwrap();
    (admin, pool, name)
}
async fn cleanup(admin: PgPool, pool: PgPool, name: String) {
    pool.close().await;
    sqlx::query(&format!("DROP DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}
async fn claim(pool: &PgPool, scope: Uuid, caps: &[String]) -> Option<queue::Claim> {
    let mut tx = pool.begin().await.unwrap();
    let c = queue::claim(&mut tx, "test", &[scope], caps, "test-worker", 60)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    c
}
#[tokio::test]
#[ignore = "requires explicitly configured disposable PostgreSQL18 + pgvector admin URL"]
async fn postgres_reusable_queue_faults_and_lineage() {
    let (admin, pool, name) = database().await;
    MIGRATOR.run(&pool).await.unwrap();
    let scope = Uuid::new_v4();
    let other = Uuid::new_v4();
    let input = json!({"opaque":"payload"});
    let requirements = vec!["model:revision-a".into(), "provider:one".into()];
    let mut tx = pool.begin().await.unwrap();
    let j = queue::Enqueue {
        queue: "test",
        scope_id: scope,
        kind: "custom",
        dedupe_key: "shared",
        input: &input,
        requirements: &requirements,
    };
    let inserted = queue::enqueue(&mut tx, &j).await.unwrap();
    let id = inserted.id();
    assert_eq!(inserted, queue::EnqueueResult::Inserted(id));
    assert_eq!(
        queue::enqueue(&mut tx, &j).await.unwrap(),
        queue::EnqueueResult::Existing(id)
    );
    let changed = json!({"different":true});
    assert!(matches!(
        queue::enqueue(
            &mut tx,
            &queue::Enqueue {
                input: &changed,
                ..j
            }
        )
        .await,
        Err(queue::Error::Conflict)
    ));
    let other_id = queue::enqueue(
        &mut tx,
        &queue::Enqueue {
            scope_id: other,
            ..j
        },
    )
    .await
    .unwrap()
    .id();
    assert_ne!(other_id, id);
    tx.commit().await.unwrap();
    assert!(
        claim(
            &pool,
            scope,
            &["model:revision-b".into(), "provider:one".into()]
        )
        .await
        .is_none()
    );
    assert!(
        claim(&pool, scope, &["model:revision-a".into()])
            .await
            .is_none()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i32>("SELECT attempts FROM queue.jobs WHERE id=$1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    let first = claim(&pool, scope, &requirements).await.unwrap();
    assert!(claim(&pool, scope, &requirements).await.is_none());
    let mut tx = pool.begin().await.unwrap();
    queue::renew(&mut tx, &first, 60).await.unwrap();
    queue::fail(
        &mut tx,
        &first,
        FailureClass::Permanent,
        "invalid_payload",
        None,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert!(claim(&pool, scope, &requirements).await.is_none());
    let request = Uuid::new_v4();
    sqlx::query("UPDATE queue.jobs SET window_requests=17 WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    assert!(matches!(
        queue::ack(&mut tx, &first).await,
        Err(queue::Error::Stale)
    ));
    let redrive = queue::redrive(&mut tx, id, request).await.unwrap();
    assert_eq!(redrive.generation, 1);
    assert_eq!(
        queue::redrive(&mut tx, id, request).await.unwrap().job_id,
        id
    );
    assert!(matches!(
        queue::redrive(&mut tx, other_id, request).await,
        Err(queue::Error::Conflict)
    ));
    tx.commit().await.unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT window_requests FROM queue.jobs WHERE id=$1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        17
    );
    let second = claim(&pool, scope, &requirements).await.unwrap();
    sqlx::query("UPDATE queue.jobs SET lease_until=now()-interval '1 second' WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let third = claim(&pool, scope, &requirements).await.unwrap();
    assert!(third.fence > second.fence);
    let mut tx = pool.begin().await.unwrap();
    assert!(matches!(
        queue::renew(&mut tx, &second, 60).await,
        Err(queue::Error::Stale)
    ));
    assert!(matches!(
        queue::ack(&mut tx, &second).await,
        Err(queue::Error::Stale)
    ));
    queue::fail(
        &mut tx,
        &third,
        FailureClass::Throttled,
        "throttled",
        Some(600),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert!(claim(&pool, scope, &requirements).await.is_none());
    let delay: f64 = sqlx::query_scalar(
        "SELECT extract(epoch FROM available_at-now())::float8 FROM queue.jobs WHERE id=$1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(delay > 590.0);
    sqlx::query("UPDATE queue.jobs SET available_at=now() WHERE id=$1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let fourth = claim(&pool, scope, &requirements).await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    queue::release(&mut tx, &fourth, true).await.unwrap();
    tx.commit().await.unwrap();
    let fifth = claim(&pool, scope, &requirements).await.unwrap();
    assert_eq!(fifth.attempts, 1);
    let mut tx = pool.begin().await.unwrap();
    queue::ack(&mut tx, &fifth).await.unwrap();
    let history = queue::attempts(&mut tx, id, 0, 200).await.unwrap();
    assert_eq!(history.len(), 5);
    assert_eq!(history[0].outcome, "permanent");
    assert_eq!(history[1].outcome, "lease_expired");
    assert_eq!(history[1].generation, 1);
    assert_eq!(history[2].outcome, "throttled");
    assert_eq!(history[4].outcome, "complete");
    assert_eq!(queue::redrives(&mut tx, id, 0, 200).await.unwrap().len(), 1);
    tx.commit().await.unwrap();
    let orphan = claim(&pool, other, &requirements).await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    queue::cancel_scope(&mut tx, "test", other, true)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let resumed = claim(&pool, other, &requirements).await.unwrap();
    assert!(resumed.fence > orphan.fence);
    assert_eq!(resumed.attempts, 2);
    sqlx::query("UPDATE queue.jobs SET max_attempts=attempts,lease_until=now()-interval '1 second' WHERE id=$1").bind(other_id).execute(&pool).await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    queue::retire_exhausted(&mut tx, "test", other)
        .await
        .unwrap();
    assert_eq!(
        queue::attempts(&mut tx, other_id, 0, 200)
            .await
            .unwrap()
            .last()
            .unwrap()
            .outcome,
        "exhausted"
    );
    tx.commit().await.unwrap();
    cleanup(admin, pool, name).await;
}

#[tokio::test]
#[ignore = "requires explicitly configured disposable PostgreSQL18 + pgvector admin URL"]
async fn postgres_domain_redrive_preserves_budget_and_rejects_superseded_runs() {
    let (admin, pool, name) = database().await;
    MIGRATOR.run(&pool).await.unwrap();
    SearchRepository::new(pool.clone())
        .register_model("pinned", "1", 1024)
        .await
        .unwrap();
    let catalog = CatalogRepository::new(pool.clone());
    let ingest = IngestRepository::new(pool.clone());
    let source = catalog
        .configure_source(
            "queue-domain",
            "jellyfin",
            &json!({"adapter":"mock"}),
            &json!({}),
        )
        .await
        .unwrap();
    catalog
        .configure_library(source.id, "movies", "movies", "Movies")
        .await
        .unwrap();
    let p=ingest.create_policy(source.id,&PolicyWrite {enabled:true,selection_json:json!({"library_ids":["movies"],"work_budget":{"mode":"per_work_commit","max_requests":10,"max_records":10}}),max_items:100,max_requests:10,incremental_seconds:60,reconcile_seconds:3600,enrich_metadata:true,enrich_people:false,enrich_artwork:false}).await.unwrap();
    let run = ingest.start_run(p.id).await.unwrap();
    let capabilities = source_capabilities(&source.source_key);
    assert_eq!(
        ingest
            .queue_snapshot(Some(run.id), &[])
            .await
            .unwrap()
            .blocked,
        1
    );
    assert_eq!(
        ingest
            .queue_snapshot(Some(run.id), &capabilities)
            .await
            .unwrap()
            .claimable,
        1
    );
    let c = ingest
        .claim_capable("domain", 60, &capabilities)
        .await
        .unwrap()
        .unwrap();
    ingest
        .reserve_work_request(&c, OperationClass::Page)
        .await
        .unwrap();
    ingest
        .fail(&c, FailureClass::Permanent, "invalid_payload", None)
        .await
        .unwrap();
    assert_eq!(ingest.run(run.id).await.unwrap().unwrap().state, "failed");
    assert!(matches!(ingest.resume(run.id).await, Err(Error::Stale)));
    let request = Uuid::new_v4();
    let redrive = ingest.redrive_job(c.id, request).await.unwrap();
    assert_eq!(redrive.job_id, c.id);
    assert_eq!(redrive.generation, 1);
    assert_eq!(
        ingest.redrive_job(c.id, request).await.unwrap().generation,
        1
    );
    let run_after = ingest.run(run.id).await.unwrap().unwrap();
    assert_eq!(run_after.requests, 1);
    assert_eq!(run_after.embedding_model_id, run.embedding_model_id);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT window_requests FROM queue.jobs WHERE id=$1")
            .bind(c.id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    let c2 = ingest
        .claim_capable("domain", 60, &capabilities)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ingest.job_attempts(c.id, 0, 200).await.unwrap().len(), 2);
    ingest
        .fail(&c2, FailureClass::Permanent, "invalid_payload", None)
        .await
        .unwrap();
    let newer = ingest.start_run(p.id).await.unwrap();
    assert!(matches!(
        ingest.redrive_job(c.id, Uuid::new_v4()).await,
        Err(Error::Stale)
    ));
    ingest.cancel_run(newer.id).await.unwrap();
    assert!(matches!(
        ingest.redrive_job(c.id, Uuid::new_v4()).await,
        Err(Error::Stale)
    ));
    assert_eq!(ingest.job_redrives(c.id, 0, 200).await.unwrap().len(), 1);
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT prior_state FROM ingest.run_redrives WHERE request_id=$1"
        )
        .bind(request)
        .fetch_one(&pool)
        .await
        .unwrap(),
        "failed"
    );
    cleanup(admin, pool, name).await;
}

#[tokio::test]
#[ignore = "requires explicitly configured disposable PostgreSQL18 + pgvector admin URL"]
async fn postgres_queue_upgrade_preserves_job_receipt_foreign_keys() {
    let (admin, pool, name) = database().await;
    let prior = Migrator {
        migrations: Cow::Owned(
            MIGRATOR
                .iter()
                .filter(|m| m.version < 202611120010)
                .cloned()
                .collect(),
        ),
        ..Migrator::DEFAULT
    };
    prior.run(&pool).await.unwrap();
    let source:Uuid=sqlx::query_scalar("INSERT INTO catalog.sources(source_key,provider,configuration) VALUES('upgrade','jellyfin','{\"adapter\":\"mock\"}') RETURNING id").fetch_one(&pool).await.unwrap();
    let policy:Uuid=sqlx::query_scalar("INSERT INTO ingest.policies(source_id,selection_json,max_items,max_requests,incremental_seconds,reconcile_seconds,enrich_metadata,enrich_people,enrich_artwork) VALUES($1,'{}',100,100,60,3600,true,false,false) RETURNING id").bind(source).fetch_one(&pool).await.unwrap();
    let run:Uuid=sqlx::query_scalar("INSERT INTO ingest.runs(policy_id,policy_version,policy_snapshot,selection_snapshot,source_id) VALUES($1,1,'{}','{}',$2) RETURNING id").bind(policy).bind(source).fetch_one(&pool).await.unwrap();
    // Discover the pre-cutover relation only within this historical migration fixture.
    let prior_jobs:String=sqlx::query_scalar("SELECT oid::regclass::text FROM pg_class WHERE relname='jobs' AND relnamespace=(SELECT oid FROM pg_namespace WHERE nspname='ingest')").fetch_one(&pool).await.unwrap();
    let job:Uuid=sqlx::query_scalar(&format!("INSERT INTO {prior_jobs}(run_id,kind,dedupe_key,input,state,lease_owner,lease_until,fence,attempts) VALUES($1,'ingest','upgrade','{{}}','leased','old-worker',now()+interval '1 minute',4,2) RETURNING id")).bind(run).fetch_one(&pool).await.unwrap();
    let receipt:Uuid=sqlx::query_scalar("INSERT INTO ingest.request_receipts(run_id,job_id,fence,operation,outcome,is_request) VALUES($1,$2,4,'page','unknown',true) RETURNING id").bind(run).bind(job).fetch_one(&pool).await.unwrap();
    MIGRATOR.run(&pool).await.unwrap();
    MIGRATOR.run(&pool).await.unwrap();
    let preserved:(Uuid,Uuid)=sqlx::query_as("SELECT j.id,r.id FROM queue.jobs j JOIN ingest.request_receipts r ON r.job_id=j.id WHERE r.id=$1 AND j.scope_id=$2").bind(receipt).bind(run).fetch_one(&pool).await.unwrap();
    assert_eq!(preserved, (job, receipt));
    assert!(
        sqlx::query_scalar::<_, Option<String>>("SELECT to_regclass('ingest.'||'jobs')::text")
            .fetch_one(&pool)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(sqlx::query_scalar::<_,i64>("SELECT count(*) FROM queue.attempts WHERE job_id=$1 AND fence=4 AND finished_at IS NULL").bind(job).fetch_one(&pool).await.unwrap(),1);
    cleanup(admin, pool, name).await;
}

#[tokio::test]
#[ignore = "requires explicitly configured disposable PostgreSQL18 + pgvector admin URL"]
async fn postgres_embedding_admission_requires_pin_and_enqueue_cannot_create_one() {
    use screenroom_backend::ingest::{CheckpointWrite, Completion, Followup, JobKind, PageCommit};
    let (admin, pool, name) = database().await;
    MIGRATOR.run(&pool).await.unwrap();
    let catalog = CatalogRepository::new(pool.clone());
    let ingest = IngestRepository::new(pool.clone());
    let source = catalog
        .configure_source(
            "pin-admission",
            "jellyfin",
            &json!({"adapter":"mock"}),
            &json!({}),
        )
        .await
        .unwrap();
    catalog
        .configure_library(source.id, "movies", "movies", "Movies")
        .await
        .unwrap();
    let p = ingest
        .create_policy(
            source.id,
            &PolicyWrite {
                enabled: true,
                selection_json: json!({"library_ids":["movies"]}),
                max_items: 100,
                max_requests: 10,
                incremental_seconds: 60,
                reconcile_seconds: 3600,
                enrich_metadata: true,
                enrich_people: false,
                enrich_artwork: false,
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        ingest.start_run(p.id).await,
        Err(Error::Invalid("run has no valid pinned embedding model"))
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM ingest.runs")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM queue.jobs")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    let model = SearchRepository::new(pool.clone())
        .register_model("admitted", "1", 1024)
        .await
        .unwrap();
    let admitted = ingest.start_run(p.id).await.unwrap();
    assert_eq!(admitted.embedding_model_id, Some(model.id));
    ingest.cancel_run(admitted.id).await.unwrap();
    // Historical NULL pins remain readable, but neither admission nor followups may repair them.
    let legacy:Uuid=sqlx::query_scalar("INSERT INTO ingest.runs(policy_id,policy_version,policy_snapshot,selection_snapshot,source_id) VALUES($1,$2,$3,$4,$5) RETURNING id").bind(p.id).bind(p.version).bind(serde_json::to_value(&p).unwrap()).bind(&p.selection_json).bind(source.id).fetch_one(&pool).await.unwrap();
    assert!(matches!(
        ingest.start_run(p.id).await,
        Err(Error::Invalid("run has no valid pinned embedding model"))
    ));
    let mut tx = pool.begin().await.unwrap();
    queue::enqueue(
        &mut tx,
        &queue::Enqueue {
            queue: "indexing",
            scope_id: legacy,
            kind: "ingest",
            dedupe_key: "legacy-null-pin",
            input: &json!({}),
            requirements: &["adapter:mock".into()],
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert!(
        ingest
            .claim_capable("null-pin", 60, &["adapter:mock".into()])
            .await
            .unwrap()
            .is_none()
    );
    let mut tx = pool.begin().await.unwrap();
    let claim: screenroom_backend::ingest::Claim = queue::claim(
        &mut tx,
        "indexing",
        &[legacy],
        &["adapter:mock".into()],
        "historical-fence",
        60,
    )
    .await
    .unwrap()
    .unwrap()
    .into();
    tx.commit().await.unwrap();
    let page = PageCommit {
        libraries: vec![],
        items: vec![],
        artwork_inputs: vec![],
        artwork_omissions: vec![],
        followups: vec![Followup {
            kind: JobKind::Embedding,
            dedupe_key: "incidental-pin".into(),
            input: json!({"external_id":"missing","model_id":model.id}),
        }],
        checkpoint: CheckpointWrite {
            scope_key: "null-pin".into(),
            cursor: json!({}),
            watermark: None,
        },
        requests: 0,
        completion: Completion::Continue,
    };
    assert!(
        ingest
            .commit_traversal_page(&claim, &page, &Default::default())
            .await
            .is_err()
    );
    assert_eq!(
        ingest
            .run(legacy)
            .await
            .unwrap()
            .unwrap()
            .embedding_model_id,
        None
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM queue.jobs WHERE scope_id=$1 AND kind='embedding'"
        )
        .bind(legacy)
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    assert!(
        ingest
            .run_checkpoint(&ingest.run(legacy).await.unwrap().unwrap(), "null-pin")
            .await
            .unwrap()
            .is_none()
    );
    cleanup(admin, pool, name).await;
}

#[tokio::test]
#[ignore = "requires explicitly configured disposable PostgreSQL18 + pgvector admin URL"]
async fn postgres_changed_document_retains_versioned_followups_and_fences_old_effects() {
    use screenroom_backend::{
        catalog::ItemWrite,
        embedding::{DOCUMENT_VERSION, input_hash},
        ingest::{CheckpointWrite, Completion, Followup, JobKind, PageCommit, TraversalWrites},
    };
    let (admin, pool, name) = database().await;
    MIGRATOR.run(&pool).await.unwrap();
    let search = SearchRepository::new(pool.clone());
    let model = search.register_model("versioned", "1", 1024).await.unwrap();
    let catalog = CatalogRepository::new(pool.clone());
    let ingest = IngestRepository::new(pool.clone());
    let source = catalog
        .configure_source(
            "changing-document",
            "jellyfin",
            &json!({"adapter":"mock"}),
            &json!({}),
        )
        .await
        .unwrap();
    catalog
        .configure_library(source.id, "movies", "movies", "Movies")
        .await
        .unwrap();
    let p=ingest.create_policy(source.id,&PolicyWrite {enabled:true,selection_json:json!({"library_ids":["movies"],"work_budget":{"mode":"per_work_commit","max_requests":10,"max_records":10}}),max_items:100,max_requests:10,incremental_seconds:60,reconcile_seconds:3600,enrich_metadata:true,enrich_people:false,enrich_artwork:false}).await.unwrap();
    let run = ingest.start_run(p.id).await.unwrap();
    let base_capabilities = source_capabilities(&source.source_key);
    let mut capabilities = base_capabilities.clone();
    capabilities.push(format!("model:{}", model.id));
    let page = |text: &str, offset: i32, completion| PageCommit {
        libraries: vec![],
        items: vec![ItemWrite {
            public_id: "changing".into(),
            external_id: "changing".into(),
            kind: "movie".into(),
            parent_public_id: None,
            source_revision: Some(offset.to_string()),
            metadata: json!({"title":text}),
            provenance: json!({}),
            content_hash: input_hash(text).to_vec(),
            library_external_ids: vec!["movies".into()],
            external_ids: vec![],
            credits: vec![],
            document: text.into(),
            document_title: text.into(),
            document_version: DOCUMENT_VERSION,
        }],
        artwork_inputs: vec![],
        artwork_omissions: vec![],
        followups: vec![Followup {
            kind: JobKind::Embedding,
            dedupe_key: format!("full-embed:{}:changing:{}", run.id, model.id),
            input: json!({"external_id":"changing","model_id":model.id}),
        }],
        checkpoint: CheckpointWrite {
            scope_key: "changed-document".into(),
            cursor: json!({"offset":offset}),
            watermark: None,
        },
        requests: 0,
        completion,
    };
    let first = ingest
        .claim_capable("page-h1", 60, &base_capabilities)
        .await
        .unwrap()
        .unwrap();
    ingest
        .commit_traversal_page(
            &first,
            &page("First document", 1, Completion::Continue),
            &TraversalWrites::default(),
        )
        .await
        .unwrap();
    sqlx::query("UPDATE queue.jobs SET available_at=now()+interval '1 day' WHERE scope_id=$1 AND kind='ingest'").bind(run.id).execute(&pool).await.unwrap();
    let h1 = ingest
        .claim_capable("embedding-h1", 60, &capabilities)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(h1.kind, "embedding");
    sqlx::query("UPDATE queue.jobs SET available_at=now() WHERE scope_id=$1 AND kind='ingest'")
        .bind(run.id)
        .execute(&pool)
        .await
        .unwrap();
    let second = ingest.claim("page-h2", 60).await.unwrap().unwrap();
    ingest
        .commit_traversal_page(
            &second,
            &page(
                "Changed document",
                2,
                Completion::Complete {
                    evidence: json!({"traversed":true}),
                },
            ),
            &TraversalWrites::default(),
        )
        .await
        .unwrap();
    let keys:Vec<String>=sqlx::query_scalar("SELECT dedupe_key FROM queue.jobs WHERE scope_id=$1 AND kind='embedding' ORDER BY dedupe_key").bind(run.id).fetch_all(&pool).await.unwrap();
    assert_eq!(keys.len(), 2);
    assert_ne!(keys[0], keys[1]);
    let current = search
        .document_for_reference("changing-document", "changing")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.content_hash, input_hash("Changed document"));
    let checkpoint = ingest
        .run_checkpoint(
            &ingest.run(run.id).await.unwrap().unwrap(),
            "changed-document",
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(checkpoint["offset"], 2);
    let coverage = ingest.run_coverage(run.id).await.unwrap();
    assert_eq!(coverage.followup_ready, 1);
    assert_eq!(coverage.followup_leased, 1);
    assert_eq!(coverage.followup_failed, 0);
    let mut vector = vec![0.0; 1024];
    vector[0] = 1.0;
    assert!(matches!(
        search
            .commit_embedding(
                &h1,
                current.id,
                model.id,
                &model.revision,
                &input_hash("First document"),
                &vector
            )
            .await,
        Err(Error::Stale)
    ));
    assert!(matches!(
        search
            .commit_embedding(
                &h1,
                current.id,
                model.id,
                &model.revision,
                &current.content_hash,
                &vector
            )
            .await,
        Err(Error::Stale)
    ));
    let h2 = ingest.claim("embedding-h2", 60).await.unwrap().unwrap();
    assert_ne!(h1.id, h2.id);
    search
        .commit_embedding(
            &h2,
            current.id,
            model.id,
            &model.revision,
            &current.content_hash,
            &vector,
        )
        .await
        .unwrap();
    let mut obsolete = vec![0.0; 1024];
    obsolete[1] = 1.0;
    assert!(matches!(
        search
            .commit_embedding(
                &h1,
                current.id,
                model.id,
                &model.revision,
                &current.content_hash,
                &obsolete
            )
            .await,
        Err(Error::Stale)
    ));
    let stored:(Vec<u8>,String)=sqlx::query_as("SELECT input_hash,vector::text FROM search.embeddings WHERE document_id=$1 AND model_id=$2").bind(current.id).bind(model.id).fetch_one(&pool).await.unwrap();
    assert_eq!(stored.0, current.content_hash);
    assert!(stored.1.starts_with("[1,0,"));
    ingest.complete_job(&h1).await.unwrap();
    cleanup(admin, pool, name).await;
}
