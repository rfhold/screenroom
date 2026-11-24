use crate::{PlaybackService, fences::CleanupFence, records};
use screenroom_backend::{
    config::ServiceDatabaseConfig,
    connections::{ConnectionRepository, CredentialDirectory},
    playback::{
        Cleanup, PlaybackRepository, holds_gateway_lock, secrets::PlaybackSecrets, try_gateway_lock,
    },
};
use screenroom_catalog_adapter::jellyfin::playback as provider;
use std::{
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tonic::codegen::http::HeaderValue;

pub struct RuntimeConfig {
    pub listen: std::net::SocketAddr,
    pub database: ServiceDatabaseConfig,
    pub credential_directory: PathBuf,
    pub playback_directory: PathBuf,
    pub api_origin: String,
    pub origins: Vec<HeaderValue>,
}

pub(crate) struct RuntimeOwnership {
    connection: tokio::sync::Mutex<sqlx::pool::PoolConnection<sqlx::Postgres>>,
    stopping: Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Debug, PartialEq, Eq)]
struct OwnershipFailure {
    cause: &'static str,
    sqlstate: Option<String>,
    wait_ms: u64,
    elapsed_ms: u64,
}

pub(crate) fn elapsed_ms(start: tokio::time::Instant) -> u64 {
    start.elapsed().as_millis().min(60_000) as u64
}

fn bounded_sqlstate(code: Option<&str>) -> Option<String> {
    code.filter(|code| {
        code.len() == 5
            && code
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
    })
    .map(str::to_owned)
}

pub(crate) fn sqlstate(error: &sqlx::Error) -> Option<String> {
    let code = error.as_database_error().and_then(|error| error.code());
    bounded_sqlstate(code.as_deref())
}

pub(crate) fn backend_failure(error: &screenroom_backend::Error) -> (&'static str, Option<String>) {
    use screenroom_backend::Error;
    match error {
        Error::Database(error) => ("database", sqlstate(error)),
        Error::Invalid(_) => ("invalid", None),
        Error::Stale => ("stale", None),
        Error::SourceBusy { .. } => ("source_busy", None),
        Error::NotFound => ("not_found", None),
        Error::Unavailable => ("unavailable", None),
        Error::Incompatible(_) => ("incompatible", None),
        Error::Connection(_) => ("connection", None),
    }
}

async fn ownership_probe<C, W, Q, F>(wait: W, query: Q) -> Result<(), OwnershipFailure>
where
    W: Future<Output = C>,
    Q: FnOnce(C) -> F,
    F: Future<Output = Result<bool, Option<String>>>,
{
    let start = tokio::time::Instant::now();
    let deadline = start + Duration::from_millis(500);
    let connection =
        tokio::time::timeout_at(deadline, wait)
            .await
            .map_err(|_| OwnershipFailure {
                cause: "mutex_wait_timeout",
                sqlstate: None,
                wait_ms: elapsed_ms(start),
                elapsed_ms: elapsed_ms(start),
            })?;
    let wait_ms = elapsed_ms(start);
    let (cause, sqlstate) = match tokio::time::timeout_at(deadline, query(connection)).await {
        Ok(Ok(true)) => return Ok(()),
        Ok(Ok(false)) => ("lock_missing", None),
        Ok(Err(code)) => ("query_error", bounded_sqlstate(code.as_deref())),
        Err(_) => ("query_timeout", None),
    };
    Err(OwnershipFailure {
        cause,
        sqlstate,
        wait_ms,
        elapsed_ms: elapsed_ms(start),
    })
}

async fn check_ownership(
    stopping: &std::sync::atomic::AtomicBool,
    purpose: provider::DispatchPurpose,
    probe: impl Future<Output = Result<(), OwnershipFailure>>,
) -> Result<(), provider::DispatchDenial> {
    if purpose == provider::DispatchPurpose::Active && stopping.load(Ordering::Acquire) {
        return Err(provider::DispatchDenial::Revoked);
    }
    if let Err(failure) = probe.await {
        if failure.cause == "mutex_wait_timeout" {
            return Err(provider::DispatchDenial::Unavailable);
        }
        if !stopping.swap(true, Ordering::AcqRel) {
            tracing::error!(
                component = "playback_ownership",
                stage = "ownership_check",
                cause = failure.cause,
                sqlstate = failure.sqlstate.as_deref().unwrap_or(""),
                wait_ms = failure.wait_ms,
                elapsed_ms = failure.elapsed_ms,
                query_ms = failure.elapsed_ms.saturating_sub(failure.wait_ms),
                "playback runtime ownership unavailable"
            );
        }
        return Err(provider::DispatchDenial::Unavailable);
    }
    Ok(())
}

impl RuntimeOwnership {
    pub(crate) fn new(
        connection: sqlx::pool::PoolConnection<sqlx::Postgres>,
        stopping: Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        Self {
            connection: tokio::sync::Mutex::new(connection),
            stopping,
        }
    }

    #[cfg(test)]
    pub(crate) async fn unlock_for_test(&self) {
        let mut connection = self.connection.lock().await;
        sqlx::query("SELECT pg_advisory_unlock(1935897197,1)")
            .execute(&mut **connection)
            .await
            .unwrap();
    }
    pub(crate) async fn check(
        &self,
        purpose: provider::DispatchPurpose,
    ) -> Result<(), provider::DispatchDenial> {
        check_ownership(&self.stopping, purpose, self.probe()).await
    }

    async fn probe(&self) -> Result<(), OwnershipFailure> {
        ownership_probe(self.connection.lock(), |mut connection| async move {
            holds_gateway_lock(&mut connection)
                .await
                .map_err(|error| sqlstate(&error))
        })
        .await
    }

    async fn watchdog_healthy(&self) -> bool {
        watchdog_healthy(&self.stopping, self.probe()).await
    }

    pub(crate) async fn recover_abandoned(&self) -> screenroom_backend::Result<u64> {
        let mut connection = self.connection.lock().await;
        PlaybackRepository::recover_abandoned(&mut connection).await
    }

    pub(crate) async fn connection(
        &self,
    ) -> screenroom_backend::Result<
        tokio::sync::MutexGuard<'_, sqlx::pool::PoolConnection<sqlx::Postgres>>,
    > {
        tokio::time::timeout(Duration::from_millis(500), self.connection.lock())
            .await
            .map_err(|_| screenroom_backend::Error::Unavailable)
    }
}

async fn watchdog_healthy(
    stopping: &std::sync::atomic::AtomicBool,
    probe: impl Future<Output = Result<(), OwnershipFailure>>,
) -> bool {
    if stopping.load(Ordering::Acquire) {
        return false;
    }
    let outcome = check_ownership(stopping, provider::DispatchPurpose::Active, probe).await;
    outcome != Err(provider::DispatchDenial::Revoked) && !stopping.load(Ordering::Acquire)
}

impl RuntimeConfig {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        use screenroom_backend::config::required;
        let listen = std::env::var("SCREENROOM_PLAYBACK_GATEWAY_LISTEN")
            .unwrap_or_else(|_| "127.0.0.1:50054".into())
            .parse()?;
        let api_origin = required("SCREENROOM_PLAYBACK_API_ORIGIN")?;
        crate::transport::validate_origin(&api_origin)?;
        let origins = std::env::var("SCREENROOM_PLAYBACK_GATEWAY_CORS_ORIGINS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::parse::<HeaderValue>)
            .collect::<Result<Vec<_>, _>>()?;
        for origin in &origins {
            crate::transport::validate_origin(origin.to_str()?)?;
        }
        let credential_directory =
            PathBuf::from(required("SCREENROOM_JELLYFIN_CREDENTIAL_DIRECTORY")?);
        let playback_directory = PathBuf::from(required("SCREENROOM_PLAYBACK_SECRET_DIRECTORY")?);
        if credential_directory.starts_with(&playback_directory)
            || playback_directory.starts_with(&credential_directory)
        {
            return Err("separate managed and playback secret directories required".into());
        }
        Ok(Self {
            listen,
            database: ServiceDatabaseConfig::from_env()?,
            credential_directory,
            playback_directory,
            api_origin,
            origins,
        })
    }
}

impl PlaybackService {
    pub(crate) async fn cleanup(&self, job: &Cleanup) -> Result<(), bool> {
        let session = self
            .repository
            .cleanup_session(job)
            .await
            .map_err(|e| matches!(e, screenroom_backend::Error::Stale))?;
        let (live, server) = self
            .repository
            .provider_configuration(&session, &self.credentials)
            .await
            .map_err(|e| matches!(e, screenroom_backend::Error::Stale))?;
        let intent = provider::AuthorizationIntent::new(
            provider::GatewayDeviceId::from_recorded(job.device_id.clone()).map_err(|_| true)?,
            server.clone(),
            session.remote_profile_id,
            "",
        )
        .map_err(|_| true)?;
        let fence = Arc::new(CleanupFence {
            repository: self.repository.clone(),
            job: job.clone(),
            server,
            ownership: self.ownership.clone(),
        });
        let adapter = live
            .playback_adapter_fenced(intent.clone(), provider::PlaybackTimeouts::default(), fence)
            .map_err(|_| false)?;
        let token = self
            .secrets
            .read(job.credential_ref)
            .ok()
            .and_then(|token| provider::SessionToken::from_private_store(token).ok());
        let auth = if let Some(token) = token {
            adapter
                .restore_session_for_cleanup(intent.clone(), token)
                .await
                .ok()
        } else {
            None
        };
        let result = async {
            if let Some(play_session) = &job.play_session_id {
                let encoding = provider::EncodingIdentity::from_recorded(
                    intent.device().clone(),
                    play_session.clone(),
                )?;
                if let Some(auth) = &auth {
                    adapter.stop_recorded_encoding(auth, &encoding).await?;
                } else {
                    adapter.recover_encoding(&intent, &encoding).await?;
                }
            }
            if job.revoke_device {
                // Lost authentication responses have no token file. Exact-device deletion
                // is the recovery path; neither profile-wide logout nor report replay is used.
                adapter.recover_device(&intent).await?;
            }
            Ok::<_, provider::PlaybackError>(())
        }
        .await;
        result.map_err(|error| error.failure == provider::Failure::Identity)
    }

    pub(crate) async fn cleanup_once(&self) {
        self.cleanup_batch(8).await;
    }

    async fn cleanup_batch(&self, limit: usize) {
        for _ in 0..limit {
            let claim = match &self.ownership {
                Some(owner) => match owner.connection().await {
                    Ok(mut connection) => {
                        PlaybackRepository::claim_cleanup_owned(&mut connection).await
                    }
                    Err(error) => Err(error),
                },
                None => self.repository.claim_cleanup().await,
            };
            let job = match claim {
                Ok(Some(job)) => job,
                Ok(None) => break,
                Err(_) => {
                    tracing::warn!(
                        error_code = "cleanup_claim_failed",
                        "playback cleanup unavailable"
                    );
                    break;
                }
            };
            let outcome = tokio::time::timeout(Duration::from_secs(10), self.cleanup(&job)).await;
            let (success, identity_changed) = match outcome {
                Ok(Ok(())) => (true, false),
                Ok(Err(identity)) => (false, identity),
                Err(_) => (false, false),
            };
            opentelemetry::global::meter("screenroom-playback-gateway")
                .u64_counter("playback.cleanup.attempts")
                .build()
                .add(
                    1,
                    &[opentelemetry::KeyValue::new(
                        "outcome",
                        if success {
                            "complete"
                        } else if identity_changed {
                            "identity_changed"
                        } else {
                            "failed"
                        },
                    )],
                );
            if self
                .repository
                .finish_cleanup(&job, success, identity_changed)
                .await
                .is_err()
            {
                tracing::warn!(
                    error_code = "cleanup_finalize_failed",
                    "playback cleanup unavailable"
                );
            } else if !success {
                tracing::warn!(
                    error_code = if identity_changed {
                        "cleanup_identity_changed"
                    } else {
                        "cleanup_failed"
                    },
                    "playback cleanup failed"
                );
            }
        }
    }

    pub(crate) async fn maintain(&self) {
        let _ = self.repository.expire().await;
        let transports: Vec<_> = self.transports.read().await.keys().copied().collect();
        for (id, revision) in transports {
            if matches!(
                self.repository.transport_retired(id, revision).await,
                Ok(true)
            ) {
                self.transports.write().await.remove(&(id, revision));
            }
        }
        self.locks
            .lock()
            .await
            .retain(|_, lock| lock.strong_count() > 0);
        let meter = opentelemetry::global::meter("screenroom-playback-gateway");
        if let Ok(counts) = self.repository.telemetry_counts().await {
            meter
                .u64_gauge("playback.sessions.active")
                .build()
                .record(counts.active as u64, &[]);
            let backlog = meter.u64_gauge("playback.cleanup.backlog").build();
            for (state, value) in [
                ("pending", counts.pending),
                ("claimed", counts.claimed),
                ("failed", counts.failed),
                ("device_blocked_by_failed_encoding", counts.blocked_device),
            ] {
                backlog.record(
                    value as u64,
                    &[opentelemetry::KeyValue::new("state", state)],
                );
            }
            let failures = meter.u64_gauge("playback.cleanup.failures").build();
            for (reason, value) in [
                ("identity_changed", counts.identity_changed),
                ("attempts_exhausted", counts.attempts_exhausted),
                ("upstream_unavailable", counts.upstream_unavailable),
            ] {
                failures.record(
                    value as u64,
                    &[opentelemetry::KeyValue::new("reason", reason)],
                );
            }
            meter
                .f64_gauge("playback.cleanup.oldest_failed_age")
                .with_unit("s")
                .build()
                .record(counts.oldest_failed_seconds.max(0.0), &[]);
        }
        let transports = self.transports.read().await;
        let methods = meter.u64_gauge("playback.transports.active").build();
        for (method, name) in [
            (provider::PlaybackMethod::DirectPlay, "direct_play"),
            (provider::PlaybackMethod::DirectStream, "direct_stream"),
            (provider::PlaybackMethod::Transcode, "transcode"),
        ] {
            methods.record(
                transports
                    .values()
                    .filter(|t| t.negotiated.method == method)
                    .count() as u64,
                &[opentelemetry::KeyValue::new("method", name)],
            );
        }
        drop(transports);
        let _ = self
            .repository
            .retain(chrono::Utc::now() - chrono::Duration::days(1))
            .await;
        if let Ok(references) = self.repository.retired_credentials().await {
            for proof in references {
                if self.secrets.remove_retired(&proof).is_ok() {
                    let _ = self.repository.finish_credential_gc(&proof).await;
                }
            }
        }
    }
}

pub async fn run(
    config: RuntimeConfig,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), Box<dyn std::error::Error>> {
    // This function is only entered by the executable, never by isolated tests.
    crate::transport::validate_origin(&config.api_origin)?;
    for origin in &config.origins {
        crate::transport::validate_origin(origin.to_str()?)?;
    }
    if config
        .credential_directory
        .starts_with(&config.playback_directory)
        || config
            .playback_directory
            .starts_with(&config.credential_directory)
    {
        return Err("separate managed and playback secret directories required".into());
    }
    let credentials = CredentialDirectory::new(config.credential_directory)?;
    let secrets = PlaybackSecrets::new(config.playback_directory)?;
    let pool = screenroom_backend::db::connect_service(&config.database).await?;
    let restricted:bool=sqlx::query_scalar("SELECT pg_has_role(current_user,'screenroom_playback','USAGE') AND NOT rolsuper AND NOT rolcreatedb AND NOT rolcreaterole AND NOT rolbypassrls AND NOT has_database_privilege(current_user,current_database(),'CREATE') AND NOT EXISTS(SELECT FROM pg_namespace n WHERE n.nspname NOT LIKE 'pg_%' AND n.nspname<>'information_schema' AND has_schema_privilege(current_user,n.oid,'CREATE')) AND NOT EXISTS(SELECT FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE c.relkind IN ('r','p') AND n.nspname NOT LIKE 'pg_%' AND n.nspname NOT IN ('information_schema','playback') AND has_table_privilege(current_user,c.oid,'INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER')) FROM pg_roles WHERE rolname=current_user")
        .fetch_one(&pool).await?;
    if !restricted {
        return Err("restricted playback database role required".into());
    }
    let mut ownership = pool.acquire().await?;
    ownership.close_on_drop();
    let exclusive = try_gateway_lock(&mut ownership).await?;
    if !exclusive {
        return Err("playback gateway already active".into());
    }
    let repository = PlaybackRepository::new(pool.clone(), Default::default())?;
    let connections = ConnectionRepository::new(pool.clone(), Some(credentials.clone()));
    let mut service = PlaybackService::new(
        repository,
        connections,
        credentials,
        secrets,
        config.api_origin,
    )?;
    let ownership = Arc::new(RuntimeOwnership::new(ownership, service.stopping.clone()));
    service.ownership = Some(ownership.clone());
    // Ownership is held before cleanup-only recovery and before binding/readiness.
    ownership.recover_abandoned().await?;
    let listener = tokio::net::TcpListener::bind(config.listen).await?;
    let (stop, mut stopped) = tokio::sync::watch::channel(false);
    let mut cleanup_stopped = stop.subscribe();
    let worker_service = service.clone();
    let worker = tokio::spawn(async move {
        loop {
            tokio::select! {
                _=stopped.changed()=>break,
                _=tokio::time::sleep(Duration::from_secs(2))=>worker_service.maintain().await,
            }
        }
    });
    let cleanup_service = service.clone();
    let cleanup_worker = tokio::spawn(async move {
        loop {
            tokio::select! {
                _=cleanup_stopped.changed()=>break,
                _=tokio::time::sleep(Duration::from_secs(1))=>cleanup_service.cleanup_batch(1).await,
            }
        }
    });
    let gateway = service.clone();
    let monitor = ownership.clone();
    let mut signal = stop.subscribe();
    let termination = async move {
        tokio::pin!(shutdown);
        let reason = loop {
            tokio::select! {
                _=&mut shutdown=>break "shutdown_signal",
                _=tokio::time::sleep(Duration::from_secs(1))=>{
                    if !monitor.watchdog_healthy().await {break "ownership_watchdog";}
                }
            }
        };
        tracing::info!(
            component = "playback_runtime",
            stage = "shutdown",
            reason,
            "playback shutdown initiated"
        );
        gateway.owned_tasks.close().await;
        gateway.stopping.store(true, Ordering::Release);
        let _ = stop.send(true);
    };
    // The stop watcher permits graceful HTTP drain without releasing the advisory lock early.
    let mut termination = Box::pin(termination);
    let server = crate::serve(
        service.clone(),
        listener,
        async move {
            let _ = signal.changed().await;
        },
        config.origins,
    );
    tokio::pin!(server);
    let result = tokio::select! {
        result=&mut server=>Some(result),
        _=&mut termination=>None,
    };
    drop(termination);
    service.owned_tasks.close().await;
    service.stopping.store(true, Ordering::Release);
    worker.abort();
    let _ = worker.await;
    cleanup_worker.abort();
    let _ = cleanup_worker.await;
    let result = if let Some(result) = result {
        result
    } else {
        tokio::time::timeout(Duration::from_secs(10), &mut server)
            .await
            .unwrap_or(Ok(()))
    };
    // Every admitted mutation is quiescent locally before cleanup can begin.
    // Cancellation says nothing about remote completion: classify from durable claims.
    let classified = service
        .owned_tasks
        .prepare_cleanup(Duration::from_secs(30), |forced| async {
            for slot in forced {
                let operation = slot.lock().map_err(|_| records::unavailable())?.clone();
                if let Some(operation) = operation {
                    service
                        .repository
                        .finalize_abandoned(&operation)
                        .await
                        .map_err(records::database)?;
                }
            }
            ownership
                .recover_abandoned()
                .await
                .map_err(records::database)?;
            Ok::<_, tonic::Status>(())
        })
        .await;
    let classification_failed = classified.is_err();
    if !classification_failed {
        let _ = tokio::time::timeout(Duration::from_secs(10), service.cleanup_once()).await;
    }
    drop(ownership);
    let _ = tokio::time::timeout(Duration::from_secs(1), pool.close()).await;
    if classification_failed {
        return Err("playback shutdown classification unavailable; cleanup deferred".into());
    }
    result.map_err(|_| records::unavailable())?;
    Ok(())
}

#[cfg(test)]
mod ownership_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize};

    #[tokio::test]
    async fn watchdog_stops_after_ownership_failure_even_if_next_probe_could_succeed() {
        let stopping = AtomicBool::new(false);
        assert!(
            !watchdog_healthy(&stopping, async {
                stopping.store(true, Ordering::Release);
                Ok(())
            })
            .await
        );
        assert!(
            !watchdog_healthy(&stopping, async {
                panic!("latched shutdown must not probe ownership")
            })
            .await
        );
    }

    #[tokio::test]
    async fn ownership_wait_and_query_timeouts_are_distinct_and_share_500ms() {
        let query_calls = AtomicUsize::new(0);
        let waited = ownership_probe(std::future::pending::<()>(), |()| async {
            query_calls.fetch_add(1, Ordering::Relaxed);
            Ok(true)
        })
        .await
        .unwrap_err();
        assert_eq!(waited.cause, "mutex_wait_timeout");
        assert_eq!(query_calls.load(Ordering::Relaxed), 0);

        let queried = ownership_probe(
            async { tokio::time::sleep(Duration::from_millis(300)).await },
            |()| async {
                tokio::time::sleep(Duration::from_millis(300)).await;
                Ok(true)
            },
        )
        .await
        .unwrap_err();
        assert_eq!(queried.cause, "query_timeout");
        assert!(queried.wait_ms >= 300);
        assert!(queried.elapsed_ms >= 500);
    }

    #[tokio::test]
    async fn ownership_query_error_missing_lock_and_success_are_distinct() {
        let failed = ownership_probe(async {}, |()| async { Err(Some("08006".into())) })
            .await
            .unwrap_err();
        assert_eq!(failed.cause, "query_error");
        assert_eq!(failed.sqlstate.as_deref(), Some("08006"));
        let missing = ownership_probe(async {}, |()| async { Ok(false) })
            .await
            .unwrap_err();
        assert_eq!(missing.cause, "lock_missing");
        assert!(missing.sqlstate.is_none());
        assert!(
            ownership_probe(async {}, |()| async { Ok(true) })
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn local_ownership_contention_denies_dispatch_without_latching_shutdown() {
        let stopping = AtomicBool::new(false);
        let result = check_ownership(
            &stopping,
            provider::DispatchPurpose::Active,
            std::future::ready(Err(OwnershipFailure {
                cause: "mutex_wait_timeout",
                sqlstate: None,
                wait_ms: 500,
                elapsed_ms: 500,
            })),
        )
        .await;
        assert_eq!(result, Err(provider::DispatchDenial::Unavailable));
        assert!(!stopping.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn ownership_failure_latches_stopping_and_never_polls_later_active_dispatch() {
        let capture = diagnostic_capture::Capture::default();
        let _subscriber = tracing::subscriber::set_default(capture.clone());
        let stopping = AtomicBool::new(false);
        let purpose = provider::DispatchPurpose::Active;
        assert_eq!(
            check_ownership(
                &stopping,
                purpose,
                ownership_probe(async {}, |()| async { Ok(false) }),
            )
            .await,
            Err(provider::DispatchDenial::Unavailable)
        );
        assert!(stopping.load(Ordering::Acquire));
        assert_eq!(
            check_ownership(&stopping, purpose, async {
                panic!("stopped runtime must not poll dispatch ownership probe");
            })
            .await,
            Err(provider::DispatchDenial::Revoked)
        );
        assert_eq!(
            check_ownership(
                &stopping,
                provider::DispatchPurpose::Cleanup,
                ownership_probe(async {}, |()| async { Ok(true) }),
            )
            .await,
            Ok(())
        );
        assert!(stopping.load(Ordering::Acquire));
        assert_eq!(
            check_ownership(
                &stopping,
                provider::DispatchPurpose::Cleanup,
                ownership_probe(async {}, |()| async { Ok(false) }),
            )
            .await,
            Err(provider::DispatchDenial::Unavailable)
        );
        let events = capture.0.lock().unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].contains("lock_missing"));
        assert!(events[0].contains("playback_ownership"));
    }

    #[test]
    fn ownership_and_backend_diagnostics_never_include_private_error_text() {
        for private in ["secret-token", "08006 private-query", "abcde", "123456"] {
            assert_eq!(bounded_sqlstate(Some(private)), None);
        }
        assert_eq!(bounded_sqlstate(Some("XX000")).as_deref(), Some("XX000"));
        let private = "private-provider-url/token?cap=private-capability";
        let errors = [
            screenroom_backend::Error::Invalid(private),
            screenroom_backend::Error::Connection(private),
            screenroom_backend::Error::Database(sqlx::Error::Protocol(private.into())),
        ];
        for error in errors {
            let diagnostic = format!("{:?}", backend_failure(&error));
            assert!(!diagnostic.contains(private));
            assert!(!diagnostic.contains("private"));
        }
    }
}

#[cfg(test)]
pub(crate) mod diagnostic_capture {
    use std::sync::{Arc, Mutex};
    use tracing::{Event, Metadata, Subscriber, field::Visit, span};

    #[derive(Clone, Default)]
    pub(crate) struct Capture(pub Arc<Mutex<Vec<String>>>);

    struct Fields(String);

    impl Visit for Fields {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
            use std::fmt::Write;
            write!(&mut self.0, "{}={value:?};", field.name()).unwrap();
        }
    }

    impl Subscriber for Capture {
        fn enabled(&self, _: &Metadata<'_>) -> bool {
            true
        }
        fn new_span(&self, _: &span::Attributes<'_>) -> span::Id {
            span::Id::from_u64(1)
        }
        fn record(&self, _: &span::Id, _: &span::Record<'_>) {}
        fn record_follows_from(&self, _: &span::Id, _: &span::Id) {}
        fn enter(&self, _: &span::Id) {}
        fn exit(&self, _: &span::Id) {}
        fn event(&self, event: &Event<'_>) {
            let mut fields = Fields(String::new());
            event.record(&mut fields);
            self.0.lock().unwrap().push(fields.0);
        }
    }
}
