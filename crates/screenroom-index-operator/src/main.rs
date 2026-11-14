use screenroom_backend::{config::ServiceDatabaseConfig, db};
use screenroom_index_operator::{Operator, Origin};
use screenroom_observability::telemetry::{Telemetry, TelemetryConfig, shutdown_signal};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let origin = Origin::parse(&std::env::var("SCREENROOM_OPERATOR_ORIGIN")?)?;
    let address = std::env::var("SCREENROOM_OPERATOR_LISTEN")
        .unwrap_or_else(|_| "127.0.0.1:50072".into())
        .parse::<std::net::SocketAddr>()?;
    if !address.ip().is_loopback() && !address.ip().is_unspecified() {
        return Err("operator listen must be loopback or explicit unspecified address".into());
    }
    let telemetry = Telemetry::init("screenroom-index-operator", TelemetryConfig::from_env()?)?;
    let result = async {
        let pool = db::connect_service(&ServiceDatabaseConfig::from_env()?).await?;
        let connections =
            screenroom_backend::connections::ConnectionRepository::from_env(pool.clone())?;
        let app = Operator::new(pool, origin)
            .with_connections(connections)
            .router();
        let listener = tokio::net::TcpListener::bind(address).await?;
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal())
            .await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    }
    .await;
    let shutdown = telemetry.shutdown();
    result?;
    shutdown?;
    Ok(())
}
