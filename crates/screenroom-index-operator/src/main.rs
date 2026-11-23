use screenroom_backend::{config::ServiceDatabaseConfig, db};
use screenroom_index_operator::{Operator, serve};
use screenroom_observability::telemetry::{Telemetry, TelemetryConfig, shutdown_signal};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let address = std::env::var("SCREENROOM_OPERATOR_LISTEN")
        .unwrap_or_else(|_| "127.0.0.1:50072".into())
        .parse::<std::net::SocketAddr>()?;
    if !address.ip().is_loopback() && !address.ip().is_unspecified() {
        return Err("operator listen must be loopback or explicit unspecified address".into());
    }
    let origins = std::env::var("SCREENROOM_OPERATOR_CORS_ORIGINS")
        .unwrap_or_default()
        .split(',')
        .filter(|origin| !origin.is_empty())
        .map(str::parse)
        .collect::<Result<Vec<axum::http::HeaderValue>, _>>()?;
    let telemetry = Telemetry::init("screenroom-index-operator", TelemetryConfig::from_env()?)?;
    let result = async {
        let pool = db::connect_service(&ServiceDatabaseConfig::from_env()?).await?;
        let connections =
            screenroom_backend::connections::ConnectionRepository::from_env(pool.clone())?;
        let operator = Operator::new_rpc(pool).with_connections(connections);
        let listener = tokio::net::TcpListener::bind(address).await?;
        serve(operator, listener, shutdown_signal(), origins).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    }
    .await;
    let shutdown = telemetry.shutdown();
    result?;
    shutdown?;
    Ok(())
}
