use screenroom_backend::{
    Error, Uuid,
    db::MIGRATOR,
    indexing::{Configuration, ConfigureRequest, IndexingRepository, StartRequest},
    ingest::{CheckpointWrite, Completion, IngestRepository, PageCommit, WorkBudget},
    repair::*,
    sqlx,
};
use serde_json::json;

fn execution() -> RepairExecution {
    serde_json::from_value(json!({"repair":1,"scope":{"library_ids":["a","b"],"item_kinds":["movie","series"]},"artwork":{"posters":{}},"work_budget":{"mode":"per_work_commit","max_requests":10,"max_records":20}})).unwrap()
}
#[test]
fn repair_execution_has_no_legacy_decoder_and_enforces_budget_and_narrowing() {
    let mut s = execution();
    assert!(s.validate().is_ok());
    assert!(s.artwork.posters.skip_existing);
    let mut scope = s.scope.clone();
    scope.library_ids = vec!["a".into()];
    assert!(s.effective(Some(&scope)).is_ok());
    scope.library_ids.push("foreign".into());
    assert!(s.effective(Some(&scope)).is_err());
    s.work_budget = WorkBudget::PerWorkCommit {
        max_requests: 6,
        max_records: 20,
    };
    assert!(s.validate().is_err());
    let mut value = json!(execution());
    value.as_object_mut().unwrap().remove("repair");
    value["maintenance"] = json!(1);
    assert!(RepairExecution::from_value(&value).is_err());
}
fn page(completion: Completion) -> PageCommit {
    PageCommit {
        libraries: vec![],
        items: vec![],
        artwork_inputs: vec![],
        artwork_omissions: vec![],
        followups: vec![],
        checkpoint: CheckpointWrite {
            scope_key: "poster-repair-v1".into(),
            cursor: json!({}),
            watermark: None,
        },
        requests: 0,
        completion,
    }
}
#[tokio::test]
#[ignore = "requires task-owned SCREENROOM_TEST_DATABASE_URL"]
async fn postgres_canonical_repair_atomic_candidates_fences_history_and_failed_readiness() {
    let admin = sqlx::PgPool::connect(&std::env::var("SCREENROOM_TEST_DATABASE_URL").unwrap())
        .await
        .unwrap();
    let name = format!("repair_{}", Uuid::new_v4().simple());
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
    let source:Uuid=sqlx::query_scalar("INSERT INTO catalog.sources(source_key,provider,configuration) VALUES('owned','jellyfin','{\"adapter\":\"jellyfin\"}') RETURNING id").fetch_one(&pool).await.unwrap();
    for library in ["a", "b"] {
        sqlx::query("INSERT INTO catalog.libraries(source_id,public_id,external_id,title) VALUES($1,$2,$2,$2)").bind(source).bind(library).execute(&pool).await.unwrap();
    }
    let mut items = vec![];
    for id in ["one", "two"] {
        let item: Uuid = sqlx::query_scalar(
            "INSERT INTO catalog.items(public_id,kind) VALUES($1,'movie') RETURNING id",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let source_item:Uuid=sqlx::query_scalar("INSERT INTO catalog.source_items(source_id,item_id,external_id,metadata,content_hash) VALUES($1,$2,$3,'{}',decode(repeat('00',32),'hex')) RETURNING id").bind(source).bind(item).bind(id).fetch_one(&pool).await.unwrap();
        sqlx::query("INSERT INTO catalog.library_items(library_id,source_item_id,source_id,last_seen_run_id) SELECT id,$2,$1,$3 FROM catalog.libraries WHERE source_id=$1").bind(source).bind(source_item).bind(Uuid::new_v4()).execute(&pool).await.unwrap();
        items.push(source_item);
    }
    let planner = IndexingRepository::new(pool.clone());
    planner
        .enroll_locator(source, &"0".repeat(64))
        .await
        .unwrap();
    let configuration:Configuration=serde_json::from_value(json!({"indexing":1,"enabled":true,"mode":"repair","cadence_seconds":60,"strategy":"missing","components":{"metadata":false,"artwork":true,"embeddings":false},"scope":execution().scope,"operator_profile_id":"operator","limits":{"work_requests":10,"work_records":20,"lifetime_requests":100,"lifetime_records":100,"deadline_seconds":3600,"work_timeout_seconds":30}})).unwrap();
    let policy = planner
        .configure(
            source,
            &ConfigureRequest {
                request_id: Uuid::new_v4(),
                existing: None,
                configuration,
            },
        )
        .await
        .unwrap();
    let request = StartRequest {
        request_id: Uuid::new_v4(),
        policy_id: policy.id,
        policy_version: policy.version,
        scope: None,
    };
    let (one, two) = tokio::join!(planner.start(&request), planner.start(&request));
    let run = one.unwrap();
    assert_eq!(run.id, two.unwrap().id);
    assert!(run.embedding_model_id.is_none());
    let repair = RepairRepository::new(pool.clone());
    let ingest = IngestRepository::new(pool.clone());
    assert_eq!(
        repair
            .candidates(run.id, None, 100, true)
            .await
            .unwrap()
            .len(),
        2
    );
    let caps: Vec<String> =
        sqlx::query_scalar("SELECT DISTINCT unnest(requirements) FROM queue.jobs")
            .fetch_all(&pool)
            .await
            .unwrap();
    let claim = ingest
        .claim_canonical("first", 60, &caps)
        .await
        .unwrap()
        .unwrap();
    let write = |id, disposition| RepairWrites {
        candidates: vec![CandidateWrite {
            source_item_id: id,
            disposition,
            code: Some(OutcomeCode::NoPoster),
            source_artwork_id: None,
        }],
        outcomes: vec![],
    };
    assert!(
        ingest
            .commit_repair_page(
                &claim,
                &page(Completion::Continue),
                &write(items[0], Disposition::SkippedReady)
            )
            .await
            .is_err()
    );
    assert!(
        ingest
            .commit_repair_page(
                &claim,
                &page(Completion::Continue),
                &write(items[0], Disposition::Staged)
            )
            .await
            .is_err()
    );
    assert_eq!(
        repair
            .candidates(run.id, None, 100, true)
            .await
            .unwrap()
            .len(),
        2
    );
    ingest.pause(run.id).await.unwrap();
    assert!(matches!(
        ingest
            .commit_repair_page(
                &claim,
                &page(Completion::Continue),
                &write(items[0], Disposition::Unavailable)
            )
            .await,
        Err(Error::Stale)
    ));
    ingest.resume(run.id).await.unwrap();
    for (position, item) in items.iter().enumerate() {
        let claim = ingest
            .claim_canonical("resumed", 60, &caps)
            .await
            .unwrap()
            .unwrap();
        let writes = write(*item, Disposition::Unavailable);
        if position == 0 {
            assert!(
                ingest
                    .commit_repair_page(
                        &claim,
                        &page(Completion::Complete {
                            evidence: json!({})
                        }),
                        &writes
                    )
                    .await
                    .is_err()
            );
        }
        ingest
            .commit_repair_page(&claim, &page(Completion::Continue), &writes)
            .await
            .unwrap();
    }
    let claim = ingest
        .claim_canonical("finish", 60, &caps)
        .await
        .unwrap()
        .unwrap();
    ingest
        .commit_repair_page(
            &claim,
            &page(Completion::Complete {
                evidence: json!({"authoritative":false}),
            }),
            &RepairWrites::default(),
        )
        .await
        .unwrap();
    let status = repair.run_status(run.id).await.unwrap();
    assert!(status.enumeration_complete);
    assert!(status.work_settled);
    assert!(!status.work_succeeded);
    assert_eq!(status.candidates.unavailable, 2);
    assert_eq!(status.run.items, 0);
    let first = repair.candidates(run.id, None, 1, false).await.unwrap();
    let second = repair
        .candidates(run.id, Some(first[0].source_item_id), 1, false)
        .await
        .unwrap();
    assert_ne!(first[0].source_item_id, second[0].source_item_id);
    assert_eq!(repair.runs(Some(source), None, 100).await.unwrap().len(), 1);
    assert!(
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT to_regclass('ingest.maintenance_starts')::text"
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .is_none()
    );
    pool.close().await;
    sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}
