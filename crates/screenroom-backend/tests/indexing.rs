use screenroom_backend::{Error, Uuid, db::MIGRATOR, indexing::*, ingest::IngestRepository, sqlx};
use serde_json::json;

fn configuration() -> Configuration {
    serde_json::from_value(json!({"indexing":1,"enabled":true,"cadence_seconds":3600,"mode":"full","components":{"metadata":true,"artwork":false,"embeddings":false},"strategy":"all","scope":{"library_ids":["a"],"item_kinds":["movie","series"]},"operator_profile_id":"operator","limits":{"work_requests":10,"work_records":8,"lifetime_requests":20,"lifetime_records":100,"deadline_seconds":3600,"work_timeout_seconds":30}})).unwrap()
}
#[tokio::test]
#[ignore = "requires explicit disposable PostgreSQL18 + pgvector admin URL"]
async fn historical_runs_remain_readable_but_are_never_canonical_claims() {
    use screenroom_backend::queue;
    let admin = sqlx::PgPool::connect(&std::env::var("SCREENROOM_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let name = format!("historical_claim_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_with(admin.connect_options().as_ref().clone().database(&name))
        .await
        .unwrap();
    MIGRATOR.run(&pool).await.unwrap();
    let source: Uuid = sqlx::query_scalar("INSERT INTO catalog.sources(source_key,provider,configuration) VALUES('historical','jellyfin','{}') RETURNING id").fetch_one(&pool).await.unwrap();
    let policy: Uuid = sqlx::query_scalar("INSERT INTO ingest.policies(source_id,selection_json,max_items,max_requests,incremental_seconds,reconcile_seconds,enrich_metadata,enrich_people,enrich_artwork) VALUES($1,'{\"library_ids\":[\"a\"]}',100,100,60,3600,false,false,false) RETURNING id").bind(source).fetch_one(&pool).await.unwrap();
    let run: Uuid = sqlx::query_scalar("INSERT INTO ingest.runs(source_id,policy_id,policy_version,policy_snapshot,selection_snapshot) SELECT source_id,id,version,to_jsonb(p),selection_json FROM ingest.policies p WHERE id=$1 RETURNING id").bind(policy).fetch_one(&pool).await.unwrap();
    let mut tx = pool.begin().await.unwrap();
    let job = queue::enqueue(
        &mut tx,
        &queue::Enqueue {
            queue: "indexing",
            scope_id: run,
            kind: "ingest",
            dedupe_key: "historical",
            input: &json!({}),
            requirements: &[],
        },
    )
    .await
    .unwrap()
    .id();
    tx.commit().await.unwrap();
    let ingest = IngestRepository::new(pool.clone());
    assert_eq!(ingest.run(run).await.unwrap().unwrap().id, run);
    assert!(
        ingest
            .claim_canonical("canonical", 60, &[])
            .await
            .unwrap()
            .is_none()
    );
    let unchanged: (String, i32) =
        sqlx::query_as("SELECT state,attempts FROM queue.jobs WHERE id=$1")
            .bind(job)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(unchanged, ("ready".into(), 0));
    let mut tx = pool.begin().await.unwrap();
    assert!(
        queue::claim(&mut tx, "indexing", &[run], &[], "transport-proof", 60)
            .await
            .unwrap()
            .is_some()
    );
    tx.rollback().await.unwrap();
    sqlx::query("INSERT INTO ingest.checkpoints(source_id,scope_key,policy_version,cursor,observed_run_id) VALUES($1,$2,1,'{\"next\":\"historical\"}',$3)")
        .bind(source).bind(format!("policy:{policy}:history")).bind(run).execute(&pool).await.unwrap();
    let snapshot = ingest.run(run).await.unwrap().unwrap();
    assert!(
        ingest
            .run_checkpoint(&snapshot, "history")
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query("UPDATE ingest.runs SET requests=2,budget_requests=2 WHERE id=$1")
        .bind(run)
        .execute(&pool)
        .await
        .unwrap();
    for selection in [
        json!({"library_ids":["a"]}),
        json!({"library_ids":["a"],"work_budget":{"mode":"lifetime"}}),
    ] {
        let snapshot:screenroom_backend::ingest::Run=sqlx::query_as("INSERT INTO ingest.runs(source_id,policy_id,policy_version,policy_snapshot,selection_snapshot,state,finished_at,requests,budget_requests) SELECT source_id,id,version,to_jsonb(p),$2,'complete',now(),2,2 FROM ingest.policies p WHERE id=$1 RETURNING *")
            .bind(policy).bind(selection).fetch_one(&pool).await.unwrap();
        assert_eq!(
            ingest
                .run_checkpoint(&snapshot, "history")
                .await
                .unwrap()
                .unwrap()["next"],
            "historical"
        );
    }
    assert!(ingest.pause(run).await.is_err());
    let mut tx = pool.begin().await.unwrap();
    let claim: screenroom_backend::ingest::Claim = queue::claim(
        &mut tx,
        "indexing",
        &[run],
        &[],
        "historical-mutation-proof",
        60,
    )
    .await
    .unwrap()
    .unwrap()
    .into();
    tx.commit().await.unwrap();
    assert!(
        ingest
            .reserve_work_request(&claim, screenroom_backend::ingest::OperationClass::Page)
            .await
            .is_err()
    );
    let page = screenroom_backend::ingest::PageCommit {
        libraries: vec![],
        items: vec![],
        artwork_inputs: vec![],
        artwork_omissions: vec![],
        followups: vec![],
        requests: 0,
        checkpoint: screenroom_backend::ingest::CheckpointWrite {
            scope_key: "history".into(),
            cursor: json!({"replacement":true}),
            watermark: None,
        },
        completion: screenroom_backend::ingest::Completion::Complete {
            evidence: json!({}),
        },
    };
    assert!(
        ingest
            .commit_traversal_page(&claim, &page, &Default::default())
            .await
            .is_err()
    );
    let preserved = ingest.run(run).await.unwrap().unwrap();
    assert_eq!((preserved.requests, preserved.budget_requests), (2, 2));
    assert_eq!(
        ingest
            .run_checkpoint(&preserved, "history")
            .await
            .unwrap()
            .unwrap()["next"],
        "historical"
    );
    sqlx::query("UPDATE ingest.runs SET state='paused' WHERE id=$1")
        .bind(run)
        .execute(&pool)
        .await
        .unwrap();
    assert!(ingest.resume(run).await.is_err());
    historical_housekeeping_is_read_only(&pool, source, policy).await;
    pool.close().await;
    sqlx::query(&format!("DROP DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}
async fn historical_history(pool: &sqlx::PgPool) -> serde_json::Value {
    sqlx::query_scalar("SELECT jsonb_build_object('runs',(SELECT jsonb_agg(to_jsonb(r) ORDER BY id) FROM ingest.runs r),'jobs',(SELECT jsonb_agg(to_jsonb(j) ORDER BY id) FROM queue.jobs j),'attempts',(SELECT jsonb_agg(to_jsonb(a) ORDER BY job_id,fence) FROM queue.attempts a),'redrives',(SELECT jsonb_agg(to_jsonb(d) ORDER BY request_id) FROM queue.redrives d),'run_redrives',(SELECT jsonb_agg(to_jsonb(d) ORDER BY request_id) FROM ingest.run_redrives d),'receipts',(SELECT jsonb_agg(to_jsonb(x) ORDER BY id) FROM ingest.request_receipts x),'checkpoints',(SELECT jsonb_agg(to_jsonb(c) ORDER BY source_id,scope_key,policy_version) FROM ingest.checkpoints c))")
        .fetch_one(pool).await.unwrap()
}
async fn historical_housekeeping_is_read_only(pool: &sqlx::PgPool, source: Uuid, policy: Uuid) {
    use screenroom_backend::queue;
    let ingest = IngestRepository::new(pool.clone());
    let budgets = [
        (false, json!({})),
        (false, json!({"mode":"lifetime"})),
        (true, json!({})),
        (true, json!({"mode":"lifetime"})),
        (
            false,
            json!({"mode":"per_work_commit","max_requests":7,"max_records":10}),
        ),
        (
            true,
            json!({"mode":"per_work_commit","max_requests":0,"max_records":10}),
        ),
        (
            true,
            json!({"mode":"per_work_commit","max_requests":100001,"max_records":10}),
        ),
        (
            true,
            json!({"mode":"per_work_commit","max_requests":7,"max_records":1001}),
        ),
        (
            true,
            json!({"mode":"per_work_commit","max_requests":"7","max_records":10}),
        ),
        (
            true,
            json!({"mode":"per_work_commit","max_requests":7,"max_records":10,"extra":true}),
        ),
    ];
    for (admitted, budget) in budgets {
        for state in [
            "ready",
            "leased",
            "failed",
            "failed_run",
            "ready_unexhausted",
            "leased_unexhausted",
        ] {
            let source: Uuid = sqlx::query_scalar("INSERT INTO catalog.sources(source_key,provider,configuration) SELECT $2,provider,configuration FROM catalog.sources WHERE id=$1 RETURNING id")
                .bind(source).bind(format!("history-{}",Uuid::new_v4())).fetch_one(pool).await.unwrap();
            let policy: Uuid = sqlx::query_scalar("INSERT INTO ingest.policies(source_id,selection_json,max_items,max_requests,incremental_seconds,reconcile_seconds,enrich_metadata,enrich_people,enrich_artwork) SELECT $2,selection_json,max_items,max_requests,incremental_seconds,reconcile_seconds,enrich_metadata,enrich_people,enrich_artwork FROM ingest.policies WHERE id=$1 RETURNING id")
                .bind(policy).bind(source).fetch_one(pool).await.unwrap();
            let run: Uuid = sqlx::query_scalar("INSERT INTO ingest.runs(source_id,policy_id,policy_version,policy_snapshot,selection_snapshot,state,started_at,requests,budget_requests) SELECT source_id,id,version,$2,$3,$4,CASE WHEN $5 THEN now() ELSE now()-interval '1 day' END,2,2 FROM ingest.policies p WHERE id=$1 RETURNING id")
                .bind(policy).bind(json!({"selection_json":configuration()})).bind(json!({"work_budget":budget}))
                .bind(if state == "failed_run" { "failed" } else { "running" }).bind(state.ends_with("unexhausted")).fetch_one(pool).await.unwrap();
            if admitted {
                sqlx::query("INSERT INTO ingest.indexing_runs(run_id,source_configuration,mode) SELECT $1,configuration,'full' FROM catalog.sources WHERE id=$2")
                    .bind(run).bind(source).execute(pool).await.unwrap();
            }
            let mut tx = pool.begin().await.unwrap();
            let job = queue::enqueue(
                &mut tx,
                &queue::Enqueue {
                    queue: "indexing",
                    scope_id: run,
                    kind: "ingest",
                    dedupe_key: "history",
                    input: &json!({}),
                    requirements: &[],
                },
            )
            .await
            .unwrap()
            .id();
            let receipt = Uuid::new_v4();
            if !state.starts_with("ready") {
                queue::claim(&mut tx, "indexing", &[run], &[], "historical-owner", 60)
                    .await
                    .unwrap()
                    .unwrap();
            }
            if state == "failed" {
                sqlx::query("UPDATE queue.jobs SET state='failed',lease_owner=NULL,lease_until=NULL WHERE id=$1").bind(job).execute(&mut *tx).await.unwrap();
                queue::redrive(&mut tx, job, receipt).await.unwrap();
                sqlx::query("UPDATE queue.jobs SET state='failed' WHERE id=$1")
                    .bind(job)
                    .execute(&mut *tx)
                    .await
                    .unwrap();
            } else {
                sqlx::query("UPDATE queue.jobs SET attempts=CASE WHEN $2 THEN attempts ELSE max_attempts END,lease_until=CASE WHEN state='leased' THEN now()-interval '1 minute' ELSE NULL END WHERE id=$1").bind(job).bind(state.ends_with("unexhausted")).execute(&mut *tx).await.unwrap();
            }
            tx.commit().await.unwrap();
            let before = historical_history(pool).await;
            assert!(
                ingest
                    .claim_canonical("canonical", 60, &[])
                    .await
                    .unwrap()
                    .is_none()
            );
            assert_eq!(
                historical_history(pool).await,
                before,
                "housekeeping {state} {budget}"
            );
            assert!(matches!(
                ingest.redrive_job(job, Uuid::new_v4()).await,
                Err(Error::Stale)
            ));
            assert!(!ingest.redrive_eligibility(job).await.unwrap().allowed);
            if state == "failed" {
                let replay = ingest.redrive_job(job, receipt).await.unwrap();
                assert_eq!(replay.request_id, receipt);
                assert_eq!(replay.generation, 1);
            }
            let counts = ingest.queue_snapshot(Some(run), &[]).await.unwrap();
            assert_eq!((counts.claimable, counts.reclaimable), (0, 0));
            assert_eq!(counts.blocked, if state == "failed" { 0 } else { 1 });
            let blockers = ingest.queue_blockers(Some(run), Some(&[])).await.unwrap();
            assert_eq!(blockers.run_state, if state == "failed" { 0 } else { 1 });
            assert!(blockers.next_eligible_at.is_none());
            assert!(
                ingest
                    .queue_blockers(Some(run), None)
                    .await
                    .unwrap()
                    .next_eligible_at
                    .is_none()
            );
            assert_eq!(
                historical_history(pool).await,
                before,
                "redrive/diagnostics {state} {budget}"
            );
        }
    }
}
#[test]
fn mutable_work_budget_rejects_historical_lifetime() {
    use screenroom_backend::ingest::WorkBudget;
    assert!(WorkBudget::from_selection(&json!({})).is_err());
    assert!(WorkBudget::from_selection(&json!({"work_budget":{"mode":"lifetime"}})).is_err());
    assert!(
        WorkBudget::from_selection(
            &json!({"work_budget":{"mode":"per_work_commit","max_requests":7,"max_records":10}})
        )
        .is_ok()
    );
}
#[tokio::test]
#[ignore = "requires explicit disposable PostgreSQL18 + pgvector admin URL"]
async fn postgres_expiry_rechecks_settlement_after_lock_wait() {
    let admin = sqlx::PgPool::connect(&std::env::var("SCREENROOM_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let name = format!("indexing_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect_with(admin.connect_options().as_ref().clone().database(&name))
        .await
        .unwrap();
    MIGRATOR.run(&pool).await.unwrap();
    let source:Uuid=sqlx::query_scalar("INSERT INTO catalog.sources(source_key,provider,configuration) VALUES('expiry-race','jellyfin','{\"adapter\":\"jellyfin\"}') RETURNING id").fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO catalog.libraries(source_id,public_id,external_id,title) VALUES($1,'a','a','a')").bind(source).execute(&pool).await.unwrap();
    let repo = IndexingRepository::new(pool.clone());
    repo.enroll_locator(source, &"0".repeat(64)).await.unwrap();
    let policy = repo
        .configure(
            source,
            &ConfigureRequest {
                request_id: Uuid::new_v4(),
                existing: None,
                configuration: configuration(),
            },
        )
        .await
        .unwrap();
    let request = || StartRequest {
        request_id: Uuid::new_v4(),
        policy_id: policy.id,
        policy_version: policy.version,
        scope: None,
    };
    let run = repo.start(&request()).await.unwrap();
    sqlx::query(
        "UPDATE ingest.runs SET started_at=clock_timestamp()-interval '2 hours' WHERE id=$1",
    )
    .bind(run.id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE ingest.indexing_runs ir SET candidate_watermark=r.started_at FROM ingest.runs r WHERE r.id=ir.run_id AND r.id=$1").bind(run.id).execute(&pool).await.unwrap();
    let candidate = repo
        .execution(run.id)
        .await
        .unwrap()
        .unwrap()
        .candidate_watermark
        .unwrap();

    // Stage an already-successful settlement, but hold its commit behind the source lock.
    let mut settlement = pool.begin().await.unwrap();
    let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *settlement)
        .await
        .unwrap();
    sqlx::query("SELECT id FROM catalog.sources WHERE id=$1 FOR UPDATE")
        .bind(source)
        .execute(&mut *settlement)
        .await
        .unwrap();
    sqlx::query("SELECT id FROM ingest.policies WHERE id=$1 FOR UPDATE")
        .bind(policy.id)
        .execute(&mut *settlement)
        .await
        .unwrap();
    sqlx::query("SELECT id FROM ingest.runs WHERE id=$1 FOR UPDATE")
        .bind(run.id)
        .execute(&mut *settlement)
        .await
        .unwrap();
    sqlx::query("UPDATE ingest.runs SET state='complete',finished_at=started_at+interval '3599 seconds' WHERE id=$1").bind(run.id).execute(&mut *settlement).await.unwrap();
    sqlx::query("UPDATE queue.jobs SET state='complete' WHERE scope_id=$1")
        .bind(run.id)
        .execute(&mut *settlement)
        .await
        .unwrap();
    sqlx::query("UPDATE ingest.indexing_runs ir SET settled_at=r.finished_at FROM ingest.runs r WHERE r.id=ir.run_id AND r.id=$1").bind(run.id).execute(&mut *settlement).await.unwrap();
    sqlx::query("UPDATE ingest.indexing_progress SET watermark=$2 WHERE policy_id=$1")
        .bind(policy.id)
        .bind(candidate)
        .execute(&mut *settlement)
        .await
        .unwrap();

    let ingest = IngestRepository::new(pool.clone());
    let expiry = tokio::spawn(async move { ingest.expire_deadlines().await });
    // Observing the lock wait proves expiry selected the old, unsettled MVCC candidate.
    tokio::time::timeout(std::time::Duration::from_secs(10),async {
        loop {
            let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM pg_stat_activity WHERE datname=current_database() AND $1=ANY(pg_blocking_pids(pid)))").bind(blocker).fetch_one(&pool).await.unwrap();
            if waiting {break;}
            tokio::task::yield_now().await;
        }
    }).await.expect("expiry must wait behind settlement, not finish before selecting its candidate");
    settlement.commit().await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), expiry)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let ingest = IngestRepository::new(pool.clone());
    assert_eq!(ingest.run(run.id).await.unwrap().unwrap().state, "complete");
    assert!(
        repo.execution(run.id)
            .await
            .unwrap()
            .unwrap()
            .settled_at
            .is_some()
    );
    assert_eq!(
        repo.progress(policy.id).await.unwrap().unwrap().watermark,
        Some(candidate)
    );
    let complete: bool =
        sqlx::query_scalar("SELECT bool_and(state='complete') FROM queue.jobs WHERE scope_id=$1")
            .bind(run.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(complete);

    let expired = repo.start(&request()).await.unwrap();
    sqlx::query(
        "UPDATE ingest.runs SET started_at=clock_timestamp()-interval '2 hours' WHERE id=$1",
    )
    .bind(expired.id)
    .execute(&pool)
    .await
    .unwrap();
    ingest.expire_deadlines().await.unwrap();
    assert_eq!(ingest.run(run.id).await.unwrap().unwrap().state, "complete");
    assert_eq!(
        ingest.run(expired.id).await.unwrap().unwrap().state,
        "failed"
    );
    assert_eq!(
        repo.progress(policy.id).await.unwrap().unwrap().watermark,
        Some(candidate)
    );
    pool.close().await;
    sqlx::query(&format!("DROP DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}
#[test]
fn canonical_validation_rejects_ineffective_options() {
    let c = configuration();
    c.validate().unwrap();
    assert!(!c.automatic_runs);
    assert!(c.artwork.posters.skip_existing);
    let mut bad = c.clone();
    bad.mode = Mode::Incremental;
    bad.validate().unwrap();
    assert_eq!(bad.overlap_seconds, 300);
    assert_eq!(bad.reconcile_seconds, 604800);
    bad.overlap_seconds = 0;
    assert!(bad.validate().is_err());
    for strategy in [
        Strategy::Missing,
        Strategy::Changed,
        Strategy::Stale,
        Strategy::Failed,
    ] {
        let mut bad = c.clone();
        bad.strategy = strategy;
        assert!(bad.validate().is_err());
    }
    let mut bad = c.clone();
    bad.components.metadata = false;
    assert!(bad.validate().is_err());
    let mut bad = c.clone();
    bad.limits.retry.cap_seconds = 1;
    assert!(bad.validate().is_err());
    let mut bad = c.clone();
    bad.limits.work_timeout_seconds = 241;
    assert!(bad.validate().is_err());
    let mut narrow = c.scope.clone();
    narrow.library_ids = vec!["foreign".into()];
    assert!(c.effective(Some(&narrow)).is_err());
    let mut repair = c;
    repair.mode = Mode::Repair;
    repair.components = Components {
        metadata: false,
        artwork: true,
        embeddings: false,
    };
    repair.strategy = Strategy::Missing;
    repair.validate().unwrap();
    repair.strategy = Strategy::All;
    assert!(repair.validate().is_err());
    repair.artwork.posters.skip_existing = false;
    repair.validate().unwrap();
    repair.strategy = Strategy::Changed;
    repair.validate().unwrap();
    repair.strategy = Strategy::Failed;
    repair.validate().unwrap();
    repair.strategy = Strategy::Stale;
    assert!(repair.validate().is_err());
    repair.stale_after_seconds = Some(60);
    repair.validate().unwrap();
    let mut unknown = json!(repair);
    unknown["source_concurrency"] = json!(5);
    assert!(Configuration::from_value(&unknown).is_err());
}

#[tokio::test]
#[ignore = "requires explicit disposable PostgreSQL18 + pgvector admin URL"]
async fn postgres_watermark_waits_for_successful_followups_and_never_cancel() {
    use screenroom_backend::ingest::{CheckpointWrite, Completion, PageCommit, TraversalWrites};
    let admin = sqlx::PgPool::connect(&std::env::var("SCREENROOM_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let name = format!("indexing_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_with(admin.connect_options().as_ref().clone().database(&name))
        .await
        .unwrap();
    MIGRATOR.run(&pool).await.unwrap();
    let source:Uuid=sqlx::query_scalar("INSERT INTO catalog.sources(source_key,provider,configuration) VALUES('settlement','jellyfin','{\"adapter\":\"jellyfin\"}') RETURNING id").fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO catalog.libraries(source_id,public_id,external_id,title) VALUES($1,'a','a','a')").bind(source).execute(&pool).await.unwrap();
    let repo = IndexingRepository::new(pool.clone());
    repo.enroll_locator(source, &"0".repeat(64)).await.unwrap();
    let mut c = configuration();
    c.mode = Mode::Incremental;
    let policy = repo
        .configure(
            source,
            &ConfigureRequest {
                request_id: Uuid::new_v4(),
                existing: None,
                configuration: c,
            },
        )
        .await
        .unwrap();
    let start = || StartRequest {
        request_id: Uuid::new_v4(),
        policy_id: policy.id,
        policy_version: policy.version,
        scope: None,
    };
    let run = repo.start(&start()).await.unwrap();
    let ingest = IngestRepository::new(pool.clone());
    let caps = vec![
        "adapter:jellyfin".into(),
        "source:settlement".into(),
        format!("locator:{}", "0".repeat(64)),
    ];
    let claim = ingest
        .claim_canonical("test", 60, &caps)
        .await
        .unwrap()
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    screenroom_backend::queue::enqueue(
        &mut tx,
        &screenroom_backend::queue::Enqueue {
            queue: "indexing",
            scope_id: run.id,
            kind: "settlement_probe",
            dedupe_key: "settlement-fixture",
            input: &json!({}),
            requirements: &[],
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert!(
        ingest
            .claim_canonical("other-replica", 60, &caps)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        ingest
            .queue_blockers(Some(run.id), Some(&caps))
            .await
            .unwrap()
            .source_concurrency,
        1
    );
    assert_eq!(
        ingest
            .queue_snapshot(Some(run.id), &caps)
            .await
            .unwrap()
            .blocked,
        1
    );
    ingest
        .commit_traversal_page(
            &claim,
            &PageCommit {
                libraries: vec![],
                items: vec![],
                artwork_inputs: vec![],
                artwork_omissions: vec![],
                followups: vec![],
                checkpoint: CheckpointWrite {
                    scope_key: "full-index-v1".into(),
                    cursor: json!({"phase":4}),
                    watermark: None,
                },
                requests: 0,
                completion: Completion::Complete {
                    evidence: json!({}),
                },
            },
            &TraversalWrites::default(),
        )
        .await
        .unwrap();
    assert!(
        repo.progress(policy.id)
            .await
            .unwrap()
            .unwrap()
            .watermark
            .is_none()
    );
    assert!(
        repo.execution(run.id)
            .await
            .unwrap()
            .unwrap()
            .settled_at
            .is_none()
    );
    let followup = ingest
        .claim_canonical("test", 60, &caps)
        .await
        .unwrap()
        .unwrap();
    ingest
        .fail(
            &followup,
            screenroom_backend::queue::FailureClass::Transient,
            "fixture_failure",
            None,
        )
        .await
        .unwrap();
    assert!(
        repo.progress(policy.id)
            .await
            .unwrap()
            .unwrap()
            .watermark
            .is_none()
    );
    sqlx::query("UPDATE queue.jobs SET available_at=clock_timestamp() WHERE id=$1")
        .bind(followup.id)
        .execute(&pool)
        .await
        .unwrap();
    let followup = ingest
        .claim_canonical("test", 60, &caps)
        .await
        .unwrap()
        .unwrap();
    ingest.complete_job(&followup).await.unwrap();
    assert_eq!(
        repo.progress(policy.id).await.unwrap().unwrap().watermark,
        Some(run.started_at)
    );
    let cancelled = repo.start(&start()).await.unwrap();
    ingest.cancel_run(cancelled.id).await.unwrap();
    assert_eq!(
        repo.progress(policy.id).await.unwrap().unwrap().watermark,
        Some(run.started_at)
    );
    assert!(
        repo.execution(cancelled.id)
            .await
            .unwrap()
            .unwrap()
            .settled_at
            .is_none()
    );
    pool.close().await;
    sqlx::query(&format!("DROP DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires explicit disposable PostgreSQL18 + pgvector admin URL"]
async fn postgres_canonical_admission_limits_and_identity() {
    let url = std::env::var("SCREENROOM_TEST_DATABASE_URL").unwrap();
    let admin = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .unwrap();
    let name = format!("indexing_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect_with(admin.connect_options().as_ref().clone().database(&name))
        .await
        .unwrap();
    MIGRATOR.run(&pool).await.unwrap();
    let source:Uuid=sqlx::query_scalar("INSERT INTO catalog.sources(source_key,provider,configuration) VALUES('canonical','jellyfin','{\"adapter\":\"jellyfin\"}') RETURNING id").fetch_one(&pool).await.unwrap();
    sqlx::query("INSERT INTO catalog.libraries(source_id,public_id,external_id,title) VALUES($1,'a','a','a')").bind(source).execute(&pool).await.unwrap();
    let repo = IndexingRepository::new(pool.clone());
    let configure = ConfigureRequest {
        request_id: Uuid::new_v4(),
        existing: None,
        configuration: configuration(),
    };
    assert!(matches!(
        repo.configure(source, &configure).await,
        Err(Error::Invalid("source locator requires enrollment"))
    ));
    repo.enroll_locator(source, &"0".repeat(64)).await.unwrap();
    let (one, two) = tokio::join!(
        repo.configure(source, &configure),
        repo.configure(source, &configure)
    );
    let p = one.unwrap();
    assert_eq!(p.id, two.unwrap().id);
    assert!(!p.automatic_runs);
    let mut conflict = configure.clone();
    conflict.configuration.cadence_seconds = 7200;
    assert!(matches!(
        repo.configure(source, &conflict).await,
        Err(Error::Stale)
    ));
    let ingest = IngestRepository::new(pool.clone());
    let request = StartRequest {
        request_id: Uuid::new_v4(),
        policy_id: p.id,
        policy_version: p.version,
        scope: None,
    };
    let run = repo.start(&request).await.unwrap();
    assert!(run.embedding_model_id.is_none());
    assert_eq!(run.id, repo.start(&request).await.unwrap().id);
    let claim = ingest
        .claim_canonical(
            "worker",
            60,
            &[
                "adapter:jellyfin".into(),
                "source:canonical".into(),
                format!("locator:{}", "0".repeat(64)),
            ],
        )
        .await
        .unwrap()
        .unwrap();
    let retry: (i32, i32, i32) = sqlx::query_as(
        "SELECT max_attempts,retry_base_seconds,retry_cap_seconds FROM queue.jobs WHERE id=$1",
    )
    .bind(claim.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(retry, (5, 5, 3600));
    let compatible = vec![
        "adapter:jellyfin".into(),
        "source:canonical".into(),
        format!("locator:{}", "0".repeat(64)),
    ];
    sqlx::query("UPDATE queue.jobs SET max_attempts=attempts,lease_until=clock_timestamp()-interval '1 second' WHERE id=$1").bind(claim.id).execute(&pool).await.unwrap();
    let exhausted = ingest
        .queue_snapshot(Some(run.id), &compatible)
        .await
        .unwrap();
    assert_eq!(
        (
            exhausted.exhausted,
            exhausted.reclaimable,
            exhausted.blocked
        ),
        (1, 0, 0)
    );
    let exhaustion_reason = ingest
        .queue_blockers(Some(run.id), Some(&compatible))
        .await
        .unwrap();
    assert_eq!(exhaustion_reason.attempt_limit, 1);
    assert!(exhaustion_reason.next_eligible_at.is_none());
    sqlx::query("UPDATE queue.jobs SET max_attempts=5,lease_until=clock_timestamp()+interval '1 minute' WHERE id=$1").bind(claim.id).execute(&pool).await.unwrap();
    sqlx::query("UPDATE ingest.indexing_runs SET observed_records=100 WHERE run_id=$1")
        .bind(run.id)
        .execute(&pool)
        .await
        .unwrap();
    let no_io_page = screenroom_backend::ingest::PageCommit {
        libraries: vec![],
        items: vec![],
        artwork_inputs: vec![],
        artwork_omissions: vec![],
        followups: vec![],
        requests: 0,
        checkpoint: screenroom_backend::ingest::CheckpointWrite {
            scope_key: "full-index-v1".into(),
            cursor: json!({"phase":4}),
            watermark: None,
        },
        completion: screenroom_backend::ingest::Completion::Complete {
            evidence: json!({}),
        },
    };
    let unchanged = screenroom_backend::ingest::TraversalWrites {
        library_touches: vec![screenroom_backend::ingest::LibraryTouch {
            external_id: "unchanged".into(),
            library_external_id: "a".into(),
        }],
        ..Default::default()
    };
    assert!(matches!(
        ingest
            .commit_traversal_page(&claim, &no_io_page, &unchanged)
            .await,
        Err(Error::Invalid("indexing lifetime records exhausted"))
    ));
    assert_eq!(ingest.run(run.id).await.unwrap().unwrap().items, 0);
    assert!(
        ingest
            .run_checkpoint(&run, "full-index-v1")
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query("UPDATE ingest.indexing_runs SET observed_records=0 WHERE run_id=$1")
        .bind(run.id)
        .execute(&pool)
        .await
        .unwrap();
    for _ in 0..10 {
        ingest
            .reserve_work_request(&claim, screenroom_backend::ingest::OperationClass::Page)
            .await
            .unwrap();
    }
    assert!(
        ingest
            .reserve_work_request(&claim, screenroom_backend::ingest::OperationClass::Page)
            .await
            .is_err()
    );
    sqlx::query("UPDATE ingest.runs SET requests=20 WHERE id=$1")
        .bind(run.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        ingest
            .reserve_work_request(&claim, screenroom_backend::ingest::OperationClass::Page)
            .await
            .is_err()
    );
    repo.enroll_locator(source, &"1".repeat(64)).await.unwrap();
    let changed_start = StartRequest {
        request_id: Uuid::new_v4(),
        ..request.clone()
    };
    assert!(matches!(
        repo.start(&changed_start).await,
        Err(Error::Stale)
    ));
    assert!(matches!(ingest.renew(&claim, 60).await, Err(Error::Stale)));
    repo.enroll_locator(source, &"0".repeat(64)).await.unwrap();
    sqlx::query("UPDATE catalog.sources SET configuration='{\"adapter\":\"changed\"}' WHERE id=$1")
        .bind(source)
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(ingest.renew(&claim, 60).await, Err(Error::Stale)));
    assert!(
        ingest
            .claim_canonical(
                "other",
                60,
                &[
                    "adapter:jellyfin".into(),
                    "source:canonical".into(),
                    format!("locator:{}", "0".repeat(64))
                ]
            )
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query(
        "UPDATE queue.jobs SET lease_until=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(claim.id)
    .execute(&pool)
    .await
    .unwrap();
    let caps = vec![
        "adapter:jellyfin".to_owned(),
        "source:canonical".to_owned(),
        format!("locator:{}", "0".repeat(64)),
    ];
    let snapshot = ingest.queue_snapshot(Some(run.id), &caps).await.unwrap();
    assert_eq!(
        (snapshot.leased, snapshot.reclaimable, snapshot.blocked),
        (1, 0, 1)
    );
    let blockers = ingest
        .queue_blockers(Some(run.id), Some(&caps))
        .await
        .unwrap();
    assert_eq!(blockers.source_changed, 1);
    assert_eq!(blockers.capability, Some(0));
    assert!(blockers.next_eligible_at.is_none());
    assert!(
        ingest
            .queue_blockers(Some(run.id), None)
            .await
            .unwrap()
            .capability
            .is_none()
    );
    assert!(
        ingest
            .claim_canonical("other", 60, &caps)
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query("UPDATE catalog.sources s SET configuration=ir.source_configuration FROM ingest.indexing_runs ir WHERE ir.run_id=$1 AND s.id=$2").bind(run.id).bind(source).execute(&pool).await.unwrap();
    sqlx::query(
        "UPDATE queue.jobs SET lease_until=clock_timestamp()+interval '1 minute' WHERE id=$1",
    )
    .bind(claim.id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE ingest.runs SET started_at=clock_timestamp()-interval '3599 seconds' WHERE id=$1",
    )
    .bind(run.id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("CREATE FUNCTION ingest.test_commit_deadline() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN IF NEW.state='complete' AND OLD.state<>'complete' THEN PERFORM pg_sleep(1.1); END IF; RETURN NEW; END $$").execute(&pool).await.unwrap();
    sqlx::query("CREATE TRIGGER test_commit_deadline BEFORE UPDATE ON ingest.runs FOR EACH ROW EXECUTE FUNCTION ingest.test_commit_deadline()").execute(&pool).await.unwrap();
    assert!(matches!(
        ingest
            .commit_traversal_page(&claim, &no_io_page, &Default::default())
            .await,
        Err(Error::Invalid("indexing deadline expired"))
    ));
    assert_eq!(ingest.run(run.id).await.unwrap().unwrap().state, "running");
    assert!(
        ingest
            .run_checkpoint(&run, "full-index-v1")
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query("UPDATE ingest.runs SET started_at=started_at-interval '2 hours' WHERE id=$1")
        .bind(run.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        ingest
            .commit_traversal_page(&claim, &no_io_page, &Default::default())
            .await,
        Err(Error::Invalid("indexing deadline expired"))
    ));
    sqlx::query(
        "UPDATE queue.jobs SET lease_until=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(claim.id)
    .execute(&pool)
    .await
    .unwrap();
    let expired_counts = ingest.queue_snapshot(Some(run.id), &caps).await.unwrap();
    assert_eq!((expired_counts.reclaimable, expired_counts.blocked), (0, 1));
    assert_eq!(
        ingest
            .queue_blockers(Some(run.id), Some(&caps))
            .await
            .unwrap()
            .deadline_expired,
        1
    );
    ingest.expire_deadlines().await.unwrap();
    assert_eq!(ingest.run(run.id).await.unwrap().unwrap().state, "failed");
    assert_eq!(
        ingest
            .redrive_eligibility(claim.id)
            .await
            .unwrap()
            .reason
            .as_deref(),
        Some("deadline_expired")
    );
    assert!(
        repo.progress(p.id)
            .await
            .unwrap()
            .unwrap()
            .watermark
            .is_none()
    );
    pool.close().await;
    sqlx::query(&format!("DROP DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}
