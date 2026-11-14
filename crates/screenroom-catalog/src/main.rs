use screenroom_observability::telemetry::{Telemetry, TelemetryConfig, shutdown_signal};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let address = std::env::var_os("SCREENROOM_CATALOG_LISTEN")
        .map(|v| {
            v.into_string()
                .map_err(|_| "invalid SCREENROOM_CATALOG_LISTEN Unicode")
        })
        .transpose()?
        .unwrap_or_else(|| "127.0.0.1:50052".into())
        .parse::<std::net::SocketAddr>()
        .map_err(|_| "invalid SCREENROOM_CATALOG_LISTEN socket address")?;
    let origins = std::env::var_os("SCREENROOM_CATALOG_CORS_ORIGINS")
        .map(|v| {
            v.into_string()
                .map_err(|_| "invalid SCREENROOM_CATALOG_CORS_ORIGINS Unicode")
        })
        .transpose()?
        .unwrap_or_default()
        .split(',')
        .filter(|v| !v.trim().is_empty())
        .map(|value| {
            let value = value.trim();
            if value.contains('#') {
                return Err("invalid SCREENROOM_CATALOG_CORS_ORIGINS origin");
            }
            let uri = value
                .parse::<tonic::codegen::http::Uri>()
                .map_err(|_| "invalid SCREENROOM_CATALOG_CORS_ORIGINS origin")?;
            if !matches!(uri.scheme_str(), Some("http" | "https"))
                || uri.authority().is_none()
                || uri.authority().is_some_and(|a| a.as_str().contains('@'))
                || uri.path_and_query().is_some_and(|p| p.as_str() != "/")
            {
                return Err("invalid SCREENROOM_CATALOG_CORS_ORIGINS origin");
            }
            value
                .parse()
                .map_err(|_| "invalid SCREENROOM_CATALOG_CORS_ORIGINS origin")
        })
        .collect::<Result<Vec<_>, _>>()?;
    let telemetry = Telemetry::init("screenroom-catalog", TelemetryConfig::from_env()?)?;
    let result = async {
        use screenroom_backend::{config::{ServiceDatabaseConfig,required},storage::{ObjectStorage,StorageConfig}};
        let pool=screenroom_backend::db::connect_service(&ServiceDatabaseConfig::from_env()?).await?;
        let storage=if std::env::var_os("SCREENROOM_S3_ENDPOINT").is_some() {Some(ObjectStorage::new(StorageConfig::from_env()?)?)} else {None};
        let user_uri=std::env::var("SCREENROOM_USER_PRIVATE_URI").ok();
        let mut catalog=screenroom_catalog::Catalog::new(pool.clone(),required("SCREENROOM_CATALOG_PUBLIC_ORIGIN")?,storage,user_uri)?;
        if std::env::var_os("SCREENROOM_EMBEDDING_BASE_URL").is_some() {
            catalog=catalog.with_embedding(screenroom_backend::embedding::EmbeddingClient::new(screenroom_backend::embedding::EmbeddingConfig::from_env()?)?);
        }
        let listener = tokio::net::TcpListener::bind(address).await?;
        tracing::info!(service.name = "screenroom-catalog", address = %listener.local_addr()?, "RPC server listening");
        screenroom_catalog::serve(listener, shutdown_signal(), origins,catalog).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    }.await;
    let shutdown = telemetry.shutdown();
    result?;
    shutdown?;
    Ok(())
}
