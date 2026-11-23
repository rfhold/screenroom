use screenroom_observability::telemetry::{Telemetry, TelemetryConfig, shutdown_signal};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = screenroom_playback_gateway::runtime::RuntimeConfig::from_env()?;
    let telemetry = Telemetry::init("screenroom-playback-gateway", TelemetryConfig::from_env()?)?;
    let result = screenroom_playback_gateway::runtime::run(config, shutdown_signal()).await;
    let shutdown = telemetry.shutdown();
    result?;
    shutdown?;
    Ok(())
}
