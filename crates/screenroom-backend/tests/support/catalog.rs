#![allow(dead_code)]
use screenroom_backend::{
    Error, Result, Uuid,
    catalog::{Library, Source},
    sqlx::{self, PgPool},
};
use serde_json::Value;

pub struct CatalogRepository {
    inner: screenroom_backend::catalog::CatalogRepository,
    pool: PgPool,
}
impl std::ops::Deref for CatalogRepository {
    type Target = screenroom_backend::catalog::CatalogRepository;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
impl CatalogRepository {
    pub fn new(pool: PgPool) -> Self {
        Self {
            inner: screenroom_backend::catalog::CatalogRepository::new(pool.clone()),
            pool,
        }
    }
    pub async fn configure_source(
        &self,
        key: &str,
        provider: &str,
        configuration: &Value,
        capabilities: &Value,
    ) -> Result<Source> {
        if key.is_empty() || key.len() > 200 || !matches!(provider, "jellyfin" | "tmdb") {
            return Err(Error::Invalid("fixture source"));
        }
        Ok(sqlx::query_as("INSERT INTO catalog.sources(source_key,provider,configuration,capabilities) VALUES($1,$2,$3,$4) ON CONFLICT(source_key) DO UPDATE SET configuration=EXCLUDED.configuration,capabilities=EXCLUDED.capabilities RETURNING *")
            .bind(key).bind(provider).bind(configuration).bind(capabilities).fetch_one(&self.pool).await?)
    }
    pub async fn configure_library(
        &self,
        source: Uuid,
        public_id: &str,
        external_id: &str,
        title: &str,
    ) -> Result<Library> {
        if public_id.is_empty()
            || public_id == "all"
            || public_id.len() > 200
            || external_id.is_empty()
        {
            return Err(Error::Invalid("fixture library"));
        }
        Ok(sqlx::query_as("INSERT INTO catalog.libraries(source_id,public_id,external_id,title) VALUES($1,$2,$3,$4) ON CONFLICT(source_id,external_id) DO UPDATE SET title=EXCLUDED.title,public_id=EXCLUDED.public_id RETURNING *")
            .bind(source).bind(public_id).bind(external_id).bind(title).fetch_one(&self.pool).await?)
    }
}
