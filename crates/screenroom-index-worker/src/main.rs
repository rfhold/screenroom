use screenroom_backend::{
    Uuid,
    config::ServiceDatabaseConfig,
    db,
    embedding::{EmbeddingClient, EmbeddingConfig},
    storage::{ObjectStorage, StorageConfig},
};
use screenroom_index_worker::{Step, Worker};
use screenroom_observability::telemetry::{Telemetry, TelemetryConfig, shutdown_signal};
use std::time::Duration;

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!(
            "index worker failed ({}); check configuration and dependency availability",
            failure_class(error.as_ref())
        );
        std::process::exit(1);
    }
}
fn failure_class(error: &(dyn std::error::Error + 'static)) -> &'static str {
    use screenroom_backend::Error;
    match error.downcast_ref::<Error>() {
        Some(Error::SourceBusy { .. }) => "source_busy",
        Some(Error::Database(_)) => "database",
        Some(Error::Invalid(_)) => "invalid_input",
        Some(Error::Stale) => "stale",
        Some(Error::NotFound) => "not_found",
        Some(Error::Unavailable) => "unavailable",
        Some(Error::Incompatible(_)) => "incompatible",
        None => "configuration_or_dependency",
        Some(Error::Connection(_)) => "connection_unavailable",
    }
}
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "serve".into());
    if !matches!(command.as_str(), "serve" | "drain" | "activate-model") || args.next().is_some() {
        return Err("expected serve, drain, or activate-model; configure and control canonical runs through the operator API".into());
    }
    let telemetry = Telemetry::init("screenroom-index-worker", TelemetryConfig::from_env()?)?;
    let limit = std::env::var("SCREENROOM_WORKER_DRAIN_LIMIT")
        .unwrap_or_else(|_| "1000".into())
        .parse::<usize>()?;
    if !(1..=100_000).contains(&limit) {
        return Err("invalid drain limit".into());
    }
    let embedding = EmbeddingConfig::optional_from_env()?
        .map(EmbeddingClient::new)
        .transpose()?;
    let pool = db::connect_service(&ServiceDatabaseConfig::from_env()?).await?;
    let result = async {
        let worker = Worker::new(
            pool.clone(),
            embedding,
            ObjectStorage::new(StorageConfig::from_env()?)?,
            format!("worker-{}", Uuid::new_v4()),
        )
        .await?
        .with_metrics(
            screenroom_observability::metrics::QueueMetrics::new(&telemetry.meter()),
            screenroom_observability::metrics::WorkerMetrics::new(&telemetry.meter()),
        );
        if command == "activate-model" {
            worker.activate_model().await?;
            return Ok::<_, Box<dyn std::error::Error>>(());
        }
        let work = async {
            let mut steps = 0;
            loop {
                let (_, sweep_complete) = worker.start_due().await?;
                match worker.step().await? {
                    Step::Idle if command != "serve" && sweep_complete => break,
                    Step::Idle if command != "serve" => {}
                    Step::Idle => tokio::time::sleep(Duration::from_secs(2)).await,
                    step => {
                        tracing::info!(outcome=?step,"worker job processed");
                        steps += 1;
                        if command != "serve" && steps >= limit {
                            break;
                        }
                    }
                }
            }
            Ok::<_, screenroom_backend::Error>(())
        };
        tokio::select! { result=work=>result?, _=shutdown_signal()=>{} }
        Ok(())
    }
    .await;
    let closed = tokio::time::timeout(Duration::from_secs(5), pool.close())
        .await
        .is_ok();
    let shutdown = telemetry.shutdown();
    result?;
    shutdown?;
    if !closed {
        return Err("database shutdown timed out".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failure_classes_never_render_payloads() {
        assert_eq!(
            failure_class(&screenroom_backend::Error::Invalid("private payload")),
            "invalid_input"
        );
    }
}
