use screenroom_observability::telemetry::{Telemetry, TelemetryConfig, shutdown_signal};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let address = std::env::var_os("SCREENROOM_PLAYBACK_GATEWAY_LISTEN")
        .map(|v| {
            v.into_string()
                .map_err(|_| "invalid SCREENROOM_PLAYBACK_GATEWAY_LISTEN Unicode")
        })
        .transpose()?
        .unwrap_or_else(|| "127.0.0.1:50054".into())
        .parse::<std::net::SocketAddr>()
        .map_err(|_| "invalid SCREENROOM_PLAYBACK_GATEWAY_LISTEN socket address")?;
    let origins = std::env::var_os("SCREENROOM_PLAYBACK_GATEWAY_CORS_ORIGINS")
        .map(|v| {
            v.into_string()
                .map_err(|_| "invalid SCREENROOM_PLAYBACK_GATEWAY_CORS_ORIGINS Unicode")
        })
        .transpose()?
        .unwrap_or_default()
        .split(',')
        .filter(|v| !v.trim().is_empty())
        .map(|value| {
            let value = value.trim();
            if value.contains('#') {
                return Err("invalid SCREENROOM_PLAYBACK_GATEWAY_CORS_ORIGINS origin");
            }
            let uri = value
                .parse::<tonic::codegen::http::Uri>()
                .map_err(|_| "invalid SCREENROOM_PLAYBACK_GATEWAY_CORS_ORIGINS origin")?;
            if !matches!(uri.scheme_str(), Some("http" | "https"))
                || uri.authority().is_none()
                || uri.authority().is_some_and(|a| a.as_str().contains('@'))
                || uri.path_and_query().is_some_and(|p| p.as_str() != "/")
            {
                return Err("invalid SCREENROOM_PLAYBACK_GATEWAY_CORS_ORIGINS origin");
            }
            value
                .parse()
                .map_err(|_| "invalid SCREENROOM_PLAYBACK_GATEWAY_CORS_ORIGINS origin")
        })
        .collect::<Result<Vec<_>, _>>()?;
    let telemetry = Telemetry::init("screenroom-playback-gateway", TelemetryConfig::from_env()?)?;
    let result = async {
        let listener = tokio::net::TcpListener::bind(address).await?;
        tracing::info!(service.name = "screenroom-playback-gateway", address = %listener.local_addr()?, "RPC server listening");
        screenroom_playback_gateway::serve(listener, shutdown_signal(), origins).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    }.await;
    let shutdown = telemetry.shutdown();
    result?;
    shutdown?;
    Ok(())
}
