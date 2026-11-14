use screenroom_observability::telemetry::{Telemetry, TelemetryConfig, shutdown_signal};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let address = std::env::var_os("SCREENROOM_USER_LISTEN")
        .map(|v| {
            v.into_string()
                .map_err(|_| "invalid SCREENROOM_USER_LISTEN Unicode")
        })
        .transpose()?
        .unwrap_or_else(|| "127.0.0.1:50053".into())
        .parse::<std::net::SocketAddr>()
        .map_err(|_| "invalid SCREENROOM_USER_LISTEN socket address")?;
    let origins = std::env::var_os("SCREENROOM_USER_CORS_ORIGINS")
        .map(|v| {
            v.into_string()
                .map_err(|_| "invalid SCREENROOM_USER_CORS_ORIGINS Unicode")
        })
        .transpose()?
        .unwrap_or_default()
        .split(',')
        .filter(|v| !v.trim().is_empty())
        .map(|value| {
            let value = value.trim();
            if value.contains('#') {
                return Err("invalid SCREENROOM_USER_CORS_ORIGINS origin");
            }
            let uri = value
                .parse::<tonic::codegen::http::Uri>()
                .map_err(|_| "invalid SCREENROOM_USER_CORS_ORIGINS origin")?;
            if !matches!(uri.scheme_str(), Some("http" | "https"))
                || uri.authority().is_none()
                || uri.authority().is_some_and(|a| a.as_str().contains('@'))
                || uri.path_and_query().is_some_and(|p| p.as_str() != "/")
            {
                return Err("invalid SCREENROOM_USER_CORS_ORIGINS origin");
            }
            value
                .parse()
                .map_err(|_| "invalid SCREENROOM_USER_CORS_ORIGINS origin")
        })
        .collect::<Result<Vec<_>, _>>()?;
    let telemetry = Telemetry::init("screenroom-user", TelemetryConfig::from_env()?)?;
    let result = async {
        let pool = screenroom_backend::db::connect_service(&screenroom_backend::config::ServiceDatabaseConfig::from_env()?).await?;
        let connections=screenroom_backend::connections::ConnectionRepository::from_env(pool.clone())?;
        let repository = screenroom_user::repository::Repository::new(pool);
        repository.ready().await?;
        let service = screenroom_user::UserService {
            repository,
            connections,
        };
        let private_address = std::env::var("SCREENROOM_USER_PRIVATE_LISTEN").unwrap_or_else(|_| "127.0.0.1:50063".into()).parse::<std::net::SocketAddr>()?;
        let listener = tokio::net::TcpListener::bind(address).await?;
        let private_listener = tokio::net::TcpListener::bind(private_address).await?;
        tracing::info!(service.name = "screenroom-user", address = %listener.local_addr()?, "RPC server listening");
        let (stop, _) = tokio::sync::watch::channel(false);
        let mut public_stop = stop.subscribe();
        let mut private_stop = stop.subscribe();
        let watched = tokio::spawn(service.clone().run_watched_sync(stop.subscribe()));
        let signal = async { shutdown_signal().await; let _ = stop.send(true); };
        let public = screenroom_user::serve(service.clone(), listener, async move { let _ = public_stop.changed().await; }, origins);
        let private = screenroom_user::serve_private(service, private_listener, async move { let _ = private_stop.changed().await; });
        tokio::pin!(public, private);
        let serving = tokio::select! {
            _ = signal => { tokio::try_join!(&mut public, &mut private).map(|_| ()) }
            result = &mut public => { let _ = stop.send(true); let other = private.await; result.and(other) }
            result = &mut private => { let _ = stop.send(true); let other = public.await; result.and(other) }
        };
        let _ = stop.send(true);
        watched.await?;
        serving?;
        Ok::<_, Box<dyn std::error::Error>>(())
    }.await;
    let shutdown = telemetry.shutdown();
    result?;
    shutdown?;
    Ok(())
}
