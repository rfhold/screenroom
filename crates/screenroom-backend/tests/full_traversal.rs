use screenroom_backend::{Error, media::*, search::SearchRepository, storage::*};
use screenroom_backend::{Uuid, catalog::*, db::MIGRATOR, ingest::*, sqlx};
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, postgres::PgPoolOptions};
mod support;
use support::{CatalogRepository, IngestRepository, PolicyWrite};
#[path = "support/image.rs"]
mod neutral_image;

static ROLE_BOOTSTRAP: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn migrate(pool: &PgPool) {
    // Fresh databases share cluster-global roles created by the first migration.
    let _bootstrap = ROLE_BOOTSTRAP.lock().await;
    MIGRATOR.run(pool).await.unwrap();
    SearchRepository::new(pool.clone())
        .register_model("test-manual", "1", 1024)
        .await
        .unwrap();
}

async fn fixture_object(source: Uuid) -> VerifiedObject {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let bytes = neutral_image::png([64, 96, 128]);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let body = bytes.clone();
    let server = tokio::spawn(async move {
        for method in ["PUT", "HEAD", "GET"] {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let end = loop {
                let mut buffer = [0; 4096];
                let n = socket.read(&mut buffer).await.unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buffer[..n]);
                if let Some(end) = request.windows(4).position(|v| v == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let headers = String::from_utf8(request[..end].to_vec()).unwrap();
            assert!(headers.starts_with(method));
            let length = headers
                .lines()
                .find_map(|s| {
                    s.to_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|s| s.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            while request.len() - end < length {
                let mut buffer = [0; 4096];
                let n = socket.read(&mut buffer).await.unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buffer[..n]);
            }
            let data = if method == "PUT" {
                &[][..]
            } else {
                body.as_slice()
            };
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: image/png\r\nConnection: close\r\n\r\n",data.len()).as_bytes()).await.unwrap();
            if method != "HEAD" {
                socket.write_all(data).await.unwrap();
            }
        }
    });
    let storage = ObjectStorage::new(StorageConfig {
        endpoint,
        region: "local".into(),
        bucket: "synthetic".into(),
        access_key_id: "test".into(),
        secret_access_key: "test".into(),
        timeout: std::time::Duration::from_secs(2),
    })
    .unwrap();
    let object = storage
        .upload_verified(source, &ValidatedImage::raster(bytes).unwrap())
        .await
        .unwrap();
    server.await.unwrap();
    object
}

async fn assert_unknown(pool: &PgPool, receipt: Uuid, claim: &Claim) {
    let outcome: String =
        sqlx::query_scalar("SELECT outcome FROM ingest.request_receipts WHERE id=$1")
            .bind(receipt)
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(outcome, "unknown");
    let state: String = sqlx::query_scalar("SELECT state FROM queue.jobs WHERE id=$1")
        .bind(claim.id)
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(state, "leased");
    assert!(window(pool, claim).await > 0);
}

#[tokio::test]
#[ignore = "requires explicitly configured disposable PostgreSQL 18 + pgvector admin URL"]
async fn postgres_terminal_omission_progress_contract() {
    let url = std::env::var("SCREENROOM_TEST_DATABASE_URL").unwrap();
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .unwrap();
    let name = format!("screenroom_omissions_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    let mut dburl = reqwest::Url::parse(&url).unwrap();
    dburl.set_path(&format!("/{name}"));
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(dburl.as_str())
        .await
        .unwrap();
    migrate(&pool).await;
    let worker = role_pool(&pool, "screenroom_worker").await;
    let reader = role_pool(&pool, "screenroom_catalog").await;
    let catalog = CatalogRepository::new(reader.clone());
    let operator = IngestRepository::new(reader.clone());
    let ingest = IngestRepository::new(worker.clone());
    let source = catalog
        .configure_source("omissions", "jellyfin", &json!({}), &json!({}))
        .await
        .unwrap();
    catalog
        .configure_library(source.id, "a", "a", "A")
        .await
        .unwrap();
    let policy=operator.create_policy(source.id,&PolicyWrite {
        enabled:true,selection_json:json!({"library_ids":["a"],"work_budget":{"mode":"per_work_commit","max_requests":7,"max_records":10}}),
        max_items:1000,max_requests:1000,incremental_seconds:60,reconcile_seconds:3600,enrich_metadata:true,enrich_people:false,enrich_artwork:true,
    }).await.unwrap();
    let run = operator.start_run(policy.id).await.unwrap();
    let claim = ingest.claim("seed", 60).await.unwrap().unwrap();
    let mut seed = page(vec![], 0, Completion::Continue);
    seed.followups.push(Followup {
        kind: JobKind::Embedding,
        dedupe_key: "isolated-window".into(),
        input: json!({"external_id":"isolated-window"}),
    });
    ingest
        .commit_traversal_page(&claim, &seed, &TraversalWrites::default())
        .await
        .unwrap();
    sqlx::query("UPDATE queue.jobs SET available_at=now()+interval '1 hour' WHERE id=$1")
        .bind(claim.id)
        .execute(&pool)
        .await
        .unwrap();
    let followup = ingest.claim("other-job", 60).await.unwrap().unwrap();
    ingest
        .reserve_work_request(&followup, OperationClass::Embedding)
        .await
        .unwrap();
    sqlx::query("UPDATE queue.jobs SET available_at=now() WHERE id=$1")
        .bind(claim.id)
        .execute(&pool)
        .await
        .unwrap();
    for index in 0..8 {
        let claim = ingest.claim("missing-image", 60).await.unwrap().unwrap();
        let receipt = ingest
            .reserve_work_request(&claim, OperationClass::Artwork)
            .await
            .unwrap();
        let outcome = RequestOutcomeWrite {
            receipt_id: receipt.id,
            outcome: RequestOutcome::Permanent,
            error_class: Some(RequestErrorClass::Invalid),
        };
        let mut commit = page(vec![], index + 1, Completion::Continue);
        commit.artwork_omissions.push(ArtworkOmission {
            source_artwork_id: format!("missing-{index}:revision-1"),
            code: ArtworkOmissionCode::Missing,
        });
        if index == 0 {
            let mut stale = claim.clone();
            stale.fence += 1;
            assert!(
                ingest
                    .commit_traversal_page(
                        &stale,
                        &commit,
                        &TraversalWrites {
                            outcomes: vec![outcome.clone()],
                            ..Default::default()
                        }
                    )
                    .await
                    .is_err()
            );
            let mut missing = outcome.clone();
            missing.receipt_id = Uuid::new_v4();
            let mut malformed = outcome.clone();
            malformed.error_class = None;
            for invalid in [missing, malformed, outcome.clone()] {
                assert!(
                    ingest
                        .commit_traversal_page(
                            &claim,
                            &commit,
                            &TraversalWrites {
                                outcomes: vec![outcome.clone(), invalid],
                                ..Default::default()
                            }
                        )
                        .await
                        .is_err()
                );
                assert_unknown(&pool, outcome.receipt_id, &claim).await;
                assert_eq!(ingest.run_coverage(run.id).await.unwrap().omissions, 0);
            }
        }
        ingest
            .commit_traversal_page(
                &claim,
                &commit,
                &TraversalWrites {
                    outcomes: vec![outcome],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(window(&pool, &claim).await, 0);
        let attempts: i32 = sqlx::query_scalar("SELECT attempts FROM queue.jobs WHERE id=$1")
            .bind(claim.id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(attempts, 0);
        assert_eq!(window(&pool, &followup).await, 1);
    }
    let claim = ingest.claim("finish", 60).await.unwrap().unwrap();
    ingest
        .commit_traversal_page(
            &claim,
            &page(
                vec![],
                9,
                Completion::Complete {
                    evidence: json!({"non_authoritative":true}),
                },
            ),
            &TraversalWrites::default(),
        )
        .await
        .unwrap();
    let coverage = ingest.run_coverage(run.id).await.unwrap();
    assert_eq!(coverage.omissions, 8);
    assert_eq!(coverage.permanent_attempts, 8);
    assert_eq!(ingest.run(run.id).await.unwrap().unwrap().state, "complete");
    ingest.complete_job(&followup).await.unwrap();
    // A new run may observe the same identity once, but replays within it are no progress.
    let repeated = operator.start_run(policy.id).await.unwrap();
    for index in 0..2 {
        let claim = ingest.claim("replay", 60).await.unwrap().unwrap();
        if index == 1 {
            for _ in 0..6 {
                let receipt = ingest
                    .reserve_work_request(&claim, OperationClass::Artwork)
                    .await
                    .unwrap();
                ingest
                    .record_work_outcome(
                        &claim,
                        &RequestOutcomeWrite {
                            receipt_id: receipt.id,
                            outcome: RequestOutcome::Permanent,
                            error_class: Some(RequestErrorClass::Invalid),
                        },
                    )
                    .await
                    .unwrap();
            }
        }
        let receipt = ingest
            .reserve_work_request(&claim, OperationClass::Artwork)
            .await
            .unwrap();
        let mut commit = page(vec![], index + 1, Completion::Continue);
        commit.artwork_omissions.push(ArtworkOmission {
            source_artwork_id: "repeated:revision-1".into(),
            code: if index == 0 {
                ArtworkOmissionCode::Missing
            } else {
                ArtworkOmissionCode::Unavailable
            },
        });
        ingest
            .commit_traversal_page(
                &claim,
                &commit,
                &TraversalWrites {
                    outcomes: vec![RequestOutcomeWrite {
                        receipt_id: receipt.id,
                        outcome: RequestOutcome::Permanent,
                        error_class: Some(RequestErrorClass::Invalid),
                    }],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(window(&pool, &claim).await, index as i64 * 7);
    }
    for index in 2..6 {
        let claim = ingest.claim("cursor-only", 60).await.unwrap().unwrap();
        assert!(
            ingest
                .reserve_work_request(&claim, OperationClass::Artwork)
                .await
                .is_err()
        );
        ingest
            .commit_traversal_page(
                &claim,
                &page(
                    vec![],
                    index + 1,
                    if index == 2 {
                        Completion::Pause
                    } else {
                        Completion::Continue
                    },
                ),
                &TraversalWrites::default(),
            )
            .await
            .unwrap();
        assert_eq!(window(&pool, &claim).await, 7);
        if index == 2 {
            operator.resume(repeated.id).await.unwrap();
        }
    }
    assert!(ingest.claim("exhausted", 60).await.unwrap().is_none());
    assert_eq!(
        ingest.run(repeated.id).await.unwrap().unwrap().state,
        "failed"
    );
    assert_eq!(ingest.run_coverage(repeated.id).await.unwrap().omissions, 1);
    worker.close().await;
    reader.close().await;
    pool.close().await;
    sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}

#[tokio::test]
#[ignore = "requires explicitly configured disposable PostgreSQL 18 + pgvector admin URL"]
async fn postgres_atomic_followup_outcomes_contract() {
    let url = std::env::var("SCREENROOM_TEST_DATABASE_URL").unwrap();
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .unwrap();
    let name = format!("screenroom_outcomes_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    let mut dburl = reqwest::Url::parse(&url).unwrap();
    dburl.set_path(&format!("/{name}"));
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(dburl.as_str())
        .await
        .unwrap();
    migrate(&pool).await;
    let worker = role_pool(&pool, "screenroom_worker").await;
    let reader = role_pool(&pool, "screenroom_catalog").await;
    let catalog = CatalogRepository::new(reader.clone());
    let operator = IngestRepository::new(reader.clone());
    let ingest = IngestRepository::new(worker.clone());
    let media = MediaRepository::new(worker.clone());
    let search = SearchRepository::new(worker.clone());
    let source = catalog
        .configure_source("outcomes", "jellyfin", &json!({}), &json!({}))
        .await
        .unwrap();
    catalog
        .configure_library(source.id, "a", "a", "A")
        .await
        .unwrap();
    let policy=operator.create_policy(source.id,&PolicyWrite {
        enabled:true,selection_json:json!({"library_ids":["a"],"work_budget":{"mode":"per_work_commit","max_requests":10,"max_records":10}}),
        max_items:1000,max_requests:1000,incremental_seconds:60,reconcile_seconds:3600,enrich_metadata:true,enrich_people:false,enrich_artwork:true,
    }).await.unwrap();
    let run = operator.start_run(policy.id).await.unwrap();
    let claim = ingest.claim("seed", 60).await.unwrap().unwrap();
    let mut commit = page(
        vec![item("atomic", "movie", None)],
        1,
        Completion::Complete {
            evidence: json!({}),
        },
    );
    let bytes = neutral_image::png([64, 96, 128]);
    commit.artwork_inputs.push(ArtworkInputWrite {
        reference: ArtworkReferenceInput {
            source_artwork_id: "atomic-art".into(),
            kind: "poster".into(),
            rights: json!({}),
            item_external_id: Some("atomic".into()),
            person_external_id: None,
            role: "poster".into(),
            ordinal: 0,
        },
        manifest: ArtworkInputManifest {
            version: 1,
            variants: vec![ArtworkVariantInput {
                variant: ArtworkVariant::Native,
                sha256: Sha256::digest(bytes).into(),
            }],
        },
    });
    commit.followups.push(Followup {
        kind: JobKind::Embedding,
        dedupe_key: "atomic-vector".into(),
        input: json!({"external_id":"atomic"}),
    });
    ingest
        .commit_traversal_page(&claim, &commit, &TraversalWrites::default())
        .await
        .unwrap();
    let a = ingest.claim("followup-a", 60).await.unwrap().unwrap();
    let b = ingest.claim("followup-b", 60).await.unwrap().unwrap();
    let (mut art, embedding) = if a.kind == "artwork" { (a, b) } else { (b, a) };
    assert_eq!(
        ingest
            .run_coverage(run.id)
            .await
            .unwrap()
            .pending_artwork_publications,
        1
    );
    sqlx::query("UPDATE queue.jobs SET state='ready',lease_owner=NULL,lease_until=NULL,available_at=now()+interval '1 hour' WHERE id=$1")
        .bind(art.id).execute(&pool).await.unwrap();
    let dependent = operator.start_run(policy.id).await.unwrap();
    let claim = ingest
        .claim("dependent-staging", 60)
        .await
        .unwrap()
        .unwrap();
    let mut identical = page(
        vec![],
        1,
        Completion::Complete {
            evidence: json!({}),
        },
    );
    identical.artwork_inputs = commit.artwork_inputs.clone();
    ingest
        .commit_traversal_page(&claim, &identical, &TraversalWrites::default())
        .await
        .unwrap();
    let dependent_coverage = ingest.run_coverage(dependent.id).await.unwrap();
    assert_eq!(
        dependent_coverage.followup_ready
            + dependent_coverage.followup_delayed
            + dependent_coverage.followup_leased,
        1
    );
    assert_eq!(dependent_coverage.pending_artwork_publications, 1);
    let restarted_pool = role_pool(&pool, "screenroom_worker").await;
    assert_eq!(
        IngestRepository::new(restarted_pool.clone())
            .run_coverage(dependent.id)
            .await
            .unwrap()
            .pending_artwork_publications,
        1
    );
    restarted_pool.close().await;
    sqlx::query("UPDATE queue.jobs SET state='failed' WHERE id=$1")
        .bind(art.id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        ingest
            .run_coverage(dependent.id)
            .await
            .unwrap()
            .pending_artwork_publications,
        1
    );
    sqlx::query("UPDATE queue.jobs SET state='ready',available_at=now() WHERE id=$1")
        .bind(art.id)
        .execute(&pool)
        .await
        .unwrap();
    // The dependent run retains its own transport identity even while the first owner fails.
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM queue.jobs WHERE scope_id=$1 AND kind='artwork' AND state='ready'"
        )
        .bind(dependent.id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
    sqlx::query("UPDATE queue.jobs SET available_at=now()+interval '1 hour' WHERE scope_id=$1 AND kind='artwork'").bind(dependent.id).execute(&pool).await.unwrap();
    art = ingest.claim("original-owner", 60).await.unwrap().unwrap();
    assert_eq!(art.run_id, Some(run.id));
    assert_eq!(
        ingest
            .run_coverage(dependent.id)
            .await
            .unwrap()
            .pending_artwork_publications,
        1
    );
    let art_outcome = success(
        ingest
            .reserve_work_request(&art, OperationClass::Artwork)
            .await
            .unwrap(),
    );
    let second_art = success(
        ingest
            .reserve_work_request(&art, OperationClass::Artwork)
            .await
            .unwrap(),
    );
    let embed_outcome = success(
        ingest
            .reserve_work_request(&embedding, OperationClass::Embedding)
            .await
            .unwrap(),
    );
    let object = fixture_object(source.id).await;
    let model = search
        .register_model("atomic-model", "r1", 1024)
        .await
        .unwrap();
    let document = search
        .document_for_source_item(
            catalog
                .get(&Scope::default(), "atomic")
                .await
                .unwrap()
                .unwrap()
                .source_item_id,
        )
        .await
        .unwrap()
        .unwrap();
    let mut vector = vec![0.; 1024];
    vector[0] = 1.;
    let other = operator.start_run(policy.id).await.unwrap();
    operator.cancel_run(other.id).await.unwrap();
    let mut stale = art.clone();
    stale.fence += 1;
    assert!(
        media
            .publish_with_outcomes(&stale, &object, std::slice::from_ref(&art_outcome))
            .await
            .is_err()
    );
    let mut wrong_input = art.input.clone();
    wrong_input["expected_sha256"] = json!(vec![9_u8; 32]);
    sqlx::query("UPDATE queue.jobs SET input=$2 WHERE id=$1")
        .bind(art.id)
        .bind(wrong_input)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        media
            .publish_with_outcomes(&art, &object, std::slice::from_ref(&art_outcome))
            .await
            .is_err()
    );
    assert_unknown(&pool, art_outcome.receipt_id, &art).await;
    sqlx::query("UPDATE queue.jobs SET input=$2 WHERE id=$1")
        .bind(art.id)
        .bind(&art.input)
        .execute(&pool)
        .await
        .unwrap();
    let mut stale = embedding.clone();
    stale.fence += 1;
    assert!(
        search
            .commit_embedding_with_outcomes(
                &stale,
                document.id,
                model.id,
                &model.revision,
                &document.content_hash,
                &vector,
                std::slice::from_ref(&embed_outcome)
            )
            .await
            .is_err()
    );
    assert!(
        search
            .commit_embedding_with_outcomes(
                &embedding,
                document.id,
                model.id,
                &model.revision,
                &[9; 32],
                &vector,
                std::slice::from_ref(&embed_outcome)
            )
            .await
            .is_err()
    );
    assert!(
        search
            .commit_embedding_with_outcomes(
                &embedding,
                document.id,
                model.id,
                "wrong-revision",
                &document.content_hash,
                &vector,
                std::slice::from_ref(&embed_outcome)
            )
            .await
            .is_err()
    );
    // Each failure occurs after at least one valid receipt in the supplied batch.
    let mut missing = art_outcome.clone();
    missing.receipt_id = Uuid::new_v4();
    let mut bad_error = art_outcome.clone();
    bad_error.error_class = Some(RequestErrorClass::Invalid);
    for invalid in [
        missing,
        bad_error,
        art_outcome.clone(),
        embed_outcome.clone(),
    ] {
        assert!(
            media
                .publish_with_outcomes(&art, &object, &[art_outcome.clone(), invalid])
                .await
                .is_err()
        );
        assert_unknown(&pool, art_outcome.receipt_id, &art).await;
    }
    let mut missing = embed_outcome.clone();
    missing.receipt_id = Uuid::new_v4();
    let mut bad_error = embed_outcome.clone();
    bad_error.outcome = RequestOutcome::Retryable;
    for invalid in [
        missing,
        bad_error,
        embed_outcome.clone(),
        art_outcome.clone(),
    ] {
        assert!(
            search
                .commit_embedding_with_outcomes(
                    &embedding,
                    document.id,
                    model.id,
                    &model.revision,
                    &document.content_hash,
                    &vector,
                    &[embed_outcome.clone(), invalid]
                )
                .await
                .is_err()
        );
        assert_unknown(&pool, embed_outcome.receipt_id, &embedding).await;
    }
    for (receipt, job) in [
        (art_outcome.receipt_id, &art),
        (embed_outcome.receipt_id, &embedding),
    ] {
        sqlx::query("UPDATE ingest.request_receipts SET run_id=$2 WHERE id=$1")
            .bind(receipt)
            .bind(other.id)
            .execute(&pool)
            .await
            .unwrap();
        let invalid = if job.kind == "artwork" {
            media
                .publish_with_outcomes(job, &object, std::slice::from_ref(&art_outcome))
                .await
                .map(|_| ())
        } else {
            search
                .commit_embedding_with_outcomes(
                    job,
                    document.id,
                    model.id,
                    &model.revision,
                    &document.content_hash,
                    &vector,
                    std::slice::from_ref(&embed_outcome),
                )
                .await
        };
        assert!(invalid.is_err());
        assert_unknown(&pool, receipt, job).await;
        sqlx::query("UPDATE ingest.request_receipts SET run_id=$2,fence=fence+1 WHERE id=$1")
            .bind(receipt)
            .bind(run.id)
            .execute(&pool)
            .await
            .unwrap();
        let invalid = if job.kind == "artwork" {
            media
                .publish_with_outcomes(job, &object, std::slice::from_ref(&art_outcome))
                .await
                .map(|_| ())
        } else {
            search
                .commit_embedding_with_outcomes(
                    job,
                    document.id,
                    model.id,
                    &model.revision,
                    &document.content_hash,
                    &vector,
                    std::slice::from_ref(&embed_outcome),
                )
                .await
        };
        assert!(invalid.is_err());
        assert_unknown(&pool, receipt, job).await;
        sqlx::query("UPDATE ingest.request_receipts SET fence=fence-1 WHERE id=$1")
            .bind(receipt)
            .execute(&pool)
            .await
            .unwrap();
    }
    let oversized = vec![art_outcome.clone(); 100_001];
    assert!(
        media
            .publish_with_outcomes(&art, &object, &oversized)
            .await
            .is_err()
    );
    assert!(
        search
            .commit_embedding_with_outcomes(
                &embedding,
                document.id,
                model.id,
                &model.revision,
                &document.content_hash,
                &vector,
                &oversized
            )
            .await
            .is_err()
    );
    // Force a failure after receipt writes, proving both outputs and outcomes roll back.
    sqlx::query("CREATE FUNCTION public.reject_completion() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'synthetic finish failure'; END $$").execute(&pool).await.unwrap();
    sqlx::query("CREATE TRIGGER reject_completion BEFORE UPDATE OF state ON queue.jobs FOR EACH ROW WHEN(NEW.state='complete') EXECUTE FUNCTION public.reject_completion() ").execute(&pool).await.unwrap();
    assert!(
        media
            .publish_with_outcomes(&art, &object, &[art_outcome.clone(), second_art.clone()])
            .await
            .is_err()
    );
    assert!(
        search
            .commit_embedding_with_outcomes(
                &embedding,
                document.id,
                model.id,
                &model.revision,
                &document.content_hash,
                &vector,
                std::slice::from_ref(&embed_outcome)
            )
            .await
            .is_err()
    );
    assert_unknown(&pool, art_outcome.receipt_id, &art).await;
    assert_unknown(&pool, second_art.receipt_id, &art).await;
    assert_unknown(&pool, embed_outcome.receipt_id, &embedding).await;
    let outputs:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM media.objects),(SELECT count(*) FROM media.variants),(SELECT count(*) FROM search.embeddings)").fetch_one(&pool).await.unwrap();
    assert_eq!(outputs, (0, 0, 0));
    sqlx::query("DROP TRIGGER reject_completion ON queue.jobs")
        .execute(&pool)
        .await
        .unwrap();
    media
        .publish_with_outcomes(&art, &object, &[art_outcome.clone(), second_art.clone()])
        .await
        .unwrap();
    search
        .commit_embedding_with_outcomes(
            &embedding,
            document.id,
            model.id,
            &model.revision,
            &document.content_hash,
            &vector,
            std::slice::from_ref(&embed_outcome),
        )
        .await
        .unwrap();
    for (receipt, job) in [
        (art_outcome.receipt_id, &art),
        (second_art.receipt_id, &art),
        (embed_outcome.receipt_id, &embedding),
    ] {
        let state:(String,String)=sqlx::query_as("SELECT r.outcome,j.state FROM ingest.request_receipts r JOIN queue.jobs j ON j.id=r.job_id WHERE r.id=$1").bind(receipt).fetch_one(&pool).await.unwrap();
        assert_eq!(state, ("success".into(), "complete".into()));
        assert_eq!(window(&pool, job).await, 0);
    }
    assert!(
        media
            .publish_with_outcomes(&art, &object, std::slice::from_ref(&art_outcome))
            .await
            .is_err()
    );
    assert!(
        search
            .commit_embedding_with_outcomes(
                &embedding,
                document.id,
                model.id,
                &model.revision,
                &document.content_hash,
                &vector,
                std::slice::from_ref(&embed_outcome)
            )
            .await
            .is_err()
    );
    let outputs:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM media.objects),(SELECT count(*) FROM media.variants),(SELECT count(*) FROM search.embeddings)").fetch_one(&pool).await.unwrap();
    assert_eq!(outputs, (1, 1, 1));
    assert_eq!(
        ingest
            .run_coverage(dependent.id)
            .await
            .unwrap()
            .pending_artwork_publications,
        0
    );
    // The incomplete-generation success branch must settle receipts too.
    let staged = operator.start_run(policy.id).await.unwrap();
    let claim = ingest.claim("two-variants", 60).await.unwrap().unwrap();
    let mut next = page(vec![], 2, Completion::Continue);
    let mut input = commit.artwork_inputs[0].clone();
    input.manifest.variants.push(ArtworkVariantInput {
        variant: ArtworkVariant::Original,
        sha256: input.manifest.variants[0].sha256,
    });
    next.artwork_inputs.push(input);
    ingest
        .commit_traversal_page(&claim, &next, &TraversalWrites::default())
        .await
        .unwrap();
    sqlx::query("UPDATE queue.jobs SET available_at=now()+interval '1 hour' WHERE scope_id=$1 AND kind='artwork'").bind(staged.id).execute(&pool).await.unwrap();
    let claim = ingest
        .claim("newer-staged-generation", 60)
        .await
        .unwrap()
        .unwrap();
    next.artwork_inputs[0].reference.rights = json!({"notice":"new staged generation"});
    next.completion = Completion::Complete {
        evidence: json!({}),
    };
    ingest
        .commit_traversal_page(&claim, &next, &TraversalWrites::default())
        .await
        .unwrap();
    assert_eq!(
        ingest
            .run_coverage(staged.id)
            .await
            .unwrap()
            .pending_artwork_publications,
        1
    );
    assert_eq!(
        ingest
            .run_coverage(dependent.id)
            .await
            .unwrap()
            .pending_artwork_publications,
        0
    );
    for completed in 1..=2 {
        let claim = ingest.claim("variant", 60).await.unwrap().unwrap();
        let outcome = success(
            ingest
                .reserve_work_request(&claim, OperationClass::Artwork)
                .await
                .unwrap(),
        );
        let art_id = media
            .publish_with_outcomes(&claim, &object, std::slice::from_ref(&outcome))
            .await
            .unwrap();
        let outcome: String =
            sqlx::query_scalar("SELECT outcome FROM ingest.request_receipts WHERE id=$1")
                .bind(outcome.receipt_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(outcome, "success");
        assert_eq!(window(&pool, &claim).await, 0);
        let state: String =
            sqlx::query_scalar("SELECT state FROM media.artwork_generations WHERE artwork_id=$1 AND revision=(SELECT current_generation FROM media.artwork WHERE id=$1)")
                .bind(art_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            state,
            if completed == 2 {
                "published"
            } else {
                "pending"
            }
        );
        assert_eq!(
            ingest
                .run_coverage(staged.id)
                .await
                .unwrap()
                .pending_artwork_publications,
            if completed == 2 { 0 } else { 1 }
        );
    }
    assert_eq!(
        ingest.run(staged.id).await.unwrap().unwrap().state,
        "complete"
    );
    let unrelated = catalog
        .configure_source("unrelated-artwork", "jellyfin", &json!({}), &json!({}))
        .await
        .unwrap();
    catalog
        .configure_library(unrelated.id, "unrelated-a", "a", "A")
        .await
        .unwrap();
    let unrelated_policy=operator.create_policy(unrelated.id,&PolicyWrite {enabled:true,selection_json:json!({"library_ids":["unrelated-a"],"work_budget":{"mode":"per_work_commit","max_requests":7,"max_records":10}}),max_items:1000,max_requests:1000,incremental_seconds:60,reconcile_seconds:3600,enrich_metadata:true,enrich_people:false,enrich_artwork:true}).await.unwrap();
    let unrelated_run = operator.start_run(unrelated_policy.id).await.unwrap();
    let unrelated_claim = ingest
        .claim("unrelated-staging", 60)
        .await
        .unwrap()
        .unwrap();
    let mut unrelated_page = page(
        vec![item("atomic", "movie", None)],
        0,
        Completion::Complete {
            evidence: json!({}),
        },
    );
    unrelated_page.artwork_inputs = commit.artwork_inputs.clone();
    ingest
        .commit_traversal_page(
            &unrelated_claim,
            &unrelated_page,
            &TraversalWrites::default(),
        )
        .await
        .unwrap();
    assert_eq!(
        ingest
            .run_coverage(unrelated_run.id)
            .await
            .unwrap()
            .pending_artwork_publications,
        1
    );
    assert_eq!(
        ingest
            .run_coverage(staged.id)
            .await
            .unwrap()
            .pending_artwork_publications,
        0
    );
    assert_eq!(
        ingest
            .run_coverage(dependent.id)
            .await
            .unwrap()
            .pending_artwork_publications,
        0
    );
    // Historical runs without publication receipts retain their read interpretation.
    let legacy_run:Uuid=sqlx::query_scalar("INSERT INTO ingest.runs(policy_id,policy_version,policy_snapshot,selection_snapshot,source_id,state,finished_at) SELECT id,version,to_jsonb(p),'{}',source_id,'complete',now() FROM ingest.policies p WHERE id=$1 RETURNING id")
        .bind(unrelated_policy.id).fetch_one(&pool).await.unwrap();
    assert_eq!(
        ingest
            .run_coverage(legacy_run)
            .await
            .unwrap()
            .pending_artwork_publications,
        0
    );
    worker.close().await;
    reader.close().await;
    pool.close().await;
    sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}

async fn role_pool(pool: &PgPool, role: &'static str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(5)
        .after_connect(move |c, _| {
            Box::pin(async move {
                sqlx::query(&format!("SET ROLE {role}")).execute(c).await?;
                Ok(())
            })
        })
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap()
}
fn item(id: &str, kind: &str, parent: Option<&str>) -> ItemWrite {
    ItemWrite {
        public_id: id.into(),
        external_id: id.into(),
        kind: kind.into(),
        parent_public_id: parent.map(str::to_owned),
        source_revision: Some("r1".into()),
        metadata: json!({"title":id}),
        provenance: json!({}),
        content_hash: vec![0; 32],
        library_external_ids: vec!["a".into()],
        external_ids: vec![],
        credits: vec![],
        document: id.into(),
        document_title: id.into(),
        document_version: 1,
    }
}
fn page(items: Vec<ItemWrite>, offset: usize, completion: Completion) -> PageCommit {
    PageCommit {
        items,
        completion,
        libraries: vec![],
        artwork_inputs: vec![],
        artwork_omissions: vec![],
        followups: vec![],
        checkpoint: CheckpointWrite {
            scope_key: "full".into(),
            cursor: json!({"offset":offset}),
            watermark: None,
        },
        requests: 0,
    }
}
fn success(receipt: RequestReceipt) -> RequestOutcomeWrite {
    RequestOutcomeWrite {
        receipt_id: receipt.id,
        outcome: RequestOutcome::Success,
        error_class: None,
    }
}
async fn window(pool: &PgPool, claim: &Claim) -> i64 {
    sqlx::query_scalar("SELECT window_requests FROM queue.jobs WHERE id=$1")
        .bind(claim.id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires explicitly configured disposable PostgreSQL 18 + pgvector admin URL"]
async fn postgres_full_traversal_contract() {
    let url = std::env::var("SCREENROOM_TEST_DATABASE_URL").unwrap();
    let admin = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .unwrap();
    let name = format!("screenroom_full_{}", Uuid::new_v4().simple());
    sqlx::query(&format!("CREATE DATABASE {name}"))
        .execute(&admin)
        .await
        .unwrap();
    let mut dburl = reqwest::Url::parse(&url).unwrap();
    dburl.set_path(&format!("/{name}"));
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(dburl.as_str())
        .await
        .unwrap();
    migrate(&pool).await;
    let worker = role_pool(&pool, "screenroom_worker").await;
    let reader = role_pool(&pool, "screenroom_catalog").await;
    let catalog = CatalogRepository::new(reader.clone());
    let ingest = IngestRepository::new(worker.clone());
    let operator = IngestRepository::new(reader.clone());
    let source = catalog
        .configure_source("full-test", "jellyfin", &json!({}), &json!({}))
        .await
        .unwrap();
    for lib in ["a", "b", "outside"] {
        catalog
            .configure_library(source.id, lib, lib, lib)
            .await
            .unwrap();
    }
    let p=operator.create_policy(source.id,&PolicyWrite {
        enabled:true, selection_json:json!({"library_ids":["a","b"],"work_budget":{"mode":"per_work_commit","max_requests":7,"max_records":200}}),
        max_items:100000,max_requests:100000,incremental_seconds:60,reconcile_seconds:3600,enrich_metadata:true,enrich_people:false,enrich_artwork:false,
    }).await.unwrap();
    let run = operator.start_run(p.id).await.unwrap();
    let claim = ingest.claim("first", 60).await.unwrap().unwrap();
    let _unknown = ingest
        .reserve_work_request(&claim, OperationClass::Page)
        .await
        .unwrap();
    ingest.retry(&claim, "transport", 0).await.unwrap();
    sqlx::query("UPDATE queue.jobs SET available_at=now() WHERE id=$1")
        .bind(claim.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        ingest
            .reserve_work_request(&claim, OperationClass::Page)
            .await
            .is_err()
    );
    let claim = ingest.claim("second", 60).await.unwrap().unwrap();
    let attempt = ingest
        .reserve_work_request(&claim, OperationClass::Page)
        .await
        .unwrap();
    for _ in 0..5 {
        ingest
            .reserve_work_request(&claim, OperationClass::Page)
            .await
            .unwrap();
    }
    assert!(
        ingest
            .reserve_work_request(&claim, OperationClass::Page)
            .await
            .is_err()
    );
    // A no-progress pause retains the same job and its spent window across resume.
    ingest
        .commit_traversal_page(
            &claim,
            &page(vec![], 0, Completion::Pause),
            &TraversalWrites::default(),
        )
        .await
        .unwrap();
    operator.resume(run.id).await.unwrap();
    let claim = ingest.claim("resumed", 60).await.unwrap().unwrap();
    assert_eq!(window(&pool, &claim).await, 7);
    assert!(
        ingest
            .reserve_work_request(&claim, OperationClass::Page)
            .await
            .is_err()
    );
    // An old-fence attempt cannot settle or replenish the window.
    let stale_outcome = success(attempt);
    assert!(
        ingest
            .record_work_outcome(&claim, &stale_outcome)
            .await
            .is_err()
    );
    // Preserve an honest exhausted run; a new explicit run provides a new snapshot/window.
    operator.cancel_run(run.id).await.unwrap();
    assert_eq!(
        ingest.run_coverage(run.id).await.unwrap().unknown_attempts,
        7
    );
    let run = operator.start_run(p.id).await.unwrap();
    let claim = ingest.claim("hierarchy", 60).await.unwrap().unwrap();
    let attempt = ingest
        .reserve_work_request(&claim, OperationClass::Collection)
        .await
        .unwrap();
    let observation_receipt_id = attempt.id;
    let hierarchy = vec![
        item("series", "series", None),
        item("season", "season", Some("series")),
        item("episode", "episode", Some("season")),
        item("box", "collection", None),
    ];
    let mut initial = page(hierarchy, 0, Completion::Continue);
    initial.followups.push(Followup {
        kind: JobKind::Artwork,
        dedupe_key: "invalid-direct-art".into(),
        input: json!({}),
    });
    assert!(
        ingest
            .commit_traversal_page(&claim, &initial, &TraversalWrites::default())
            .await
            .is_err()
    );
    initial.followups.clear();
    initial.followups.push(Followup {
        kind: JobKind::Embedding,
        dedupe_key: "full-followup".into(),
        input: json!({"external_id":"full-followup"}),
    });
    let writes = TraversalWrites {
        outcomes: vec![success(attempt)],
        library_touches: vec![LibraryTouch {
            external_id: "series".into(),
            library_external_id: "b".into(),
        }],
        collection_members: vec![CollectionMembershipWrite {
            container_external_id: "box".into(),
            member_external_id: "episode".into(),
            ordinal: Some(5),
            observation_receipt_id,
        }],
        ..Default::default()
    };
    let mut duplicate_page = writes.clone();
    duplicate_page
        .collection_members
        .push(writes.collection_members[0].clone());
    assert!(matches!(
        ingest
            .commit_traversal_page(&claim, &initial, &duplicate_page)
            .await,
        Err(Error::Invalid("repeated collection member"))
    ));
    assert_eq!(window(&pool, &claim).await, 1);
    let outcome: String =
        sqlx::query_scalar("SELECT outcome FROM ingest.request_receipts WHERE id=$1")
            .bind(observation_receipt_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(outcome, "unknown");
    let edges: i64 =
        sqlx::query_scalar("SELECT count(*) FROM catalog.collection_members WHERE source_id=$1")
            .bind(source.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(edges, 0);
    ingest
        .commit_traversal_page(&claim, &initial, &writes)
        .await
        .unwrap();
    assert_eq!(window(&pool, &claim).await, 0);
    assert_eq!(ingest.run_coverage(run.id).await.unwrap().primary_unique, 0);
    sqlx::query("UPDATE queue.jobs SET available_at=now()+interval '1 day' WHERE scope_id=$1 AND kind<>'ingest'").bind(run.id).execute(&pool).await.unwrap();
    // Adversarially overwrite mutable provenance; immutable per-run observations must win.
    let overwritten_run: Uuid =
        sqlx::query_scalar("SELECT id FROM ingest.runs WHERE source_id=$1 AND id<>$2 LIMIT 1")
            .bind(source.id)
            .bind(run.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query("UPDATE catalog.collection_members SET observed_run_id=$2 WHERE source_id=$1")
        .bind(source.id)
        .bind(overwritten_run)
        .execute(&pool)
        .await
        .unwrap();
    let repeated = ingest.claim("repeated-member", 60).await.unwrap().unwrap();
    let receipt = ingest
        .reserve_work_request(&repeated, OperationClass::Collection)
        .await
        .unwrap();
    let mut edge = writes.collection_members[0].clone();
    edge.observation_receipt_id = receipt.id;
    let outcome = success(receipt);
    let attempt = page(vec![], 1, Completion::Continue);
    let reopened = IngestRepository::new(role_pool(&pool, "screenroom_worker").await);
    assert!(matches!(
        reopened
            .commit_traversal_page(
                &repeated,
                &attempt,
                &TraversalWrites {
                    outcomes: vec![outcome.clone()],
                    collection_members: vec![edge],
                    ..Default::default()
                }
            )
            .await,
        Err(Error::Invalid("repeated collection member"))
    ));
    assert_eq!(window(&pool, &repeated).await, 1);
    assert_eq!(
        ingest.run_checkpoint(&run, "full").await.unwrap().unwrap(),
        json!({"offset":0})
    );
    let state: String =
        sqlx::query_scalar("SELECT outcome FROM ingest.request_receipts WHERE id=$1")
            .bind(outcome.receipt_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(state, "unknown");
    let latest: Uuid = sqlx::query_scalar(
        "SELECT observed_run_id FROM catalog.collection_members WHERE source_id=$1",
    )
    .bind(source.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(latest, overwritten_run);
    ingest
        .commit_traversal_page(
            &repeated,
            &attempt,
            &TraversalWrites {
                outcomes: vec![outcome],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    for start in (0..20_001).step_by(200) {
        let claim = ingest.claim("pages", 60).await.unwrap().unwrap();
        let outcome = success(
            ingest
                .reserve_work_request(&claim, OperationClass::Page)
                .await
                .unwrap(),
        );
        let end = (start + 200).min(20_001);
        let items = (start..end)
            .map(|i| item(&format!("full-{i:05}"), "movie", None))
            .collect::<Vec<_>>();
        let primary_external_ids = items.iter().map(|i| i.external_id.clone()).collect();
        let commit = page(items, end, Completion::Continue);
        assert!(commit.checkpoint.cursor.to_string().len() < 64);
        ingest
            .commit_traversal_page(
                &claim,
                &commit,
                &TraversalWrites {
                    primary_external_ids,
                    outcomes: vec![outcome],
                    ..Default::default()
                },
            )
            .await
            .unwrap();
    }
    let coverage = ingest.run_coverage(run.id).await.unwrap();
    assert_eq!(coverage.primary_unique, 20_001);
    let progress = ingest.run(run.id).await.unwrap().unwrap();
    assert_eq!(progress.items, 20_005);
    assert!(progress.requests > 7);
    let observed = ingest
        .observed_items(run.id, "collection", None, 200)
        .await
        .unwrap();
    assert_eq!(observed.len(), 1);
    assert_eq!(observed[0].external_id, "box");
    let collection_documents:i64=sqlx::query_scalar("SELECT count(*) FROM search.documents d JOIN catalog.source_items si ON si.id=d.source_item_id JOIN catalog.items i ON i.id=si.item_id WHERE i.kind='collection'").fetch_one(&pool).await.unwrap();
    assert_eq!(collection_documents, 0);
    assert!(
        ingest
            .observed_items(run.id, "collection", Some("box"), 200)
            .await
            .unwrap()
            .is_empty()
    );
    let first = ingest
        .observed_items(run.id, "movie", None, 200)
        .await
        .unwrap();
    assert_eq!(first.len(), 200);
    let second = ingest
        .observed_items(run.id, "movie", Some(&first[199].external_id), 200)
        .await
        .unwrap();
    assert_eq!(second[0].external_id, "full-00200");
    let fresh_pool = role_pool(&pool, "screenroom_worker").await;
    let fresh = IngestRepository::new(fresh_pool.clone());
    assert_eq!(
        fresh.run_coverage(run.id).await.unwrap().primary_unique,
        20_001
    );
    assert_eq!(
        fresh
            .observed_items(run.id, "collection", None, 200)
            .await
            .unwrap()[0]
            .external_id,
        "box"
    );
    fresh_pool.close().await;
    // Duplicate primary/ancestor writes do not add primary receipts.
    let claim = ingest.claim("duplicates", 60).await.unwrap().unwrap();
    let outcome = success(
        ingest
            .reserve_work_request(&claim, OperationClass::Ancestor)
            .await
            .unwrap(),
    );
    ingest
        .commit_traversal_page(
            &claim,
            &page(
                vec![item("series", "series", None)],
                20_001,
                Completion::Continue,
            ),
            &TraversalWrites {
                primary_external_ids: vec!["full-00000".into(), "full-00000".into()],
                outcomes: vec![outcome],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(window(&pool, &claim).await, 1);
    assert_eq!(
        ingest.run_coverage(run.id).await.unwrap().primary_unique,
        20_001
    );
    let claim = ingest.claim("last", 60).await.unwrap().unwrap();
    let outcome = success(
        ingest
            .reserve_work_request(&claim, OperationClass::Page)
            .await
            .unwrap(),
    );
    ingest
        .commit_traversal_page(
            &claim,
            &page(
                vec![],
                20_002,
                Completion::Complete {
                    evidence: json!({"non_authoritative":true}),
                },
            ),
            &TraversalWrites {
                outcomes: vec![outcome],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(!operator.policy(p.id).await.unwrap().unwrap().automatic_runs);
    assert!(
        catalog
            .get(&Scope::default(), "full-00000")
            .await
            .unwrap()
            .is_some()
    );
    let members = catalog
        .collection_members(&Scope::default(), "box", None, 1)
        .await
        .unwrap();
    assert_eq!(members[0].item.public_id, "episode");
    assert_eq!(members[0].item.parent_public_id.as_deref(), Some("season"));
    assert_eq!(members[0].ordinal, Some(5));
    assert!(
        catalog
            .collection_members(&Scope::default(), "box", Some("episode"), 1)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        catalog
            .collection_members(
                &Scope {
                    library_ids: vec!["b".into()],
                    ..Default::default()
                },
                "box",
                None,
                10
            )
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        catalog
            .children(&Scope::default(), "season", None, 10)
            .await
            .unwrap()[0]
            .public_id,
        "episode"
    );
    // A completed run's followup has its own window, despite the ingest aggregate.
    sqlx::query("UPDATE queue.jobs SET available_at=now() WHERE scope_id=$1 AND kind<>'ingest'")
        .bind(run.id)
        .execute(&pool)
        .await
        .unwrap();
    let followup = ingest.claim("followup", 60).await.unwrap().unwrap();
    let receipt = ingest
        .reserve_work_request(&followup, OperationClass::Embedding)
        .await
        .unwrap();
    let outcome = RequestOutcomeWrite {
        receipt_id: receipt.id,
        outcome: RequestOutcome::Retryable,
        error_class: Some(RequestErrorClass::Throttled),
    };
    // Another job/run cannot spend requests during the source cooldown.
    let next = operator.start_run(p.id).await.unwrap();
    let competing = ingest.claim("competing", 60).await.unwrap().unwrap();
    ingest.defer_work(&followup, &outcome, 60).await.unwrap();
    assert!(ingest.claim("cooldown", 60).await.unwrap().is_none());
    assert!(
        ingest
            .reserve_work_request(&competing, OperationClass::Page)
            .await
            .is_err()
    );
    assert_eq!(window(&pool, &competing).await, 0);
    sqlx::query(
        "UPDATE ingest.source_cooldowns SET until_at=now()-interval '1 second' WHERE source_id=$1",
    )
    .bind(source.id)
    .execute(&pool)
    .await
    .unwrap();
    let receipt = ingest
        .reserve_work_request(&competing, OperationClass::Page)
        .await
        .unwrap();
    let mut stale = competing.clone();
    stale.fence += 1;
    assert!(
        ingest
            .record_work_outcome(&stale, &success(receipt))
            .await
            .is_err()
    );
    sqlx::query("UPDATE queue.jobs SET lease_until=now()-interval '1 second' WHERE id=$1")
        .bind(competing.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        ingest
            .reserve_work_request(&competing, OperationClass::Page)
            .await
            .is_err()
    );
    let newer = ingest.claim("newer", 60).await.unwrap().unwrap();
    assert_eq!(newer.id, competing.id);
    assert_eq!(window(&pool, &newer).await, 1);
    for _ in 0..5 {
        ingest
            .reserve_work_request(&newer, OperationClass::Page)
            .await
            .unwrap();
    }
    let (a, b) = tokio::join!(
        ingest.reserve_work_request(&newer, OperationClass::Page),
        ingest.reserve_work_request(&newer, OperationClass::Page)
    );
    assert_ne!(a.is_ok(), b.is_ok());
    assert_eq!(window(&pool, &newer).await, 7);
    let receipt = a.or(b).unwrap();
    let observation_receipt_id = receipt.id;
    // Collection provenance requires a successful collection operation, not arbitrary I/O.
    ingest
        .record_work_outcome(&newer, &success(receipt))
        .await
        .unwrap();
    let writer = CatalogRepository::new(worker.clone());
    let wrong_operation = CollectionMembershipWrite {
        container_external_id: "box".into(),
        member_external_id: "series".into(),
        ordinal: None,
        observation_receipt_id,
    };
    assert!(
        writer
            .upsert_collection_members(&newer, &[wrong_operation])
            .await
            .is_err()
    );
    for i in 0..205 {
        ingest
            .record_unresolved_operation(&newer, OperationClass::Collection, &format!("member:{i}"))
            .await
            .unwrap();
    }
    assert!(
        ingest
            .record_unresolved_operation(&newer, OperationClass::Collection, "https://secret")
            .await
            .is_err()
    );
    assert_eq!(
        ingest
            .run_coverage(next.id)
            .await
            .unwrap()
            .unresolved_operations,
        205
    );
    let incomplete = page(
        vec![],
        99,
        Completion::Complete {
            evidence: json!({}),
        },
    );
    assert!(
        ingest
            .commit_traversal_page(&newer, &incomplete, &TraversalWrites::default())
            .await
            .is_err()
    );
    let mut after = None;
    let mut resolved = 0;
    loop {
        let pending = ingest
            .pending_operations(next.id, after, 200)
            .await
            .unwrap();
        if pending.is_empty() {
            break;
        }
        after = Some(pending.last().unwrap().id);
        for operation in pending {
            ingest
                .resolve_operation(&newer, operation.id)
                .await
                .unwrap();
            resolved += 1;
        }
    }
    assert_eq!(resolved, 205);
    assert_eq!(window(&pool, &newer).await, 7);
    assert_eq!(
        ingest
            .run_coverage(next.id)
            .await
            .unwrap()
            .unresolved_operations,
        0
    );
    operator.cancel_run(next.id).await.unwrap();
    let coverage = ingest.run_coverage(run.id).await.unwrap();
    assert_eq!(coverage.followup_delayed, 1);
    assert_eq!(coverage.retryable_attempts, 1);
    let last = operator.start_run(p.id).await.unwrap();
    sqlx::query("UPDATE catalog.library_items SET last_seen_run_id=$2 WHERE source_id=$1")
        .bind(source.id)
        .bind(last.id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        ingest
            .observed_items(run.id, "collection", None, 200)
            .await
            .unwrap()[0]
            .external_id,
        "box"
    );
    let claim = ingest.claim("edge-cases", 60).await.unwrap().unwrap();
    // Both aggregate counters reject overflow without a partial window charge.
    sqlx::query("UPDATE ingest.runs SET requests=$2 WHERE id=$1")
        .bind(last.id)
        .bind(i64::MAX)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        ingest
            .reserve_work_request(&claim, OperationClass::Page)
            .await
            .is_err()
    );
    assert_eq!(window(&pool, &claim).await, 0);
    sqlx::query("UPDATE ingest.runs SET requests=0,items=$2 WHERE id=$1")
        .bind(last.id)
        .bind(i64::MAX)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        ingest
            .commit_traversal_page(
                &claim,
                &page(
                    vec![item("overflow", "movie", None)],
                    0,
                    Completion::Continue
                ),
                &TraversalWrites::default()
            )
            .await
            .is_err()
    );
    sqlx::query("UPDATE ingest.runs SET items=0 WHERE id=$1")
        .bind(last.id)
        .execute(&pool)
        .await
        .unwrap();
    let oversized = page(
        (0..201)
            .map(|i| item(&format!("oversized-{i}"), "movie", None))
            .collect(),
        0,
        Completion::Continue,
    );
    assert!(
        ingest
            .commit_traversal_page(&claim, &oversized, &TraversalWrites::default())
            .await
            .is_err()
    );
    catalog.set_source_state(source.id, false).await.unwrap();
    assert!(
        ingest
            .reserve_work_request(&claim, OperationClass::Collection)
            .await
            .is_err()
    );
    assert!(
        catalog
            .collection_members(&Scope::default(), "box", None, 200)
            .await
            .unwrap()
            .is_empty()
    );
    catalog.set_source_state(source.id, true).await.unwrap();
    sqlx::query("UPDATE ingest.policies SET version=version+1 WHERE id=$1")
        .bind(p.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        ingest
            .reserve_work_request(&claim, OperationClass::Collection)
            .await
            .is_err()
    );
    sqlx::query("UPDATE ingest.policies SET version=version-1 WHERE id=$1")
        .bind(p.id)
        .execute(&pool)
        .await
        .unwrap();
    let receipt = ingest
        .reserve_work_request(&claim, OperationClass::Collection)
        .await
        .unwrap();
    let observation_receipt_id = receipt.id;
    ingest
        .record_work_outcome(&claim, &success(receipt))
        .await
        .unwrap();
    let member = CollectionMembershipWrite {
        container_external_id: "box".into(),
        member_external_id: "series".into(),
        ordinal: None,
        observation_receipt_id,
    };
    writer
        .upsert_collection_members(&claim, std::slice::from_ref(&member))
        .await
        .unwrap();
    assert!(matches!(
        writer
            .upsert_collection_members(&claim, std::slice::from_ref(&member))
            .await,
        Err(Error::Invalid("repeated collection member"))
    ));
    for (container, member_id) in [
        ("episode", "series"),
        ("box", "foreign-only"),
        ("box", "box"),
    ] {
        let invalid = CollectionMembershipWrite {
            container_external_id: container.into(),
            member_external_id: member_id.into(),
            ordinal: None,
            observation_receipt_id,
        };
        assert!(
            writer
                .upsert_collection_members(&claim, &[invalid])
                .await
                .is_err()
        );
    }
    let first = catalog
        .collection_members(&Scope::default(), "box", None, 1)
        .await
        .unwrap();
    let second = catalog
        .collection_members(&Scope::default(), "box", Some(&first[0].item.public_id), 1)
        .await
        .unwrap();
    assert_eq!(first[0].item.public_id, "episode");
    assert_eq!(second[0].item.public_id, "series");
    assert_eq!(
        catalog
            .collection_members(
                &Scope {
                    kinds: vec!["episode".into()],
                    ..Default::default()
                },
                "box",
                None,
                200
            )
            .await
            .unwrap()
            .len(),
        1
    );
    // The other job's terminal success never replenishes this ingest window.
    sqlx::query("UPDATE queue.jobs SET available_at=now() WHERE scope_id=$1 AND kind<>'ingest'")
        .bind(run.id)
        .execute(&pool)
        .await
        .unwrap();
    let followup = ingest.claim("finish-followup", 60).await.unwrap().unwrap();
    assert_eq!(window(&pool, &followup).await, 1);
    let receipt = ingest
        .reserve_work_request(&followup, OperationClass::Embedding)
        .await
        .unwrap();
    ingest
        .record_work_outcome(&followup, &success(receipt))
        .await
        .unwrap();
    ingest.complete_job(&followup).await.unwrap();
    assert_eq!(window(&pool, &followup).await, 0);
    assert_eq!(window(&pool, &claim).await, 1);
    operator.cancel_run(last.id).await.unwrap();
    let other = catalog
        .configure_source("aaa-other-source", "jellyfin", &json!({}), &json!({}))
        .await
        .unwrap();
    catalog
        .configure_library(other.id, "other-a", "a", "Other")
        .await
        .unwrap();
    let other_policy=operator.create_policy(other.id,&PolicyWrite {
        enabled:true, selection_json:json!({"library_ids":["other-a"],"work_budget":{"mode":"per_work_commit","max_requests":7,"max_records":200}}),
        max_items:100000,max_requests:100000,incremental_seconds:60,reconcile_seconds:3600,enrich_metadata:true,enrich_people:false,enrich_artwork:false,
    }).await.unwrap();
    let other_run = operator.start_run(other_policy.id).await.unwrap();
    let claim = ingest.claim("other-source", 60).await.unwrap().unwrap();
    let receipt = ingest
        .reserve_work_request(&claim, OperationClass::Collection)
        .await
        .unwrap();
    let observation_receipt_id = receipt.id;
    ingest
        .commit_traversal_page(
            &claim,
            &page(
                vec![
                    item("series", "series", None),
                    item("season", "season", Some("series")),
                    item("episode", "episode", Some("season")),
                    item("box", "collection", None),
                    item("foreign-only", "movie", None),
                ],
                0,
                Completion::Complete {
                    evidence: json!({}),
                },
            ),
            &TraversalWrites {
                outcomes: vec![success(receipt)],
                collection_members: vec![
                    CollectionMembershipWrite {
                        container_external_id: "box".into(),
                        member_external_id: "episode".into(),
                        ordinal: Some(9),
                        observation_receipt_id,
                    },
                    CollectionMembershipWrite {
                        container_external_id: "box".into(),
                        member_external_id: "series".into(),
                        ordinal: None,
                        observation_receipt_id,
                    },
                ],
                ..Default::default()
            },
        )
        .await
        .unwrap();
    sqlx::query("DELETE FROM catalog.collection_members cm USING catalog.source_items si WHERE cm.source_id=$1 AND cm.member_id=si.id AND si.external_id='episode'")
        .bind(source.id).execute(&pool).await.unwrap();
    let mixed = catalog
        .collection_members(&Scope::default(), "box", None, 200)
        .await
        .unwrap();
    assert_eq!(mixed[0].item.source_key, "aaa-other-source");
    assert_eq!(mixed[1].item.source_key, "aaa-other-source");
    sqlx::query("UPDATE catalog.sources SET provider=CASE WHEN id=$1 THEN 'unsupported' ELSE 'jellyfin' END WHERE id=ANY($2)")
        .bind(source.id).bind(vec![source.id,other.id]).execute(&pool).await.unwrap();
    let renamed = catalog
        .collection_members(&Scope::default(), "box", None, 200)
        .await
        .unwrap();
    assert_eq!(renamed[1].item.source_key, "aaa-other-source");
    let scoped = catalog
        .collection_members(
            &Scope {
                library_ids: vec!["other-a".into()],
                ..Default::default()
            },
            "box",
            None,
            200,
        )
        .await
        .unwrap();
    assert_eq!(scoped.len(), 2);
    assert_eq!(scoped[0].item.parent_public_id.as_deref(), Some("season"));
    let cross = operator.start_run(p.id).await.unwrap();
    let claim = ingest
        .claim("cross-source-edge", 60)
        .await
        .unwrap()
        .unwrap();
    let receipt = ingest
        .reserve_work_request(&claim, OperationClass::Collection)
        .await
        .unwrap();
    let observation_receipt_id = receipt.id;
    ingest
        .record_work_outcome(&claim, &success(receipt))
        .await
        .unwrap();
    assert!(
        writer
            .upsert_collection_members(
                &claim,
                &[CollectionMembershipWrite {
                    container_external_id: "box".into(),
                    member_external_id: "foreign-only".into(),
                    ordinal: None,
                    observation_receipt_id
                }]
            )
            .await
            .is_err()
    );
    let shared = vec![
        CollectionMembershipWrite {
            container_external_id: "box".into(),
            member_external_id: "episode".into(),
            ordinal: None,
            observation_receipt_id,
        },
        CollectionMembershipWrite {
            container_external_id: "box-two".into(),
            member_external_id: "episode".into(),
            ordinal: None,
            observation_receipt_id,
        },
    ];
    ingest
        .commit_traversal_page(
            &claim,
            &page(
                vec![
                    item("series", "series", None),
                    item("season", "season", Some("series")),
                    item("episode", "episode", Some("season")),
                    item("box", "collection", None),
                    item("box-two", "collection", None),
                ],
                0,
                Completion::Complete {
                    evidence: json!({}),
                },
            ),
            &TraversalWrites {
                collection_members: shared,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        ingest.run(cross.id).await.unwrap().unwrap().state,
        "complete"
    );
    assert_eq!(
        ingest
            .run_coverage(other_run.id)
            .await
            .unwrap()
            .primary_unique,
        0
    );
    let exhausted = operator.start_run(p.id).await.unwrap();
    let claim = ingest.claim("no-progress", 60).await.unwrap().unwrap();
    sqlx::query("UPDATE queue.jobs SET max_attempts=attempts WHERE id=$1")
        .bind(claim.id)
        .execute(&pool)
        .await
        .unwrap();
    ingest
        .commit_traversal_page(
            &claim,
            &page(vec![], 0, Completion::Pause),
            &TraversalWrites::default(),
        )
        .await
        .unwrap();
    operator.resume(exhausted.id).await.unwrap();
    assert!(
        ingest
            .claim("retire-exhausted", 60)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        ingest.run(exhausted.id).await.unwrap().unwrap().state,
        "failed"
    );
    worker.close().await;
    reader.close().await;
    pool.close().await;
    sqlx::query(&format!("DROP DATABASE {name} WITH (FORCE)"))
        .execute(&admin)
        .await
        .unwrap();
    admin.close().await;
}
