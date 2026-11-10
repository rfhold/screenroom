use screenroom_backend::{
    Uuid,
    config::ServiceDatabaseConfig,
    db,
    embedding::{EmbeddingClient, EmbeddingConfig},
    storage::{ObjectStorage, StorageConfig},
};
use screenroom_catalog_adapter::{Faults, MockAdapters};
use screenroom_index_worker::{Step, Worker};
use screenroom_observability::telemetry::{Telemetry, TelemetryConfig, shutdown_signal};
use std::time::Duration;

#[tokio::main]
async fn main() {
    if run().await.is_err() {
        eprintln!("index worker failed; check configuration and dependency availability");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let telemetry = Telemetry::init("screenroom-index-worker", TelemetryConfig::from_env()?)?;
    let mut pool = None;
    let result = tokio::select! {
        result = operate(&mut pool) => result,
        _ = shutdown_signal() => Ok(()),
    };
    let closed = if let Some(pool) = pool {
        tokio::time::timeout(Duration::from_secs(5), pool.close())
            .await
            .is_ok()
    } else {
        true
    };
    let shutdown = telemetry.shutdown();
    result?;
    shutdown?;
    if !closed {
        return Err("database shutdown timed out".into());
    }
    Ok(())
}

async fn operate(
    pool_slot: &mut Option<screenroom_backend::sqlx::PgPool>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "serve".into());
    if !matches!(
        command.as_str(),
        "serve" | "drain" | "resume" | "activate-model"
    ) {
        return Err("expected serve, drain, resume RUN_UUID, or activate-model".into());
    }
    let run_id = if command == "resume" {
        Some(args.next().ok_or("missing run UUID")?.parse::<Uuid>()?)
    } else {
        None
    };
    if args.next().is_some() {
        return Err("unexpected argument".into());
    }
    let root =
        std::env::var("SCREENROOM_FIXTURE_ROOT").unwrap_or_else(|_| "fixtures/catalog".into());
    let limit = std::env::var("SCREENROOM_WORKER_DRAIN_LIMIT")
        .unwrap_or_else(|_| "1000".into())
        .parse::<usize>()?;
    if !(1..=100_000).contains(&limit) {
        return Err("invalid drain limit".into());
    }
    let config = ServiceDatabaseConfig::from_env()?;
    let pool = db::connect_service(&config).await?;
    *pool_slot = Some(pool.clone());
    let worker = Worker::new(
        pool.clone(),
        MockAdapters::load(root, Faults::default())?,
        EmbeddingClient::new(EmbeddingConfig::from_env()?)?,
        ObjectStorage::new(StorageConfig::from_env()?)?,
        format!("worker-{}", Uuid::new_v4()),
    )
    .await?;
    if command == "activate-model" {
        worker.activate_model().await?;
    } else {
        if let Some(id) = run_id {
            worker.resume(id).await?;
        }
        let work = async {
            let mut steps = 0;
            loop {
                worker.start_due().await?;
                match worker.step().await? {
                    Step::Idle if command != "serve" => break,
                    Step::Idle => tokio::time::sleep(Duration::from_secs(2)).await,
                    step => {
                        tracing::info!(outcome=?step, "worker job processed");
                        steps += 1;
                        if command != "serve" && steps >= limit {
                            break;
                        }
                    }
                }
            }
            Ok::<_, screenroom_backend::Error>(())
        };
        work.await?;
    }
    Ok(())
}
