//! Real Jellyfin execution over task-local HTTP and disposable PostgreSQL.
use screenroom_backend::{
    Uuid,
    indexing::{Configuration, ConfigureRequest, IndexingRepository},
    ingest::{IngestRepository, OperationClass},
    repair::RepairRepository,
    sqlx,
};
use screenroom_index_worker::Step;
use serde_json::json;
use std::sync::atomic::Ordering;
#[path = "support/http.rs"]
mod http;
use http::{Fixture, Http, drain, ready};

#[tokio::test]
#[ignore = "requires task-owned SCREENROOM_W_TEST_DATABASE_URL"]
async fn managed_connection_becomes_usable_without_restart_and_replacement_is_fenced() {
    use screenroom_backend::connections::{ConnectionRepository, CredentialDirectory, SaveRequest};
    let http = Http::new().await;
    let f = Fixture::empty().await;
    let w = http.worker(f.pool.clone(), false, "unused").await;
    assert!(w.capabilities().await.unwrap().is_empty());
    assert_eq!(w.step().await.unwrap(), Step::Idle);
    assert_eq!(http.state.calls.load(Ordering::SeqCst), 0);
    let user = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO identity.users(id,public_id,display_name) VALUES($1,$2,'Actual local user')",
    )
    .bind(user)
    .bind(user.to_string())
    .execute(&f.pool)
    .await
    .unwrap();
    let connections = ConnectionRepository::new(
        f.pool.clone(),
        Some(CredentialDirectory::new(http.credential_directory()).unwrap()),
    );
    let saved = connections
        .save(&SaveRequest {
            request_id: Uuid::new_v4(),
            base_url: http.url.clone(),
            api_key: "test-only-secret".into(),
            server_id: "local-http-server".into(),
            profile_id: "selected".into(),
            library_ids: vec!["movies".into()],
            local_user_id: user,
            refresh: true,
        })
        .await
        .unwrap();
    let run = saved.run_id.unwrap();
    assert!(
        w.capabilities()
            .await
            .unwrap()
            .contains(&format!("source:{}", saved.connection.source_key))
    );
    drain(&w).await;
    let status = RepairRepository::new(f.pool.clone())
        .run_status(run)
        .await
        .unwrap();
    assert!(status.work_succeeded, "{status:?}");
    assert_eq!(status.run.items, 6);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM search.embedding_models")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        0
    );
    let second = IndexingRepository::new(f.pool.clone())
        .start(&screenroom_backend::indexing::StartRequest {
            request_id: Uuid::new_v4(),
            policy_id: saved.policy_id,
            policy_version: saved.policy_version,
            scope: None,
        })
        .await
        .unwrap();
    http.state.replacement_server.store(1, Ordering::SeqCst);
    let calls = http.state.calls.load(Ordering::SeqCst);
    assert_eq!(w.step().await.unwrap(), Step::Failed);
    assert_eq!(http.state.calls.load(Ordering::SeqCst), calls + 1);
    let blocked = RepairRepository::new(f.pool.clone())
        .run_status(second.id)
        .await
        .unwrap();
    assert!(!blocked.work_succeeded);
    assert_eq!(blocked.run.items, 0);
    assert_eq!(blocked.coverage.permanent_attempts, 1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM catalog.source_items")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        6
    );
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires task-owned SCREENROOM_W_TEST_DATABASE_URL"]
async fn successful_phase_transitions_do_not_exhaust_single_attempt_policy() {
    let http = Http::new().await;
    http.state.count.store(4, Ordering::SeqCst);
    let f = Fixture::new(&http).await;
    let w = http.worker(f.pool.clone(), false, "unused").await;
    let policy = f.policy("full", true, false, "all").await;
    let mut configuration = Configuration::from_value(&policy.selection_json).unwrap();
    configuration.limits.retry.max_attempts = 1;
    let policy = IndexingRepository::new(f.pool.clone())
        .configure(
            f.source,
            &ConfigureRequest {
                request_id: Uuid::new_v4(),
                existing: Some(screenroom_backend::indexing::Existing {
                    policy_id: policy.id,
                    expected_version: policy.version,
                }),
                configuration,
            },
        )
        .await
        .unwrap();
    for _ in 0..2 {
        let run = f.start(&policy).await;
        drain(&w).await;
        let status = RepairRepository::new(f.pool.clone())
            .run_status(run.id)
            .await
            .unwrap();
        assert!(status.work_succeeded, "{status:?}");
        assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM queue.attempts a JOIN queue.jobs j ON j.id=a.job_id WHERE j.scope_id=$1 AND a.outcome NOT IN ('released','complete')")
            .bind(run.id).fetch_one(&f.pool).await.unwrap(), 0);
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM media.artwork_generations WHERE state='published'"
            )
            .fetch_one(&f.pool)
            .await
            .unwrap(),
            4
        );
        assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM ingest.request_receipts WHERE run_id=$1 AND outcome<>'success'")
            .bind(run.id).fetch_one(&f.pool).await.unwrap(), 0);
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM queue.jobs WHERE scope_id=$1 AND max_attempts<>1"
            )
            .bind(run.id)
            .fetch_one(&f.pool)
            .await
            .unwrap(),
            0
        );
    }
    assert!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM queue.jobs WHERE kind='artwork' AND state='complete'"
        )
        .fetch_one(&f.pool)
        .await
        .unwrap()
            > 0
    );
    f.cleanup().await;
}

async fn automatic_policies(
    f: &Fixture,
    include_compatible: bool,
) -> Option<screenroom_backend::ingest::Policy> {
    let incompatible = f.policy("full", false, true, "all").await;
    let mut configuration = Configuration::from_value(&incompatible.selection_json).unwrap();
    configuration.automatic_runs = true;
    let indexing = IndexingRepository::new(f.pool.clone());
    for _ in 0..101 {
        indexing
            .configure(
                f.source,
                &ConfigureRequest {
                    request_id: Uuid::new_v4(),
                    existing: None,
                    configuration: configuration.clone(),
                },
            )
            .await
            .unwrap();
    }
    let compatible = if include_compatible {
        let mut compatible = configuration;
        compatible.components.embeddings = false;
        compatible.components.artwork = true;
        Some(
            indexing
                .configure(
                    f.source,
                    &ConfigureRequest {
                        request_id: Uuid::new_v4(),
                        existing: None,
                        configuration: compatible,
                    },
                )
                .await
                .unwrap(),
        )
    } else {
        None
    };
    sqlx::query("UPDATE ingest.policies SET next_due_at=clock_timestamp()-interval '2 hours' WHERE automatic_runs")
        .execute(&f.pool).await.unwrap();
    if let Some(compatible) = &compatible {
        sqlx::query(
        "UPDATE ingest.policies SET next_due_at=clock_timestamp()-interval '1 hour' WHERE id=$1",
    )
    .bind(compatible.id)
    .execute(&f.pool)
    .await
    .unwrap();
    }
    compatible
}

#[tokio::test]
#[ignore = "requires task-owned SCREENROOM_W_TEST_DATABASE_URL"]
async fn bounded_scheduler_continuation_admits_compatible_policy_after_101_incompatible() {
    let http = Http::new().await;
    let f = Fixture::new(&http).await;
    let w = http.worker(f.pool.clone(), false, "unused").await;
    let compatible = automatic_policies(&f, true).await.unwrap();
    assert_eq!(w.start_due().await.unwrap(), (0, false));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM ingest.runs")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(w.start_due().await.unwrap(), (1, true));
    let run: Uuid = sqlx::query_scalar("SELECT id FROM ingest.runs WHERE policy_id=$1")
        .bind(compatible.id)
        .fetch_one(&f.pool)
        .await
        .unwrap();
    drain(&w).await;
    assert!(
        RepairRepository::new(f.pool.clone())
            .run_status(run)
            .await
            .unwrap()
            .work_succeeded
    );
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM ingest.runs r JOIN ingest.policies p ON p.id=r.policy_id WHERE p.selection_json#>>'{components,embeddings}'='true'").fetch_one(&f.pool).await.unwrap(), 0);
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM queue.attempts a JOIN queue.jobs j ON j.id=a.job_id WHERE j.scope_id<>$1").bind(run).fetch_one(&f.pool).await.unwrap(), 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM search.embedding_models")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        0
    );
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires task-owned SCREENROOM_W_TEST_DATABASE_URL"]
async fn invalid_due_policy_is_deferred_without_skipping_the_next_policy() {
    let http = Http::new().await;
    let f = Fixture::new(&http).await;
    let worker = http.worker(f.pool.clone(), false, "unused").await;
    let compatible = automatic_policies(&f, true).await.unwrap();
    let invalid: Uuid = sqlx::query_scalar(
        "UPDATE ingest.policies SET selection_json=jsonb_set(selection_json,'{mode}','\"invalid\"'),next_due_at=clock_timestamp()-interval '3 hours'
         WHERE id=(SELECT id FROM ingest.policies WHERE automatic_runs AND id<>$1 ORDER BY id LIMIT 1) RETURNING id",
    )
    .bind(compatible.id)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(worker.start_due().await.unwrap(), (0, false));
    assert!(
        sqlx::query_scalar::<_, bool>(
            "SELECT next_due_at>clock_timestamp() FROM ingest.policies WHERE id=$1"
        )
        .bind(invalid)
        .fetch_one(&f.pool)
        .await
        .unwrap()
    );
    assert_eq!(worker.start_due().await.unwrap(), (1, true));
    let request = screenroom_backend::indexing::StartRequest {
        request_id: Uuid::new_v4(),
        policy_id: compatible.id,
        policy_version: compatible.version,
        scope: None,
    };
    assert!(matches!(
        IndexingRepository::new(f.pool.clone())
            .start_automatic(&request)
            .await,
        Err(screenroom_backend::Error::Stale)
    ));
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires task-owned SCREENROOM_W_TEST_DATABASE_URL"]
async fn automatic_embedding_policy_without_model_does_not_stop_compatible_work() {
    let http = Http::new().await;
    let f = Fixture::new(&http).await;
    let w = http.worker(f.pool.clone(), false, "unused").await;
    let compatible = f.policy("full", true, false, "all").await;
    let incompatible = f.policy("full", false, true, "all").await;
    let mut configuration = Configuration::from_value(&incompatible.selection_json).unwrap();
    configuration.automatic_runs = true;
    let incompatible = IndexingRepository::new(f.pool.clone())
        .configure(
            f.source,
            &ConfigureRequest {
                request_id: Uuid::new_v4(),
                existing: Some(screenroom_backend::indexing::Existing {
                    policy_id: incompatible.id,
                    expected_version: incompatible.version,
                }),
                configuration,
            },
        )
        .await
        .unwrap();
    sqlx::query("UPDATE ingest.policies SET next_due_at=clock_timestamp() WHERE id=$1")
        .bind(incompatible.id)
        .execute(&f.pool)
        .await
        .unwrap();
    assert_eq!(w.start_due().await.unwrap(), (0, true));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM ingest.runs")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        0
    );
    let run = f.start(&compatible).await;
    assert_eq!(w.start_due().await.unwrap(), (0, true));
    drain(&w).await;
    assert!(
        RepairRepository::new(f.pool.clone())
            .run_status(run.id)
            .await
            .unwrap()
            .work_succeeded
    );
    assert_eq!(w.start_due().await.unwrap(), (0, true));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM ingest.runs WHERE policy_id=$1")
            .bind(incompatible.id)
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM queue.jobs WHERE scope_id IN (SELECT id FROM ingest.runs WHERE policy_id=$1)")
        .bind(incompatible.id).fetch_one(&f.pool).await.unwrap(), 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM search.embedding_models")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(w.step().await.unwrap(), Step::Idle);
    sqlx::query("UPDATE ingest.policies SET selection_json=jsonb_set(selection_json,'{limits,work_timeout_seconds}','0') WHERE id=$1")
        .bind(incompatible.id).execute(&f.pool).await.unwrap();
    assert!(matches!(
        w.start_due().await,
        Err(screenroom_backend::Error::Invalid(_))
    ));
    f.cleanup().await;
}

struct WorkerProcess(std::process::Child);
impl Drop for WorkerProcess {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn worker_process(http: &Http, f: &Fixture, command: &str) -> WorkerProcess {
    use std::process::{Command, Stdio};
    WorkerProcess(
        Command::new(env!("CARGO_BIN_EXE_screenroom-index-worker"))
            .arg(command)
            .env_clear()
            .env("DATABASE_URL", f.database_url())
            .env(
                "SCREENROOM_JELLYFIN_CREDENTIAL_DIRECTORY",
                http.credential_directory(),
            )
            .env("SCREENROOM_S3_ENDPOINT", &http.url)
            .env("SCREENROOM_S3_REGION", "garage")
            .env("SCREENROOM_S3_BUCKET", "objects")
            .env("SCREENROOM_S3_ACCESS_KEY_ID", "test")
            .env("SCREENROOM_S3_SECRET_ACCESS_KEY", "test-only")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    )
}

async fn process_scheduling_sweep(command: &str, include_compatible: bool) {
    use std::time::Duration;
    let http = Http::new().await;
    http.state.count.store(1, Ordering::SeqCst);
    let f = Fixture::new(&http).await;
    let compatible = automatic_policies(&f, include_compatible).await;
    let mut process = worker_process(&http, &f, command);
    if command == "serve" {
        let policy = compatible.as_ref().unwrap();
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                assert!(
                    process.0.try_wait().unwrap().is_none(),
                    "serve exited prematurely"
                );
                let run: Option<Uuid> =
                    sqlx::query_scalar("SELECT id FROM ingest.runs WHERE policy_id=$1")
                        .bind(policy.id)
                        .fetch_optional(&f.pool)
                        .await
                        .unwrap();
                if let Some(run) = run
                    && RepairRepository::new(f.pool.clone())
                        .run_status(run)
                        .await
                        .unwrap()
                        .work_succeeded
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("serve never reached the compatible automatic policy");
        assert!(
            std::process::Command::new("/bin/kill")
                .args(["-TERM", &process.0.id().to_string()])
                .status()
                .unwrap()
                .success()
        );
    }
    let status = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let Some(status) = process.0.try_wait().unwrap() {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("worker did not terminate after a scheduling sweep or shutdown signal");
    assert!(status.success(), "worker failed: {status}");
    if let Some(compatible) = compatible {
        let run: Uuid = sqlx::query_scalar("SELECT id FROM ingest.runs WHERE policy_id=$1")
            .bind(compatible.id)
            .fetch_one(&f.pool)
            .await
            .unwrap();
        assert!(
            RepairRepository::new(f.pool.clone())
                .run_status(run)
                .await
                .unwrap()
                .work_succeeded
        );
        assert!(http.state.calls.load(Ordering::SeqCst) > 0);
        assert!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM queue.attempts a JOIN queue.jobs j ON j.id=a.job_id WHERE j.scope_id=$1")
            .bind(run).fetch_one(&f.pool).await.unwrap()>0);
    } else {
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM ingest.runs")
                .fetch_one(&f.pool)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM queue.jobs")
                .fetch_one(&f.pool)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM queue.attempts")
                .fetch_one(&f.pool)
                .await
                .unwrap(),
            0
        );
        assert_eq!(http.state.calls.load(Ordering::SeqCst), 0);
    }
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM ingest.runs r JOIN ingest.policies p ON p.id=r.policy_id WHERE p.selection_json#>>'{components,embeddings}'='true'").fetch_one(&f.pool).await.unwrap(), 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM search.embedding_models")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        0
    );
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires task-owned SCREENROOM_W_TEST_DATABASE_URL"]
async fn process_drain_admits_later_automatic_policy_after_101_incompatible() {
    process_scheduling_sweep("drain", true).await;
}

#[tokio::test]
#[ignore = "requires task-owned SCREENROOM_W_TEST_DATABASE_URL"]
async fn process_drain_terminates_after_101_incompatible_policies_only() {
    process_scheduling_sweep("drain", false).await;
}

#[tokio::test]
#[ignore = "requires task-owned SCREENROOM_W_TEST_DATABASE_URL"]
async fn process_serve_admits_later_automatic_policy_after_101_incompatible() {
    process_scheduling_sweep("serve", true).await;
}

#[tokio::test]
#[ignore = "requires task-owned SCREENROOM_W_TEST_DATABASE_URL"]
async fn sigterm_during_blocked_upload_reclaims_lease_without_false_publication() {
    use std::{process::Command, time::Duration};
    let http = Http::new().await;
    http.state.count.store(1, Ordering::SeqCst);
    let f = Fixture::new(&http).await;
    let replacement = http.worker(f.pool.clone(), false, "unused").await;
    let policy = f.policy("full", true, false, "all").await;
    let run = f.start(&policy).await;
    for _ in 0..20 {
        if sqlx::query_scalar::<_, i64>("SELECT count(*) FROM queue.jobs WHERE scope_id=$1 AND kind='artwork' AND state='ready'")
            .bind(run.id).fetch_one(&f.pool).await.unwrap() > 0 { break; }
        assert_eq!(replacement.step().await.unwrap(), Step::Committed);
    }
    http.state.block_upload.store(1, Ordering::SeqCst);
    let mut process = worker_process(&http, &f, "serve");
    tokio::time::timeout(
        Duration::from_secs(15),
        http.state.upload_started.notified(),
    )
    .await
    .unwrap();
    let leased: (Uuid, i64) = sqlx::query_as(
        "SELECT id,fence FROM queue.jobs WHERE scope_id=$1 AND kind='artwork' AND state='leased'",
    )
    .bind(run.id)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM media.artwork_generations WHERE state='published'"
        )
        .fetch_one(&f.pool)
        .await
        .unwrap(),
        0
    );
    assert!(
        Command::new("/bin/kill")
            .args(["-TERM", &process.0.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    let status = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(status) = process.0.try_wait().unwrap() {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        status.success(),
        "worker did not shut down cleanly: {status}"
    );
    tokio::time::timeout(
        Duration::from_secs(5),
        http.state.upload_cancelled.notified(),
    )
    .await
    .unwrap();
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM queue.jobs WHERE id=$1")
            .bind(leased.0)
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        "leased"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM media.artwork_generations WHERE state='published'"
        )
        .fetch_one(&f.pool)
        .await
        .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM media.objects")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        0
    );
    sqlx::query(
        "UPDATE queue.jobs SET lease_until=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(leased.0)
    .execute(&f.pool)
    .await
    .unwrap();
    http.state.block_upload.store(0, Ordering::SeqCst);
    drain(&replacement).await;
    assert!(
        RepairRepository::new(f.pool.clone())
            .run_status(run.id)
            .await
            .unwrap()
            .work_succeeded
    );
    let completed: (String, i64) = sqlx::query_as("SELECT state,fence FROM queue.jobs WHERE id=$1")
        .bind(leased.0)
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(completed.0, "complete");
    assert!(completed.1 > leased.1);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM queue.attempts WHERE job_id=$1")
            .bind(leased.0)
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM media.artwork_generations WHERE state='published'"
        )
        .fetch_one(&f.pool)
        .await
        .unwrap(),
        1
    );
    f.cleanup().await;
}

async fn selected_items(f: &Fixture, run: Uuid) -> Vec<(String, Option<String>)> {
    sqlx::query_as("SELECT external_id,selection_reason FROM ingest.maintenance_candidates WHERE run_id=$1 ORDER BY external_id")
        .bind(run).fetch_all(&f.pool).await.unwrap()
}

#[tokio::test]
#[ignore = "requires task-owned SCREENROOM_W_TEST_DATABASE_URL"]
async fn repair_changed_stale_unknown_age_and_current_failed_generation_selection() {
    let http = Http::new().await;
    http.state.count.store(3, Ordering::SeqCst);
    let f = Fixture::new(&http).await;
    let w = http.worker(f.pool.clone(), false, "unused").await;
    let ingest = IngestRepository::new(f.pool.clone());
    let full = f.policy("full", true, false, "all").await;
    f.start(&full).await;
    drain(&w).await;

    let changed = f.policy("repair", true, false, "changed").await;
    let unchanged = f.start(&changed).await;
    assert!(selected_items(&f, unchanged.id).await.is_empty());
    drain(&w).await;
    sqlx::query("UPDATE catalog.source_items SET metadata=jsonb_set(metadata,'{provider_artwork_inputs,0,tag}','\"b\"') WHERE external_id='movie0'")
        .execute(&f.pool).await.unwrap();
    http.state.tag.store(1, Ordering::SeqCst);
    http.state.image.store(1, Ordering::SeqCst);
    let changed_run = f.start(&changed).await;
    assert_eq!(
        selected_items(&f, changed_run.id)
            .await
            .iter()
            .map(|v| v.0.as_str())
            .collect::<Vec<_>>(),
        ["movie0"]
    );
    drain(&w).await;
    let unchanged = f.start(&changed).await;
    assert!(selected_items(&f, unchanged.id).await.is_empty());
    drain(&w).await;

    sqlx::query("UPDATE media.artwork_generations g SET published_at=CASE WHEN si.external_id='movie1' THEN NULL ELSE clock_timestamp()-interval '2 hours' END FROM media.item_artwork ia JOIN catalog.source_items si ON si.id=ia.source_item_id WHERE ia.artwork_id=g.artwork_id AND si.external_id IN ('movie0','movie1') AND g.state='published'")
        .execute(&f.pool).await.unwrap();
    sqlx::query("UPDATE ingest.maintenance_candidates SET updated_at=clock_timestamp()-interval '2 hours' WHERE external_id='movie0'")
        .execute(&f.pool).await.unwrap();
    let mut stale = Configuration::from_value(&changed.selection_json).unwrap();
    stale.strategy = screenroom_backend::indexing::Strategy::Stale;
    stale.stale_after_seconds = Some(3600);
    let stale = IndexingRepository::new(f.pool.clone())
        .configure(
            f.source,
            &ConfigureRequest {
                request_id: Uuid::new_v4(),
                existing: None,
                configuration: stale,
            },
        )
        .await
        .unwrap();
    let stale_run = f.start(&stale).await;
    assert_eq!(
        selected_items(&f, stale_run.id).await,
        vec![
            ("movie0".into(), Some("known_stale".into())),
            ("movie1".into(), Some("unknown_age".into())),
        ]
    );
    ingest.cancel_run(stale_run.id).await.unwrap();

    http.state.single_changed.store(1, Ordering::SeqCst);
    http.state.tag.store(2, Ordering::SeqCst);
    http.state.image.store(2, Ordering::SeqCst);
    let failed_full = f.start(&full).await;
    for _ in 0..100 {
        if sqlx::query_scalar::<_, i64>("SELECT count(*) FROM queue.jobs WHERE scope_id=$1 AND kind='artwork' AND state='ready'")
            .bind(failed_full.id).fetch_one(&f.pool).await.unwrap() > 0 { break; }
        assert_eq!(w.step().await.unwrap(), Step::Committed);
    }
    http.state.storage_status.store(503, Ordering::SeqCst);
    let mut failures = 0;
    for _ in 0..100 {
        match w.step().await.unwrap() {
            Step::Idle => break,
            Step::Committed => {}
            Step::Failed => failures += 1,
            Step::Retried | Step::Deferred => ready(&f.pool, failed_full.id).await,
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(failures > 0);
    let failed_job: (Uuid, String) = sqlx::query_as("SELECT id,input->>'revision' FROM queue.jobs WHERE scope_id=$1 AND kind='artwork' AND state='failed' ORDER BY id LIMIT 1")
        .bind(failed_full.id).fetch_one(&f.pool).await.unwrap();
    let failed = f.policy("repair", true, false, "failed").await;
    let failed_run = f.start(&failed).await;
    assert_eq!(
        selected_items(&f, failed_run.id)
            .await
            .iter()
            .map(|v| v.0.as_str())
            .collect::<Vec<_>>(),
        ["movie0"]
    );
    ingest.cancel_run(failed_run.id).await.unwrap();

    http.state.tag.store(1, Ordering::SeqCst);
    http.state.image.store(1, Ordering::SeqCst);
    http.state.storage_status.store(0, Ordering::SeqCst);
    let replacement = f.start(&full).await;
    drain(&w).await;
    assert!(
        RepairRepository::new(f.pool.clone())
            .run_status(replacement.id)
            .await
            .unwrap()
            .work_succeeded
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM queue.jobs WHERE id=$1")
            .bind(failed_job.0)
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        "failed"
    );
    assert!(!sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT FROM media.artwork a JOIN media.item_artwork ia ON ia.artwork_id=a.id JOIN catalog.source_items si ON si.id=ia.source_item_id WHERE si.external_id='movie0' AND a.current_generation::text=$1)")
        .bind(failed_job.1).fetch_one(&f.pool).await.unwrap());
    let superseded = f.start(&failed).await;
    assert!(selected_items(&f, superseded.id).await.is_empty());
    drain(&w).await;
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires task-owned SCREENROOM_W_TEST_DATABASE_URL"]
async fn metadata_and_artwork_run_without_embedding_configuration_and_pin() {
    let http = Http::new().await;
    let f = Fixture::new(&http).await;
    let w = http.worker(f.pool.clone(), false, "unused").await;
    assert!(
        !w.capabilities()
            .await
            .unwrap()
            .iter()
            .any(|c| c.starts_with("model:"))
    );
    assert!(
        !w.capabilities()
            .await
            .unwrap()
            .iter()
            .any(|c| c == "adapter:mock")
    );
    let p = f.policy("full", true, false, "all").await;
    let run = f.start(&p).await;
    assert!(run.embedding_model_id.is_none());
    drain(&w).await;
    let status = RepairRepository::new(f.pool.clone())
        .run_status(run.id)
        .await
        .unwrap();
    assert!(status.work_succeeded);
    assert_eq!(status.run.items, 6);
    assert_eq!(status.observed_records, Some(6));
    assert_eq!(http.state.embedding_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM search.embedding_models")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM media.artwork_generations WHERE state='published'"
        )
        .fetch_one(&f.pool)
        .await
        .unwrap(),
        6
    );
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires task-owned SCREENROOM_W_TEST_DATABASE_URL"]
async fn capability_routing_keeps_incompatible_attempts_unconsumed() {
    let http = Http::new().await;
    let f = Fixture::new(&http).await;
    let configured = http.worker(f.pool.clone(), true, "r1").await;
    let p = f.policy("full", false, true, "all").await;
    let run = f.start(&p).await;
    let metadata = http.worker(f.pool.clone(), false, "unused").await;
    drain(&metadata).await;
    let pending:Vec<(Uuid,i32)>=sqlx::query_as("SELECT id,attempts FROM queue.jobs WHERE scope_id=$1 AND kind='embedding' AND state='ready'").bind(run.id).fetch_all(&f.pool).await.unwrap();
    assert_eq!(pending.len(), 6);
    assert!(pending.iter().all(|(_, a)| *a == 0));
    let other = http.worker(f.pool.clone(), true, "r2").await;
    assert_eq!(other.step().await.unwrap(), Step::Idle);
    let attempts:i64=sqlx::query_scalar("SELECT COALESCE(sum(attempts),0)::bigint FROM queue.jobs WHERE scope_id=$1 AND kind='embedding'").bind(run.id).fetch_one(&f.pool).await.unwrap();
    assert_eq!(attempts, 0);
    drain(&configured).await;
    assert_eq!(http.state.embedding_calls.load(Ordering::SeqCst), 6);
    assert!(
        RepairRepository::new(f.pool.clone())
            .run_status(run.id)
            .await
            .unwrap()
            .work_succeeded
    );
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires task-owned SCREENROOM_W_TEST_DATABASE_URL"]
async fn canonical_delta_restart_overlap_fallback_and_settlement() {
    let http = Http::new().await;
    let f = Fixture::new(&http).await;
    let w = http.worker(f.pool.clone(), false, "unused").await;
    let p = f.policy("incremental", false, false, "all").await;
    let repo = IndexingRepository::new(f.pool.clone());
    let ingest = IngestRepository::new(f.pool.clone());
    let baseline = f.start(&p).await;
    assert_eq!(
        repo.execution(baseline.id).await.unwrap().unwrap().mode,
        "reconcile"
    );
    drain(&w).await;
    let watermark = repo
        .progress(p.id)
        .await
        .unwrap()
        .unwrap()
        .watermark
        .unwrap();
    let delta = f.start(&p).await;
    assert_eq!(
        repo.execution(delta.id)
            .await
            .unwrap()
            .unwrap()
            .lower_watermark,
        Some(watermark - screenroom_backend::chrono::Duration::seconds(120))
    );
    assert_eq!(w.step().await.unwrap(), Step::Committed);
    let crash = ingest
        .claim_canonical("crashed", 60, &w.capabilities().await.unwrap())
        .await
        .unwrap()
        .unwrap();
    ingest
        .reserve_work_request(&crash, OperationClass::Discovery)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE queue.jobs SET lease_until=clock_timestamp()-interval '1 second' WHERE id=$1",
    )
    .bind(crash.id)
    .execute(&f.pool)
    .await
    .unwrap();
    http.state.provider_status.store(503, Ordering::SeqCst);
    assert_eq!(w.step().await.unwrap(), Step::Deferred);
    assert!(
        repo.execution(delta.id)
            .await
            .unwrap()
            .unwrap()
            .settled_at
            .is_none()
    );
    assert!(
        ingest
            .run_coverage(delta.id)
            .await
            .unwrap()
            .unresolved_attempts
            > 0
    );
    http.state.provider_status.store(0, Ordering::SeqCst);
    ready(&f.pool, delta.id).await;
    assert_eq!(w.step().await.unwrap(), Step::Committed);
    let checkpoint = ingest
        .run_checkpoint(&delta, "full-index-v1")
        .await
        .unwrap()
        .unwrap();
    assert!(checkpoint["delta"].is_object());
    let restarted = http.worker(f.pool.clone(), false, "unused").await;
    drain(&restarted).await;
    let coverage = ingest.run_coverage(delta.id).await.unwrap();
    assert!(coverage.unknown_attempts > 0);
    assert_eq!(coverage.unresolved_attempts, 0);
    assert_eq!(
        repo.progress(p.id).await.unwrap().unwrap().watermark,
        Some(delta.started_at)
    );
    assert_eq!(ingest.run(delta.id).await.unwrap().unwrap().items, 0);
    http.state.delta_reject.store(1, Ordering::SeqCst);
    let fallback = f.start(&p).await;
    drain(&restarted).await;
    let execution = repo.execution(fallback.id).await.unwrap().unwrap();
    assert_eq!(
        execution.fallback_reason.as_deref(),
        Some("filter_rejected")
    );
    assert!(execution.settled_at.is_some());
    http.state.delta_reject.store(0, Ordering::SeqCst);
    sqlx::query("UPDATE ingest.indexing_progress SET next_reconcile_at=clock_timestamp()-interval '1 second' WHERE policy_id=$1").bind(p.id).execute(&f.pool).await.unwrap();
    let reconcile = f.start(&p).await;
    assert_eq!(
        repo.execution(reconcile.id).await.unwrap().unwrap().mode,
        "reconcile"
    );
    drain(&restarted).await;
    assert!(
        repo.progress(p.id)
            .await
            .unwrap()
            .unwrap()
            .next_reconcile_at
            > reconcile.started_at
    );
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires task-owned SCREENROOM_W_TEST_DATABASE_URL"]
async fn canonical_retry_classification_receipts_redrive_and_cancel_fences() {
    for status in [401, 429, 503] {
        let http = Http::new().await;
        let f = Fixture::new(&http).await;
        let w = http.worker(f.pool.clone(), false, "unused").await;
        let p = f.policy("full", false, false, "all").await;
        let run = f.start(&p).await;
        http.state.provider_status.store(status, Ordering::SeqCst);
        assert_eq!(
            w.step().await.unwrap(),
            if status == 401 {
                Step::Failed
            } else {
                Step::Deferred
            }
        );
        let ingest = IngestRepository::new(f.pool.clone());
        let coverage = ingest.run_coverage(run.id).await.unwrap();
        if status == 401 {
            assert!(coverage.unresolved_attempts > 0);
            assert!(coverage.permanent_attempts > 0);
        } else {
            assert!(coverage.retryable_attempts > 0);
        }
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT count(*) FROM ingest.request_receipts WHERE run_id=$1 AND outcome='success'"
            )
            .bind(run.id)
            .fetch_one(&f.pool)
            .await
            .unwrap(),
            0
        );
        let (job, attempts, state): (Uuid, i32, String) = sqlx::query_as(
            "SELECT id,attempts,state FROM queue.jobs WHERE scope_id=$1 AND kind='ingest'",
        )
        .bind(run.id)
        .fetch_one(&f.pool)
        .await
        .unwrap();
        assert_eq!(attempts, 1);
        assert_eq!(state, if status == 401 { "failed" } else { "ready" });
        http.state.provider_status.store(0, Ordering::SeqCst);
        if status == 401 {
            let request = Uuid::new_v4();
            let first = ingest.redrive_job(job, request).await.unwrap();
            assert_eq!(first.job_id, job);
            assert_eq!(
                ingest.redrive_job(job, request).await.unwrap().generation,
                first.generation
            );
        } else {
            ready(&f.pool, run.id).await;
        }
        let restarted = http.worker(f.pool.clone(), false, "unused").await;
        drain(&restarted).await;
        assert_eq!(
            ingest
                .run_coverage(run.id)
                .await
                .unwrap()
                .unresolved_attempts,
            0
        );
        let next = f.start(&p).await;
        let lease = ingest
            .claim_canonical("late", 60, &w.capabilities().await.unwrap())
            .await
            .unwrap()
            .unwrap();
        ingest.cancel_run(next.id).await.unwrap();
        assert!(matches!(
            ingest.renew(&lease, 60).await,
            Err(screenroom_backend::Error::Stale)
        ));
        let late_page = screenroom_backend::ingest::PageCommit {
            libraries: vec![],
            items: vec![],
            artwork_inputs: vec![],
            artwork_omissions: vec![],
            followups: vec![],
            checkpoint: screenroom_backend::ingest::CheckpointWrite {
                scope_key: "full-index-v1".into(),
                cursor: json!({"late":true}),
                watermark: None,
            },
            requests: 0,
            completion: screenroom_backend::ingest::Completion::Continue,
        };
        assert!(matches!(
            ingest
                .commit_traversal_page(&lease, &late_page, &Default::default())
                .await,
            Err(screenroom_backend::Error::Stale)
        ));
        let before = http.state.calls.load(Ordering::SeqCst);
        assert_eq!(w.step().await.unwrap(), Step::Idle);
        assert_eq!(before, http.state.calls.load(Ordering::SeqCst));
        f.cleanup().await;
    }
}

#[tokio::test]
#[ignore = "requires task-owned SCREENROOM_W_TEST_DATABASE_URL"]
async fn real_artwork_a_b_a_preserves_previous_complete_and_storage_dedupe() {
    let http = Http::new().await;
    let f = Fixture::new(&http).await;
    http.state.count.store(1, Ordering::SeqCst);
    let w = http.worker(f.pool.clone(), false, "unused").await;
    let p = f.policy("full", true, false, "all").await;
    f.start(&p).await;
    drain(&w).await;
    let first: (i64, i64) =
        sqlx::query_as("SELECT revision,current_generation FROM media.artwork LIMIT 1")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(first.0, first.1);
    http.state.tag.store(1, Ordering::SeqCst);
    http.state.image.store(1, Ordering::SeqCst);
    let run = f.start(&p).await;
    for _ in 0..100 {
        let pending:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM queue.jobs WHERE scope_id=$1 AND kind='artwork' AND state='ready')").bind(run.id).fetch_one(&f.pool).await.unwrap();
        if pending {
            break;
        }
        assert_eq!(w.step().await.unwrap(), Step::Committed);
    }
    let before: (i64, i64) =
        sqlx::query_as("SELECT revision,current_generation FROM media.artwork LIMIT 1")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(before.0, first.0);
    assert!(before.1 > first.1);
    http.state.storage_status.store(503, Ordering::SeqCst);
    assert_eq!(w.step().await.unwrap(), Step::Retried);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT revision FROM media.artwork LIMIT 1")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        first.0
    );
    http.state.storage_status.store(0, Ordering::SeqCst);
    ready(&f.pool, run.id).await;
    drain(&w).await;
    let second: i64 = sqlx::query_scalar("SELECT revision FROM media.artwork LIMIT 1")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert!(second > first.0);
    http.state.tag.store(0, Ordering::SeqCst);
    http.state.image.store(0, Ordering::SeqCst);
    f.start(&p).await;
    let restarted = http.worker(f.pool.clone(), false, "unused").await;
    drain(&restarted).await;
    let third: i64 = sqlx::query_scalar("SELECT revision FROM media.artwork LIMIT 1")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert!(third > second);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM media.artwork_generations WHERE state='published'"
        )
        .fetch_one(&f.pool)
        .await
        .unwrap(),
        3
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM media.objects")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        2
    );
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires task-owned SCREENROOM_W_TEST_DATABASE_URL"]
async fn canonical_poster_repair_candidates_restart_and_no_metadata_mutation() {
    let http = Http::new().await;
    let f = Fixture::new(&http).await;
    let w = http.worker(f.pool.clone(), false, "unused").await;
    let full = f.policy("full", false, false, "all").await;
    f.start(&full).await;
    drain(&w).await;
    let metadata: Vec<(Uuid, serde_json::Value)> =
        sqlx::query_as("SELECT id,metadata FROM catalog.source_items ORDER BY id")
            .fetch_all(&f.pool)
            .await
            .unwrap();
    let repair = f.policy("repair", true, false, "missing").await;
    let run = f.start(&repair).await;
    assert_eq!(run.selection_snapshot["repair"], 1);
    assert!(run.selection_snapshot.get("maintenance").is_none());
    assert!(run.embedding_model_id.is_none());
    let repo = RepairRepository::new(f.pool.clone());
    assert_eq!(
        repo.candidates(run.id, None, 100, true)
            .await
            .unwrap()
            .len(),
        6
    );
    assert_eq!(w.step().await.unwrap(), Step::Committed);
    let restarted = http.worker(f.pool.clone(), false, "unused").await;
    drain(&restarted).await;
    assert!(repo.run_status(run.id).await.unwrap().work_succeeded);
    assert_eq!(
        metadata,
        sqlx::query_as::<_, (Uuid, serde_json::Value)>(
            "SELECT id,metadata FROM catalog.source_items ORDER BY id"
        )
        .fetch_all(&f.pool)
        .await
        .unwrap()
    );
    let second = f.start(&repair).await;
    assert!(
        repo.candidates(second.id, None, 100, true)
            .await
            .unwrap()
            .is_empty()
    );
    drain(&restarted).await;
    let all = f.policy("repair", true, false, "all").await;
    let refresh = f.start(&all).await;
    assert_eq!(
        repo.candidates(refresh.id, None, 100, true)
            .await
            .unwrap()
            .len(),
        6
    );
    drain(&restarted).await;
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires task-owned SCREENROOM_W_TEST_DATABASE_URL"]
async fn hierarchy_and_repeated_pages_never_falsely_complete() {
    for fault in [0, 1, 2] {
        let http = Http::new().await;
        let f = Fixture::new(&http).await;
        let w = http.worker(f.pool.clone(), false, "unused").await;
        let mut config:Configuration=serde_json::from_value(json!({"indexing":1,"enabled":true,"mode":"full","cadence_seconds":60,"components":{"metadata":true,"artwork":false,"embeddings":false},"strategy":"all","scope":{"library_ids":[Fixture::library("shows")],"item_kinds":["movie","series"]},"operator_profile_id":"profile","limits":{"work_requests":100,"work_records":3,"lifetime_requests":10000,"lifetime_records":1000,"deadline_seconds":3600,"work_timeout_seconds":30}})).unwrap();
        if fault == 1 {
            http.state.missing_parent.store(1, Ordering::SeqCst);
        }
        if fault == 2 {
            http.state.repeated_page.store(1, Ordering::SeqCst);
            http.state.count.store(12, Ordering::SeqCst);
            config.scope.library_ids = vec![Fixture::library("movies")];
        }
        let policy = IndexingRepository::new(f.pool.clone())
            .configure(
                f.source,
                &ConfigureRequest {
                    request_id: Uuid::new_v4(),
                    existing: None,
                    configuration: config,
                },
            )
            .await
            .unwrap();
        let run = f.start(&policy).await;
        for _ in 0..100 {
            match w.step().await.unwrap() {
                Step::Committed => {}
                Step::Failed | Step::Idle => break,
                step => panic!("{step:?}"),
            }
        }
        let status = RepairRepository::new(f.pool.clone())
            .run_status(run.id)
            .await
            .unwrap();
        if fault == 0 {
            assert!(status.work_succeeded);
            assert_eq!(status.run.items, 8);
        } else {
            assert!(!status.work_succeeded);
        }
        f.cleanup().await;
    }
}

#[tokio::test]
#[ignore = "requires task-owned SCREENROOM_W_TEST_DATABASE_URL"]
async fn full_embedding_errors_are_not_false_successes() {
    let http = Http::new().await;
    let f = Fixture::new(&http).await;
    let configured = http.worker(f.pool.clone(), true, "r1").await;
    let p = f.policy("full", false, true, "all").await;
    let run = f.start(&p).await;
    drain(&http.worker(f.pool.clone(), false, "unused").await).await;
    http.state.embedding_status.store(503, Ordering::SeqCst);
    assert_eq!(configured.step().await.unwrap(), Step::Deferred);
    assert!(
        IngestRepository::new(f.pool.clone())
            .run_coverage(run.id)
            .await
            .unwrap()
            .retryable_attempts
            > 0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM search.embeddings")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        0
    );
    http.state.embedding_status.store(0, Ordering::SeqCst);
    ready(&f.pool, run.id).await;
    drain(&configured).await;
    assert!(
        RepairRepository::new(f.pool.clone())
            .run_status(run.id)
            .await
            .unwrap()
            .work_succeeded
    );
    f.cleanup().await;
}

#[tokio::test]
#[ignore = "requires task-owned SCREENROOM_W_TEST_DATABASE_URL; large generated traversal"]
async fn full_21021_generated_records_resume_bounded_checkpoint() {
    let http = Http::new().await;
    http.state.count.store(21021, Ordering::SeqCst);
    let f = Fixture::new(&http).await;
    let p = f.policy("full", false, false, "all").await;
    let run = f.start(&p).await;
    let first = http.worker(f.pool.clone(), false, "unused").await;
    for _ in 0..5 {
        assert_eq!(first.step().await.unwrap(), Step::Committed);
    }
    let restarted = http.worker(f.pool.clone(), false, "unused").await;
    drain(&restarted).await;
    let status = RepairRepository::new(f.pool.clone())
        .run_status(run.id)
        .await
        .unwrap();
    assert!(status.work_succeeded);
    assert_eq!(status.run.items, 21021);
    let checkpoint = IngestRepository::new(f.pool.clone())
        .run_checkpoint(&run, "full-index-v1")
        .await
        .unwrap()
        .unwrap();
    assert!(checkpoint.to_string().len() < 32768);
    f.cleanup().await;
}

#[test]
fn retired_cli_commands_fail_before_configuration_or_database() {
    for command in [
        "sample",
        "index",
        "index-status",
        "index-stop",
        "index-resume",
        "resume",
    ] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_screenroom-index-worker"))
            .arg(command)
            .env_clear()
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("password"));
    }
}
