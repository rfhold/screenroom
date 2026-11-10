#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    screenroom_backend::db::migrate_service_schema(
        &screenroom_backend::config::ServiceDatabaseConfig::from_env()?,
    )
    .await?;
    Ok(())
}
