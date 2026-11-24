//! Opt-in only: creates and drops unique databases on the explicitly authorized
//! disposable PostgreSQL 18 + vector target. Never uses the development selector.
use screenroom_backend::{
    Uuid,
    catalog::Scope,
    chrono::Utc,
    db::MIGRATOR,
    playback::*,
    sqlx::{self, PgPool, postgres::PgPoolOptions},
    watched::{Binding, WatchedRepository},
};
use screenroom_catalog_adapter::user_state::{WatchedObservation, WatchedSnapshot};
use serde_json::{Value, json};
use std::borrow::Cow;

struct Fixture {
    admin: PgPool,
    pool: PgPool,
    runtime: PgPool,
    name: String,
    user: Uuid,
    source: Uuid,
    items: Vec<Uuid>,
    repo: PlaybackRepository,
}

impl Fixture {
    async fn recover_abandoned(&self) -> screenroom_backend::Result<u64> {
        let mut connection = self.runtime.acquire().await?;
        connection.close_on_drop();
        assert!(try_gateway_lock(&mut connection).await?);
        PlaybackRepository::recover_abandoned(&mut connection).await
    }

    async fn new(upgrade: bool) -> Self {
        let url = std::env::var("SCREENROOM_PLAYBACK_TEST_DATABASE_URL")
            .expect("explicit disposable SCREENROOM_PLAYBACK_TEST_DATABASE_URL required");
        let admin = PgPool::connect(&url).await.unwrap();
        let name = format!("playback_{}", Uuid::new_v4().simple());
        sqlx::query(&format!("CREATE DATABASE {name}"))
            .execute(&admin)
            .await
            .unwrap();
        let options = admin.connect_options().as_ref().clone().database(&name);
        let pool = PgPoolOptions::new()
            .max_connections(6)
            .connect_with(options.clone())
            .await
            .unwrap();
        if upgrade {
            let before = sqlx::migrate::Migrator {
                migrations: Cow::Owned(
                    MIGRATOR
                        .iter()
                        .filter(|m| m.version < 202611140023)
                        .cloned()
                        .collect(),
                ),
                ..sqlx::migrate::Migrator::DEFAULT
            };
            before.run(&pool).await.unwrap();
        } else {
            MIGRATOR.run(&pool).await.unwrap();
        }
        let user = Uuid::new_v4();
        let source = Uuid::new_v4();
        let connection = Uuid::new_v4();
        let library = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO identity.users(id,public_id,display_name) VALUES($1,$2,'Owned fixture')",
        )
        .bind(user)
        .bind(user.to_string())
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO catalog.sources(id,source_key,provider,configuration) VALUES($1,'owned-source','jellyfin',$2)")
            .bind(source).bind(json!({"connection_id":connection,"connection_generation":"1","locator_revision":"a".repeat(64)})).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO connections.jellyfin(id,source_id,server_id,server_name,server_version,base_url,credential_ref,revision,indexing_profile_id) VALUES($1,$2,'owned-server','Owned','10.10.7','http://127.0.0.1:1', $3,$4,'not-personal')")
            .bind(connection).bind(source).bind(Uuid::new_v4()).bind("a".repeat(64)).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO identity.jellyfin_user_links(user_id,source_id,jellyfin_user_id) VALUES($1,'owned-source','personal-profile')")
            .bind(user).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO catalog.libraries(id,source_id,public_id,external_id,title) VALUES($1,$2,'owned-library','remote-library','Owned')")
            .bind(library).bind(source).execute(&pool).await.unwrap();
        let mut items = Vec::new();
        for n in 0..4 {
            let item = Uuid::new_v4();
            let source_item = Uuid::new_v4();
            sqlx::query("INSERT INTO catalog.items(id,public_id,kind) VALUES($1,$2,$3)")
                .bind(item)
                .bind(format!("owned-item-{n}"))
                .bind(if n == 3 { "series" } else { "movie" })
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query("INSERT INTO catalog.source_items(id,source_id,item_id,external_id,metadata,content_hash) VALUES($1,$2,$3,$4,'{}',decode(repeat('00',32),'hex'))")
                .bind(source_item).bind(source).bind(item).bind(format!("remote-{n}")).execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO catalog.library_items(library_id,source_item_id,source_id,last_seen_run_id) VALUES($1,$2,$3,$4)")
                .bind(library).bind(source_item).bind(source).bind(Uuid::new_v4()).execute(&pool).await.unwrap();
            items.push(source_item);
        }
        if upgrade {
            let generation = Uuid::new_v4();
            sqlx::query("INSERT INTO personal.watched_snapshots(user_id,source_id,generation,binding_revision,fetched_at,complete) SELECT $1,$2,$3,updated_at::text,clock_timestamp(),true FROM identity.users WHERE id=$1")
                .bind(user).bind(source).bind(generation).execute(&pool).await.unwrap();
            sqlx::query("INSERT INTO personal.watched_observations(generation,external_item_id,watched) VALUES($1,'remote-0',true)")
                .bind(generation).execute(&pool).await.unwrap();
            MIGRATOR.run(&pool).await.unwrap();
        }
        let runtime = PgPoolOptions::new()
            .max_connections(6)
            .after_connect(|connection, _| {
                Box::pin(async move {
                    sqlx::query("SET ROLE screenroom_playback")
                        .execute(connection)
                        .await?;
                    Ok(())
                })
            })
            .connect_with(options)
            .await
            .unwrap();
        let repo = PlaybackRepository::new(runtime.clone(), Limits::default()).unwrap();
        Self {
            admin,
            pool,
            runtime,
            name,
            user,
            source,
            items,
            repo,
        }
    }

    async fn create(
        &self,
        n: usize,
        request: Uuid,
        body: &Value,
    ) -> screenroom_backend::Result<Created> {
        self.repo
            .create(Create {
                user_id: self.user,
                public_item_id: &format!("owned-item-{n}"),
                scope: &scope(),
                request_id: request,
                normalized_body: body,
                capabilities: &json!({"owned":true}),
                control_hash: &secrets::capability_hash("owned-control"),
            })
            .await
    }

    async fn ready(&self, n: usize) -> Created {
        let created = self
            .create(n, Uuid::new_v4(), &json!({"item":n}))
            .await
            .unwrap();
        self.repo.before_io(&created.operation).await.unwrap();
        self.repo
            .finish(
                &created.operation,
                Outcome::Confirmed(&json!({"ready":true})),
                "ready",
            )
            .await
            .unwrap();
        created
    }

    async fn binding(&self) -> Binding {
        Binding {
            user_id: self.user,
            source_id: self.source,
            revision: sqlx::query_scalar("SELECT updated_at::text FROM identity.users WHERE id=$1")
                .bind(self.user)
                .fetch_one(&self.pool)
                .await
                .unwrap(),
        }
    }

    async fn close(self) {
        self.runtime.close().await;
        self.pool.close().await;
        assert!(Uuid::parse_str(self.name.strip_prefix("playback_").unwrap()).is_ok());
        sqlx::query(&format!("DROP DATABASE {}", self.name))
            .execute(&self.admin)
            .await
            .unwrap();
        self.admin.close().await;
    }
}

fn scope() -> Scope {
    Scope {
        source_keys: vec!["owned-source".into()],
        library_ids: vec!["owned-library".into()],
        ..Scope::default()
    }
}
fn authority(f: &Fixture, s: &Session) -> Authority<'static> {
    Authority {
        user_id: f.user,
        session_id: s.id,
        control_capability: "owned-control",
    }
}
fn new(reservation: Reservation) -> Operation {
    match reservation {
        Reservation::New(op) => op,
        _ => panic!("expected new reservation"),
    }
}
fn event(kind: EventKind, position: &str) -> EventBody {
    EventBody {
        kind,
        position_ms: position.into(),
        paused: false,
        muted: false,
        volume: None,
    }
}

#[tokio::test]
#[ignore = "requires explicitly approved disposable PostgreSQL; database tests deliberately UNRUN"]
async fn gateway_receipts_capture_atomic_state_and_safe_uncertain_metadata() {
    let f = Fixture::new(false).await;
    let created = f
        .create(0, Uuid::new_v4(), &json!({"position_ms":"0"}))
        .await
        .unwrap();
    f.repo.before_io(&created.operation).await.unwrap();
    f.repo
        .finish_recorded(
            &created.operation,
            Outcome::Confirmed(&json!({"prepared":true})),
            "ready",
        )
        .await
        .unwrap();
    let receipt = f.repo.operation_receipt(&created.operation).await.unwrap();
    let snapshot: ReceiptSnapshot =
        serde_json::from_value(receipt.result.as_ref().unwrap()["snapshot"].clone()).unwrap();
    assert_eq!(snapshot.state, "ready");
    assert_eq!(snapshot.replay_ref, created.operation.replay_ref.unwrap());
    assert!(
        !serde_json::to_string(receipt.result.as_ref().unwrap())
            .unwrap()
            .contains("owned-control")
    );
    let auth = authority(&f, &created.session);
    let op = new(f
        .repo
        .reserve_event(&auth, 1, 0, &event(EventKind::Started, "0"))
        .await
        .unwrap());
    f.repo.before_io(&op).await.unwrap();
    f.repo.finish_failure_recorded(&op, 12).await.unwrap();
    let receipt = f.repo.operation_receipt(&op).await.unwrap();
    assert_eq!(receipt.status, "uncertain");
    assert_eq!(receipt.result.as_ref().unwrap()["outcome"]["failure"], 12);
    assert_eq!(
        receipt.result.as_ref().unwrap()["snapshot"]["state"],
        "uncertain"
    );
    let effect: String =
        sqlx::query_scalar("SELECT status FROM playback.effects WHERE operation_id=$1")
            .bind(op.id)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(effect, "unknown");
    assert!(
        f.repo
            .reserve_event(&auth, 2, 0, &event(EventKind::Progress, "10"))
            .await
            .is_err()
    );
    f.close().await;
}

#[tokio::test]
#[ignore = "requires explicitly approved disposable PostgreSQL; database tests deliberately UNRUN"]
async fn gateway_local_heartbeat_records_exact_lease_and_finalizer_uses_dispatch_bit() {
    let f = Fixture::new(false).await;
    let created = f.ready(0).await;
    let auth = authority(&f, &created.session);
    let operation = new(f
        .repo
        .heartbeat_recorded(&auth, 1, 0, &event(EventKind::Heartbeat, "0"))
        .await
        .unwrap());
    let receipt = f.repo.operation_receipt(&operation).await.unwrap();
    let snapshot: ReceiptSnapshot =
        serde_json::from_value(receipt.result.as_ref().unwrap()["snapshot"].clone()).unwrap();
    let current = f.repo.get(&auth).await.unwrap();
    assert_eq!(snapshot.lease_until, current.lease_until);
    assert_eq!(snapshot.last_sequence, 1);
    assert!(!receipt.dispatched);
    let operation = new(f
        .repo
        .reserve_event(&auth, 2, 0, &event(EventKind::Progress, "1"))
        .await
        .unwrap());
    f.repo.finalize_abandoned(&operation).await.unwrap();
    let receipt = f.repo.operation_receipt(&operation).await.unwrap();
    assert_eq!(receipt.status, "failed");
    let effect: (String, String) =
        sqlx::query_as("SELECT status,evidence FROM playback.effects WHERE operation_id=$1")
            .bind(operation.id)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(effect, ("no_effect".into(), "not_dispatched".into()));
    f.close().await;
}

#[tokio::test]
#[ignore = "requires explicitly approved disposable PostgreSQL; database tests deliberately UNRUN"]
async fn gateway_lazy_resource_registration_inherits_exact_scope_and_revocation() {
    let f = Fixture::new(false).await;
    let created = f.create(0, Uuid::new_v4(), &json!({})).await.unwrap();
    f.repo.before_io(&created.operation).await.unwrap();
    let root = Uuid::new_v4();
    let resource = Resource {
        id: root,
        session_id: created.session.id,
        transport_revision: 1,
        source_item_id: created.session.source_item_id,
        media_source_id: "private-source".into(),
        private_mapping: json!({"opaque":root}),
    };
    let master = Resource {
        id: Uuid::new_v4(),
        private_mapping: json!({"kind":"master"}),
        ..resource.clone()
    };
    f.repo
        .publish_transport(
            &created.operation,
            Transport {
                play_session_id: "owned-play",
                media_hash: &secrets::capability_hash("media-only"),
                tracks: &json!([]),
                descriptor: &json!({}),
                resources: &[master, resource],
            },
        )
        .await
        .unwrap();
    f.repo
        .finish_recorded(&created.operation, Outcome::Confirmed(&json!({})), "ready")
        .await
        .unwrap();
    let child = Uuid::new_v4();
    assert!(
        f.repo
            .register_media_child(
                created.session.id,
                1,
                "owned-control",
                root,
                child,
                &json!({})
            )
            .await
            .is_err()
    );
    assert!(
        f.repo
            .register_media_child(
                created.session.id,
                1,
                "media-only",
                Uuid::new_v4(),
                child,
                &json!({})
            )
            .await
            .is_err()
    );
    f.repo
        .register_media_child(
            created.session.id,
            1,
            "media-only",
            root,
            child,
            &json!({"opaque":child}),
        )
        .await
        .unwrap();
    let (_, mapping) = f
        .repo
        .media(created.session.id, 1, "media-only", child)
        .await
        .unwrap();
    assert_eq!(mapping.source_item_id, created.session.source_item_id);
    assert_eq!(mapping.media_source_id, "private-source");
    // Two playlist roots plus 2050 children must cross the former total-resource cap.
    for _ in 1..2050 {
        let id = Uuid::new_v4();
        f.repo
            .register_media_child(
                created.session.id,
                1,
                "media-only",
                root,
                id,
                &json!({"opaque":id}),
            )
            .await
            .unwrap();
    }
    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM playback.resources WHERE session_id=$1 AND transport_revision=1",
    )
    .bind(created.session.id)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(total, 2052);
    // Seed only the boundary setup; the final admitted and rejected inserts use the repository.
    let remaining: Vec<Uuid> = (2052..8191).map(|_| Uuid::new_v4()).collect();
    sqlx::query("INSERT INTO playback.resources(id,session_id,transport_revision,source_item_id,media_source_id,private_mapping) SELECT id,$1,1,$2,'private-source','{}'::jsonb FROM unnest($3::uuid[]) AS children(id)")
        .bind(created.session.id)
        .bind(created.session.source_item_id)
        .bind(remaining)
        .execute(&f.pool)
        .await
        .unwrap();
    f.repo
        .register_media_child(
            created.session.id,
            1,
            "media-only",
            root,
            Uuid::new_v4(),
            &json!({}),
        )
        .await
        .unwrap();
    assert!(matches!(
        f.repo
            .register_media_child(
                created.session.id,
                1,
                "media-only",
                root,
                Uuid::new_v4(),
                &json!({}),
            )
            .await,
        Err(screenroom_backend::Error::Invalid("resource capacity"))
    ));
    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM playback.resources WHERE session_id=$1 AND transport_revision=1",
    )
    .bind(created.session.id)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(total, 8192);
    let auth = authority(&f, &created.session);
    f.repo
        .reserve(
            &auth,
            Mutation::Stop {
                request_id: Uuid::new_v4(),
                final_sequence: 0,
            },
            1,
            &json!({}),
            true,
        )
        .await
        .unwrap();
    assert!(matches!(
        f.repo
            .register_media_child(
                created.session.id,
                1,
                "media-only",
                root,
                Uuid::new_v4(),
                &json!({})
            )
            .await,
        Err(screenroom_backend::Error::Stale)
    ));
    f.close().await;
}
fn snapshot(watched: bool) -> WatchedSnapshot {
    WatchedSnapshot::complete(vec![WatchedObservation {
        external_item_id: "remote-0".into(),
        watched: Some(watched),
    }])
    .unwrap()
}

async fn binding_for(f: &Fixture, user: Uuid) -> Binding {
    Binding {
        user_id: user,
        source_id: f.source,
        revision: sqlx::query_scalar("SELECT updated_at::text FROM identity.users WHERE id=$1")
            .bind(user)
            .fetch_one(&f.pool)
            .await
            .unwrap(),
    }
}

async fn relink(f: &Fixture, user: Uuid, profile: &str) {
    let mut tx = f.pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM identity.users WHERE id=$1 FOR UPDATE")
        .bind(user)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    sqlx::query("INSERT INTO identity.jellyfin_user_links(user_id,source_id,jellyfin_user_id) VALUES($1,'owned-source',$2) ON CONFLICT(user_id,source_id) DO UPDATE SET jellyfin_user_id=EXCLUDED.jellyfin_user_id")
        .bind(user).bind(profile).execute(&mut *tx).await.unwrap();
    sqlx::query("UPDATE identity.users SET updated_at=GREATEST(clock_timestamp(),updated_at+interval '1 microsecond') WHERE id=$1")
        .bind(user).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
}

async fn alias_user(f: &Fixture, profile: &str) -> Uuid {
    let user = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO identity.users(id,public_id,display_name) VALUES($1,$2,'Owned alias')",
    )
    .bind(user)
    .bind(user.to_string())
    .execute(&f.pool)
    .await
    .unwrap();
    relink(f, user, profile).await;
    user
}

async fn effect_state(
    f: &Fixture,
    operation: &Operation,
) -> (String, bool, String, Option<String>) {
    sqlx::query_as("SELECT o.status,o.dispatched,e.status,e.evidence FROM playback.operations o JOIN playback.effects e ON e.operation_id=o.id WHERE o.id=$1")
        .bind(operation.id).fetch_one(&f.pool).await.unwrap()
}

#[tokio::test]
#[ignore = "explicit disposable SCREENROOM_PLAYBACK_TEST_DATABASE_URL required"]
async fn playback_user_lock_allows_alias_fk_insert_while_alias_waits_for_profile_lock() {
    let f = Fixture::new(false).await;
    let alias = alias_user(&f, "personal-profile").await;
    let mut first = f.runtime.begin().await.unwrap();
    let mut second = f.runtime.begin().await.unwrap();
    let _: Option<String> = sqlx::query_scalar("SELECT playback.lock_user($1)")
        .bind(f.user)
        .fetch_one(&mut *first)
        .await
        .unwrap();
    let _: Option<String> = sqlx::query_scalar("SELECT playback.lock_user($1)")
        .bind(alias)
        .fetch_one(&mut *second)
        .await
        .unwrap();
    let waiting_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *second)
        .await
        .unwrap();
    let profile_key = format!("{}:personal-profile", f.source);
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,1935897197))")
        .bind(&profile_key)
        .execute(&mut *first)
        .await
        .unwrap();
    let alias_revision = binding_for(&f, alias).await.revision;
    let first_insert = async {
        loop {
            let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM pg_locks WHERE pid=$1 AND locktype='advisory' AND NOT granted)")
                .bind(waiting_pid).fetch_one(&f.pool).await.unwrap();
            if waiting {
                break;
            }
            tokio::task::yield_now().await;
        }
        // advance_epoch's cross-alias INSERT runs while the alias holds its
        // playback user lock and is already waiting on the profile lock.
        sqlx::query("INSERT INTO playback.freshness(user_id,source_id,binding_revision,epoch) SELECT u.id,$1,u.updated_at::text,1 FROM identity.users u JOIN identity.jellyfin_user_links l ON l.user_id=u.id JOIN catalog.sources s ON s.source_key=l.source_id WHERE s.id=$1 AND l.jellyfin_user_id=$2 AND (u.id<>$3 OR u.updated_at::text<>$4) ON CONFLICT(user_id,source_id,binding_revision) DO UPDATE SET epoch=playback.freshness.epoch+1,updated_at=clock_timestamp()")
            .bind(f.source).bind("personal-profile").bind(f.user).bind(f.binding().await.revision)
            .execute(&mut *first).await.expect("alias FK KEY SHARE must coexist with its playback user lock");
        first.commit().await.unwrap();
    };
    let second_reservation = async {
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,1935897197))")
            .bind(&profile_key)
            .execute(&mut *second)
            .await
            .expect("profile-lock waiter must not be deadlock-aborted");
        let epoch:i64=sqlx::query_scalar("SELECT epoch FROM playback.freshness WHERE user_id=$1 AND source_id=$2 AND binding_revision=$3")
            .bind(alias).bind(f.source).bind(&alias_revision).fetch_one(&mut *second).await.unwrap();
        second.rollback().await.unwrap();
        epoch
    };
    let ((), epoch) = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        tokio::join!(first_insert, second_reservation)
    })
    .await
    .expect("controlled user/profile lock ordering must complete without a lock cycle");
    assert_eq!(epoch, 1);
    f.close().await;
}

#[tokio::test]
#[ignore = "explicit disposable SCREENROOM_PLAYBACK_TEST_DATABASE_URL required"]
async fn abandonment_distinguishes_unsent_and_dispatched_for_restart_expiry_and_profile_revoke() {
    for cause in ["restart", "expiry", "profile_revoke"] {
        for dispatched in [false, true] {
            let f = Fixture::new(false).await;
            let created = f.ready(0).await;
            let auth = authority(&f, &created.session);
            let operation = new(f
                .repo
                .reserve_event(&auth, 1, 0, &event(EventKind::Started, "0"))
                .await
                .unwrap());
            if dispatched {
                f.repo.before_io(&operation).await.unwrap();
            }
            match cause {
                "restart" => {
                    f.recover_abandoned().await.unwrap();
                }
                "expiry" => {
                    sqlx::query("UPDATE playback.sessions SET lease_until=clock_timestamp()-interval '1 second' WHERE id=$1")
                        .bind(created.session.id).execute(&f.pool).await.unwrap();
                    f.repo.expire().await.unwrap();
                }
                "profile_revoke" => {
                    relink(&f, f.user, "different-profile").await;
                    relink(&f, f.user, "personal-profile").await;
                    f.repo.expire().await.unwrap();
                }
                _ => unreachable!(),
            }
            assert_eq!(
                effect_state(&f, &operation).await,
                if dispatched {
                    ("uncertain".into(), true, "unknown".into(), None)
                } else {
                    (
                        "failed".into(),
                        false,
                        "no_effect".into(),
                        Some("not_dispatched".into()),
                    )
                }
            );
            assert!(
                f.repo.before_io(&operation).await.is_err(),
                "abandoned claims never replay"
            );
            let watched = WatchedRepository::new(f.pool.clone());
            let binding = f.binding().await;
            assert_eq!(
                watched.begin_acquisition(&binding).await.is_err(),
                dispatched
            );
            if dispatched {
                // Another read and completed local cleanup cannot prove that the
                // authorized write will not arrive late.
                assert!(watched.read(f.user, &f.items).await.unwrap().is_empty());
                let cleanup = f.repo.claim_cleanup().await.unwrap().unwrap();
                f.repo.finish_cleanup(&cleanup, true, false).await.unwrap();
                assert_eq!(effect_state(&f, &operation).await.2, "unknown");
                assert!(watched.begin_acquisition(&binding).await.is_err());
            } else {
                let acquisition = watched.begin_acquisition(&binding).await.unwrap();
                watched
                    .publish(&acquisition, &snapshot(false), Utc::now())
                    .await
                    .unwrap();
            }
            f.close().await;
        }
    }
}

#[tokio::test]
#[ignore = "explicit disposable SCREENROOM_PLAYBACK_TEST_DATABASE_URL required"]
async fn dispatch_claim_racing_recovery_cannot_be_classified_unsent_after_authorization() {
    let f = Fixture::new(false).await;
    for _ in 0..4 {
        let created = f.ready(0).await;
        let operation = new(f
            .repo
            .reserve_event(
                &authority(&f, &created.session),
                1,
                0,
                &event(EventKind::Started, "0"),
            )
            .await
            .unwrap());
        let (dispatch, recovery) =
            tokio::join!(f.repo.before_io(&operation), f.recover_abandoned());
        recovery.unwrap();
        let state = effect_state(&f, &operation).await;
        if dispatch.is_ok() {
            assert_eq!(state, ("uncertain".into(), true, "unknown".into(), None));
            f.repo
                .resolve_effect(operation.id, CompletionEvidence::ControlledQuiescence)
                .await
                .unwrap();
        } else {
            assert_eq!(
                state,
                (
                    "failed".into(),
                    false,
                    "no_effect".into(),
                    Some("not_dispatched".into())
                )
            );
        }
        assert!(f.repo.before_io(&operation).await.is_err());
    }
    f.close().await;
}

#[tokio::test]
#[ignore = "explicit disposable SCREENROOM_PLAYBACK_TEST_DATABASE_URL required"]
async fn shared_remote_profile_serializes_two_users_effects_and_fences_concurrent_snapshots() {
    let f = Fixture::new(false).await;
    let alias = alias_user(&f, "personal-profile").await;
    let first = f.ready(0).await;
    let second = f
        .repo
        .create(Create {
            user_id: alias,
            public_item_id: "owned-item-1",
            scope: &scope(),
            request_id: Uuid::new_v4(),
            normalized_body: &json!({"item":1}),
            capabilities: &json!({}),
            control_hash: &secrets::capability_hash("alias-control"),
        })
        .await
        .unwrap();
    f.repo.before_io(&second.operation).await.unwrap();
    f.repo
        .finish(&second.operation, Outcome::Confirmed(&json!({})), "ready")
        .await
        .unwrap();
    let watched = WatchedRepository::new(f.pool.clone());
    let bindings = [f.binding().await, binding_for(&f, alias).await];
    let captures = [
        watched.begin_acquisition(&bindings[0]).await.unwrap(),
        watched.begin_acquisition(&bindings[1]).await.unwrap(),
    ];
    let old_snapshot = snapshot(false);
    for capture in &captures {
        watched
            .publish(capture, &old_snapshot, Utc::now())
            .await
            .unwrap();
    }
    let first_auth = authority(&f, &first.session);
    let second_auth = Authority {
        user_id: alias,
        session_id: second.session.id,
        control_capability: "alias-control",
    };
    let started = event(EventKind::Started, "0");
    let (a, b, acquisition, publication) = tokio::join!(
        f.repo.reserve_event(&first_auth, 1, 0, &started),
        f.repo.reserve_event(&second_auth, 1, 0, &started),
        watched.begin_acquisition(&bindings[1]),
        watched.publish(&captures[1], &old_snapshot, Utc::now()),
    );
    let (winner, loser) = match (a, b) {
        (Ok(winner), Err(loser)) | (Err(loser), Ok(winner)) => (winner, loser),
        _ => panic!("one remote profile permits only one pending personal effect"),
    };
    assert!(
        matches!(
            loser,
            screenroom_backend::Error::Invalid("personal_state_unknown")
        ),
        "reservation loser must be the freshness guard, not a database error: {loser}"
    );
    let operation = new(winner);
    assert!(watched.read(f.user, &f.items).await.unwrap().is_empty());
    assert!(watched.read(alias, &f.items).await.unwrap().is_empty());
    if let Ok(acquisition) = acquisition {
        assert!(
            watched
                .publish(&acquisition, &old_snapshot, Utc::now())
                .await
                .is_err()
        );
    }
    // Publication may precede reservation, but its captured epoch cannot survive it.
    let _ = publication;
    f.repo.before_io(&operation).await.unwrap();
    let completion = json!({});
    let (finished, acquisition, publication) = tokio::join!(
        f.repo
            .finish(&operation, Outcome::Confirmed(&completion), "playing"),
        watched.begin_acquisition(&bindings[1]),
        watched.publish(&captures[1], &old_snapshot, Utc::now()),
    );
    finished.unwrap();
    assert!(publication.is_err());
    if let Ok(acquisition) = acquisition {
        assert!(acquisition.epoch > captures[1].epoch);
        watched
            .publish(&acquisition, &snapshot(true), Utc::now())
            .await
            .unwrap();
    }
    for (binding, before) in bindings.iter().zip(&captures) {
        assert!(
            watched
                .publish(before, &old_snapshot, Utc::now())
                .await
                .is_err()
        );
        let after = watched.begin_acquisition(binding).await.unwrap();
        assert!(after.epoch > before.epoch);
        watched
            .publish(&after, &snapshot(true), Utc::now())
            .await
            .unwrap();
    }
    f.close().await;
}

#[tokio::test]
#[ignore = "explicit disposable SCREENROOM_PLAYBACK_TEST_DATABASE_URL required"]
async fn shared_profile_link_changes_racing_epoch_reservation_and_completion_fence_old_acquisitions()
 {
    let f = Fixture::new(false).await;
    let alias = alias_user(&f, "personal-profile").await;
    let watched = WatchedRepository::new(f.pool.clone());
    let old_binding = binding_for(&f, alias).await;
    let old = watched.begin_acquisition(&old_binding).await.unwrap();
    let old_snapshot = snapshot(false);
    watched
        .publish(&old, &old_snapshot, Utc::now())
        .await
        .unwrap();
    let created = f.ready(0).await;
    let auth = authority(&f, &created.session);
    let started = event(EventKind::Started, "0");
    let (reservation, ()) = tokio::join!(
        f.repo.reserve_event(&auth, 1, 0, &started),
        relink(&f, alias, "different-profile"),
    );
    let operation = new(reservation.unwrap());
    assert!(
        watched
            .publish(&old, &old_snapshot, Utc::now())
            .await
            .is_err()
    );
    let different = watched
        .begin_acquisition(&binding_for(&f, alias).await)
        .await
        .unwrap();
    f.repo.before_io(&operation).await.unwrap();
    let completion = json!({});
    let (finished, ()) = tokio::join!(
        f.repo
            .finish(&operation, Outcome::Confirmed(&completion), "playing"),
        relink(&f, alias, "personal-profile"),
    );
    finished.unwrap();
    assert!(
        watched
            .publish(&different, &old_snapshot, Utc::now())
            .await
            .is_err()
    );
    assert!(
        watched
            .publish(&old, &old_snapshot, Utc::now())
            .await
            .is_err()
    );
    let current = watched
        .begin_acquisition(&binding_for(&f, alias).await)
        .await
        .unwrap();
    watched
        .publish(&current, &snapshot(true), Utc::now())
        .await
        .unwrap();
    assert_eq!(
        watched
            .read(alias, &f.items)
            .await
            .unwrap()
            .get(&f.items[0]),
        Some(&true)
    );
    f.close().await;
}

#[tokio::test]
#[ignore = "explicit disposable SCREENROOM_PLAYBACK_TEST_DATABASE_URL required"]
async fn fresh_and_upgrade_preserve_data_and_deny_unrelated_writes() {
    for upgrade in [false, true] {
        let f = Fixture::new(upgrade).await;
        assert_eq!(
            f.repo
                .resolve(f.user, "owned-item-0", &scope())
                .await
                .unwrap()
                .external_item_id,
            "remote-0"
        );
        assert!(
            sqlx::query("UPDATE catalog.sources SET state='disabled'")
                .execute(&f.runtime)
                .await
                .is_err()
        );
        assert!(
            sqlx::query("UPDATE identity.users SET updated_at=now()")
                .execute(&f.runtime)
                .await
                .is_err()
        );
        assert!(
            sqlx::query("UPDATE connections.jellyfin SET generation=generation+1")
                .execute(&f.runtime)
                .await
                .is_err()
        );
        assert!(
            sqlx::query("INSERT INTO ingest.runs DEFAULT VALUES")
                .execute(&f.runtime)
                .await
                .is_err()
        );
        assert!(
            sqlx::query("SELECT indexing_profile_id FROM connections.jellyfin")
                .fetch_all(&f.runtime)
                .await
                .is_err()
        );
        let can_login: bool = sqlx::query_scalar(
            "SELECT rolcanlogin FROM pg_roles WHERE rolname='screenroom_playback'",
        )
        .fetch_one(&f.pool)
        .await
        .unwrap();
        assert!(!can_login);
        if upgrade {
            assert_eq!(
                WatchedRepository::new(f.pool.clone())
                    .read(f.user, &f.items)
                    .await
                    .unwrap()
                    .get(&f.items[0]),
                Some(&true)
            );
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM catalog.source_items")
                .fetch_one(&f.pool)
                .await
                .unwrap();
            assert_eq!(count, 4);
        }
        f.close().await;
    }
}

#[tokio::test]
#[ignore = "explicit disposable SCREENROOM_PLAYBACK_TEST_DATABASE_URL required"]
async fn exact_scope_membership_kind_profile_and_user_fences() {
    let f = Fixture::new(false).await;
    assert!(
        f.repo
            .resolve(f.user, "owned-item-3", &scope())
            .await
            .is_err()
    );
    let mut wrong = scope();
    wrong.library_ids = vec!["other".into()];
    assert!(
        f.repo
            .resolve(f.user, "owned-item-0", &wrong)
            .await
            .is_err()
    );
    let created = f.ready(0).await;
    let auth = authority(&f, &created.session);
    sqlx::query(
        "UPDATE identity.users SET updated_at=updated_at+interval '1 microsecond' WHERE id=$1",
    )
    .bind(f.user)
    .execute(&f.pool)
    .await
    .unwrap();
    assert!(
        f.repo
            .reserve_event(&auth, 1, 0, &event(EventKind::Started, "0"))
            .await
            .is_err()
    );
    let revoked: Option<Vec<u8>> =
        sqlx::query_scalar("SELECT media_hash FROM playback.sessions WHERE id=$1")
            .bind(created.session.id)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert!(revoked.is_none());
    sqlx::query(
        "UPDATE identity.jellyfin_user_links SET jellyfin_user_id='not-personal' WHERE user_id=$1",
    )
    .bind(f.user)
    .execute(&f.pool)
    .await
    .unwrap();
    assert_eq!(
        f.repo
            .resolve(f.user, "owned-item-1", &scope())
            .await
            .unwrap()
            .remote_profile_id,
        "not-personal",
        "an explicit personal selection may coincide with the indexing profile"
    );
    sqlx::query("DELETE FROM identity.jellyfin_user_links WHERE user_id=$1")
        .bind(f.user)
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(
        f.repo
            .resolve(f.user, "owned-item-1", &scope())
            .await
            .is_err(),
        "indexing profile is never personal fallback"
    );
    f.close().await;
}

#[tokio::test]
#[ignore = "explicit disposable SCREENROOM_PLAYBACK_TEST_DATABASE_URL required"]
async fn concurrent_capacity_duplicate_and_request_receipt_conflicts() {
    let f = Fixture::new(false).await;
    let body = json!({"item":0});
    let request = Uuid::new_v4();
    let (a, b) = tokio::join!(f.create(0, request, &body), f.create(0, request, &body));
    let a = a.unwrap();
    let b = b.unwrap();
    assert_eq!(a.session.id, b.session.id);
    assert_ne!(a.replay, b.replay);
    assert!(f.create(0, request, &json!({"item":1})).await.is_err());
    assert!(f.create(0, Uuid::new_v4(), &body).await.is_err());
    let (a, b) = tokio::join!(
        f.create(1, Uuid::new_v4(), &body),
        f.create(2, Uuid::new_v4(), &body)
    );
    assert_ne!(
        a.is_ok(),
        b.is_ok(),
        "profile capacity reservation is atomic"
    );
    f.close().await;
}

#[tokio::test]
#[ignore = "explicit disposable SCREENROOM_PLAYBACK_TEST_DATABASE_URL required"]
async fn ordered_receipts_backward_seek_and_uncertain_start_never_replays() {
    let f = Fixture::new(false).await;
    let created = f.ready(0).await;
    let auth = authority(&f, &created.session);
    let first = event(EventKind::Started, "10000");
    let op = new(f.repo.reserve_event(&auth, 1, 0, &first).await.unwrap());
    f.repo.before_io(&op).await.unwrap();
    assert!(
        f.repo.before_io(&op).await.is_err(),
        "claim dispatches once"
    );
    assert!(
        f.repo.reserve_event(&auth, 2, 0, &first).await.is_err(),
        "pending effect serializes reports"
    );
    f.repo
        .finish(
            &op,
            Outcome::Confirmed(&json!({"confirmed":true})),
            "playing",
        )
        .await
        .unwrap();
    assert!(matches!(
        f.repo.reserve_event(&auth, 1, 0, &first).await.unwrap(),
        Reservation::Replay(_)
    ));
    assert!(
        f.repo
            .reserve_event(&auth, 1, 0, &event(EventKind::Started, "5"))
            .await
            .is_err()
    );
    assert!(f.repo.reserve_event(&auth, 2, 1, &first).await.is_err());
    let seek = new(f
        .repo
        .reserve_event(&auth, 2, 0, &event(EventKind::Seeked, "100"))
        .await
        .unwrap());
    f.repo.before_io(&seek).await.unwrap();
    f.repo
        .finish(&seek, Outcome::Confirmed(&json!({})), "playing")
        .await
        .unwrap();
    let uncertain = new(f
        .repo
        .reserve_event(&auth, 3, 0, &event(EventKind::Started, "100"))
        .await
        .unwrap());
    f.repo.before_io(&uncertain).await.unwrap();
    f.repo
        .finish(&uncertain, Outcome::Uncertain, "failed")
        .await
        .unwrap();
    let replay = f
        .repo
        .reserve_event(&auth, 3, 0, &event(EventKind::Started, "100"))
        .await
        .unwrap();
    match replay {
        Reservation::Replay(op) => assert_eq!(op.status, "uncertain"),
        _ => panic!("uncertain operation must not dispatch"),
    }
    assert!(f.repo.before_io(&uncertain).await.is_err());
    assert!(
        f.repo
            .reserve_event(&auth, 4, 0, &event(EventKind::Progress, "200"))
            .await
            .is_err()
    );
    f.close().await;
}

#[tokio::test]
#[ignore = "explicit disposable SCREENROOM_PLAYBACK_TEST_DATABASE_URL required"]
async fn acquisition_before_during_after_and_late_timed_out_effect() {
    let f = Fixture::new(false).await;
    let watched = WatchedRepository::new(f.pool.clone());
    let binding = f.binding().await;
    let before = watched.begin_acquisition(&binding).await.unwrap();
    watched
        .publish(&before, &snapshot(false), Utc::now())
        .await
        .unwrap();
    let created = f.ready(0).await;
    let auth = authority(&f, &created.session);
    let op = new(f
        .repo
        .reserve_event(&auth, 1, 0, &event(EventKind::Started, "0"))
        .await
        .unwrap());
    assert!(watched.begin_acquisition(&binding).await.is_err());
    assert!(
        watched
            .publish(&before, &snapshot(false), Utc::now())
            .await
            .is_err()
    );
    assert!(watched.read(f.user, &f.items).await.unwrap().is_empty());
    f.repo.before_io(&op).await.unwrap();
    f.repo
        .finish(&op, Outcome::Uncertain, "failed")
        .await
        .unwrap();
    let remote_state = std::sync::atomic::AtomicBool::new(false);
    // Reconciliation reads and arbitrarily old local deadlines do not establish
    // remote completion. Simulate a provider write landing after all these reads.
    for _ in 0..5 {
        assert!(!remote_state.load(std::sync::atomic::Ordering::SeqCst));
        assert!(watched.read(f.user, &f.items).await.unwrap().is_empty());
        assert!(watched.begin_acquisition(&binding).await.is_err());
    }
    remote_state.store(true, std::sync::atomic::Ordering::SeqCst);
    sqlx::query("UPDATE playback.sessions SET lease_until=clock_timestamp()-interval '1 day'")
        .execute(&f.pool)
        .await
        .unwrap();
    f.repo.expire().await.unwrap();
    f.recover_abandoned().await.unwrap();
    assert!(
        watched
            .publish(&before, &snapshot(false), Utc::now())
            .await
            .is_err()
    );
    assert!(
        watched.begin_acquisition(&binding).await.is_err(),
        "late timed-out write remains fenced"
    );
    // Only explicit completion evidence now permits a new acquisition.
    f.repo
        .resolve_effect(op.id, CompletionEvidence::RemoteCompletion)
        .await
        .unwrap();
    assert!(
        watched
            .publish(&before, &snapshot(false), Utc::now())
            .await
            .is_err()
    );
    let after = watched.begin_acquisition(&binding).await.unwrap();
    assert!(after.epoch > before.epoch);
    watched
        .publish(
            &after,
            &snapshot(remote_state.load(std::sync::atomic::Ordering::SeqCst)),
            Utc::now(),
        )
        .await
        .unwrap();
    assert_eq!(
        watched
            .read(f.user, &f.items)
            .await
            .unwrap()
            .get(&f.items[0]),
        Some(&true)
    );
    f.close().await;
}

#[tokio::test]
#[ignore = "explicit disposable SCREENROOM_PLAYBACK_TEST_DATABASE_URL required"]
async fn uncertain_remote_identity_survives_unlink_relink_and_binding_revision() {
    let f = Fixture::new(false).await;
    let created = f.ready(0).await;
    let auth = authority(&f, &created.session);
    let op = new(f
        .repo
        .reserve_event(&auth, 1, 0, &event(EventKind::Started, "0"))
        .await
        .unwrap());
    f.repo.before_io(&op).await.unwrap();
    f.repo
        .finish(&op, Outcome::Uncertain, "failed")
        .await
        .unwrap();
    let watched = WatchedRepository::new(f.pool.clone());
    for profile in ["different-profile", "personal-profile"] {
        let mut tx = f.pool.begin().await.unwrap();
        sqlx::query("SELECT id FROM identity.users WHERE id=$1 FOR UPDATE")
            .bind(f.user)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        sqlx::query("UPDATE identity.jellyfin_user_links SET jellyfin_user_id=$2 WHERE user_id=$1")
            .bind(f.user)
            .bind(profile)
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("UPDATE identity.users SET updated_at=GREATEST(clock_timestamp(),updated_at+interval '1 microsecond') WHERE id=$1").bind(f.user).execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        let binding = f.binding().await;
        assert_eq!(
            watched.begin_acquisition(&binding).await.is_err(),
            profile == "personal-profile"
        );
    }
    f.repo
        .resolve_effect(op.id, CompletionEvidence::ControlledQuiescence)
        .await
        .unwrap();
    watched.begin_acquisition(&f.binding().await).await.unwrap();
    f.close().await;
}

#[tokio::test]
#[ignore = "explicit disposable SCREENROOM_PLAYBACK_TEST_DATABASE_URL required"]
async fn installation_capacity_lease_absolute_paused_and_generation_fences() {
    let mut f = Fixture::new(false).await;
    f.repo = PlaybackRepository::new(
        f.runtime.clone(),
        Limits {
            per_profile: 8,
            installation: 1,
        },
    )
    .unwrap();
    let body = json!({"owned":true});
    let (a, b) = tokio::join!(
        f.create(0, Uuid::new_v4(), &body),
        f.create(1, Uuid::new_v4(), &body)
    );
    assert_ne!(a.is_ok(), b.is_ok());
    let created = if let Ok(created) = a {
        created
    } else {
        b.unwrap()
    };
    f.repo.before_io(&created.operation).await.unwrap();
    f.repo
        .finish(&created.operation, Outcome::Confirmed(&json!({})), "ready")
        .await
        .unwrap();
    let auth = authority(&f, &created.session);
    let before = f.binding().await.revision;
    f.repo
        .heartbeat(&auth, 1, 0, &event(EventKind::Heartbeat, "0"))
        .await
        .unwrap();
    assert_eq!(
        f.binding().await.revision,
        before,
        "playback never increments profile revision"
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM playback.effects")
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        0
    );
    for column in ["lease_until", "absolute_until", "paused_at"] {
        sqlx::query("UPDATE playback.sessions SET lease_until=clock_timestamp()+interval '90 seconds',absolute_until=clock_timestamp()+interval '12 hours',paused_at=NULL")
            .execute(&f.pool).await.unwrap();
        let age = if column == "paused_at" {
            "31 minutes"
        } else {
            "1 second"
        };
        sqlx::query(&format!(
            "UPDATE playback.sessions SET {column}=clock_timestamp()-interval '{age}'"
        ))
        .execute(&f.pool)
        .await
        .unwrap();
        assert!(
            f.repo
                .reserve_event(&auth, 2, 0, &event(EventKind::Progress, "1"))
                .await
                .is_err()
        );
    }
    sqlx::query("UPDATE playback.sessions SET lease_until=clock_timestamp()+interval '90 seconds',absolute_until=clock_timestamp()+interval '12 hours',paused_at=NULL")
        .execute(&f.pool).await.unwrap();
    sqlx::query("UPDATE catalog.sources SET configuration=jsonb_set(configuration,'{connection_generation}','2') WHERE id=$1")
        .bind(f.source).execute(&f.pool).await.unwrap();
    assert!(
        f.repo
            .resolve(f.user, "owned-item-0", &scope())
            .await
            .is_err()
    );
    assert!(f.repo.before_io(&created.operation).await.is_err());
    f.close().await;
}

#[tokio::test]
#[ignore = "explicit disposable SCREENROOM_PLAYBACK_TEST_DATABASE_URL required"]
async fn lost_authorization_crash_and_restart_resume_exact_cleanup_only() {
    let f = Fixture::new(false).await;
    let created = f
        .create(0, Uuid::new_v4(), &json!({"item":0}))
        .await
        .unwrap();
    let cleanup: (String, Uuid) =
        sqlx::query_as("SELECT device_id,credential_ref FROM playback.cleanup WHERE session_id=$1")
            .bind(created.session.id)
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(
        cleanup,
        (
            created.session.device_id.clone(),
            created.session.credential_ref
        )
    );
    assert!(
        f.repo.claim_cleanup().await.unwrap().is_none(),
        "intent persisted but not active during preparation"
    );
    f.repo.before_io(&created.operation).await.unwrap();
    // Token creation response was lost or process crashed before private publication.
    f.recover_abandoned().await.unwrap();
    assert!(f.repo.before_io(&created.operation).await.is_err());
    let claim = f.repo.claim_cleanup().await.unwrap().unwrap();
    assert_eq!(claim.device_id, created.session.device_id);
    assert!(claim.revoke_device);
    assert!(f.repo.claim_cleanup().await.unwrap().is_none());
    assert_eq!(
        f.repo.cleanup_session(&claim).await.unwrap().id,
        created.session.id
    );
    f.repo.finish_cleanup(&claim, false, true).await.unwrap();
    assert!(
        f.repo.finish_cleanup(&claim, true, false).await.is_err(),
        "old cleanup claims cannot overwrite failures"
    );
    let state: String = sqlx::query_scalar("SELECT state FROM playback.cleanup WHERE id=$1")
        .bind(claim.id)
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(state, "failed", "failure evidence survives retention");
    let counts = f.repo.telemetry_counts().await.unwrap();
    assert_eq!(counts.failed, 1);
    assert_eq!(counts.identity_changed, 1);
    assert_eq!(counts.blocked_device, 0);
    assert!(counts.oldest_failed_seconds >= 0.0);
    assert!(
        f.repo
            .retain(Utc::now() - screenroom_backend::chrono::Duration::days(2))
            .await
            .unwrap()
            .is_empty()
    );
    f.close().await;
}

#[tokio::test]
#[ignore = "explicit disposable SCREENROOM_PLAYBACK_TEST_DATABASE_URL required"]
async fn media_revision_resource_bindings_and_stop_revoke_before_io() {
    let f = Fixture::new(false).await;
    let created = f
        .create(0, Uuid::new_v4(), &json!({"item":0}))
        .await
        .unwrap();
    f.repo.before_io(&created.operation).await.unwrap();
    let resource = Resource {
        id: Uuid::new_v4(),
        session_id: created.session.id,
        transport_revision: 1,
        source_item_id: f.items[0],
        media_source_id: "owned-media-source".into(),
        private_mapping: json!({"path":"private-resource"}),
    };
    let hash = secrets::capability_hash("owned-media");
    let revision = f
        .repo
        .publish_transport(
            &created.operation,
            Transport {
                play_session_id: "owned-play-session",
                media_hash: &hash,
                tracks: &json!([]),
                descriptor: &json!({"method":"direct"}),
                resources: std::slice::from_ref(&resource),
            },
        )
        .await
        .unwrap();
    assert_eq!(revision, 1);
    f.repo
        .finish(&created.operation, Outcome::Confirmed(&json!({})), "ready")
        .await
        .unwrap();
    assert!(
        f.repo
            .media(created.session.id, 1, "owned-control", resource.id)
            .await
            .is_err()
    );
    assert!(
        f.repo
            .media(created.session.id, 0, "owned-media", resource.id)
            .await
            .is_err()
    );
    assert!(
        f.repo
            .media(created.session.id, 1, "owned-media", Uuid::new_v4())
            .await
            .is_err()
    );
    f.repo
        .media(created.session.id, 1, "owned-media", resource.id)
        .await
        .unwrap();
    let auth = authority(&f, &created.session);
    let update = new(f
        .repo
        .reserve(
            &auth,
            Mutation::Update {
                request_id: Uuid::new_v4(),
            },
            1,
            &json!({"audio":"opaque-choice"}),
            false,
        )
        .await
        .unwrap());
    assert_ne!(update.replay_ref, created.operation.replay_ref);
    f.repo.before_io(&update).await.unwrap();
    let next_resource = Resource {
        id: Uuid::new_v4(),
        transport_revision: 2,
        ..resource.clone()
    };
    let next_hash = secrets::capability_hash("next-media");
    f.repo
        .publish_transport(
            &update,
            Transport {
                play_session_id: "next-play-session",
                media_hash: &next_hash,
                tracks: &json!([]),
                descriptor: &json!({"method":"transcode"}),
                resources: std::slice::from_ref(&next_resource),
            },
        )
        .await
        .unwrap();
    assert!(
        f.repo.claim_cleanup().await.unwrap().is_none(),
        "cleanup cannot overtake pending control I/O"
    );
    f.repo
        .finish(&update, Outcome::Confirmed(&json!({})), "ready")
        .await
        .unwrap();
    assert!(
        f.repo
            .media(created.session.id, 1, "owned-media", resource.id)
            .await
            .is_err()
    );
    f.repo
        .media(created.session.id, 2, "next-media", next_resource.id)
        .await
        .unwrap();
    assert_eq!(
        f.repo.get(&auth).await.unwrap().replay_ref,
        update.replay_ref.unwrap()
    );
    let old = f.repo.claim_cleanup().await.unwrap().unwrap();
    assert!(!old.revoke_device);
    assert_eq!(old.play_session_id.as_deref(), Some("owned-play-session"));
    f.repo.finish_cleanup(&old, true, false).await.unwrap();
    let stop = new(f
        .repo
        .reserve(
            &auth,
            Mutation::Stop {
                request_id: Uuid::new_v4(),
                final_sequence: 1,
            },
            2,
            &json!({"position_ms":"100"}),
            true,
        )
        .await
        .unwrap());
    assert!(
        f.repo
            .media(created.session.id, 1, "owned-media", resource.id)
            .await
            .is_err()
    );
    f.repo.before_io(&stop).await.unwrap();
    f.repo
        .finish(&stop, Outcome::Confirmed(&json!({})), "stopped")
        .await
        .unwrap();
    let device = f.repo.claim_cleanup().await.unwrap().unwrap();
    assert!(device.revoke_device);
    assert_eq!(device.play_session_id.as_deref(), Some("next-play-session"));
    f.close().await;
}
