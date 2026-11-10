mod browse;
mod mapping;
mod operator;
mod page;
mod progress;
mod search;
mod transport;

use screenroom_backend::{
    catalog::CatalogRepository, media::MediaRepository, search::SearchRepository, sqlx::PgPool,
    storage::ObjectStorage,
};
use tonic::Status;

#[derive(Clone)]
pub struct Catalog {
    pub(crate) catalog: CatalogRepository,
    pub(crate) media: MediaRepository,
    pub(crate) search: SearchRepository,
    pub(crate) storage: Option<ObjectStorage>,
    pub(crate) origin: String,
    pub(crate) user_uri: Option<String>,
    pub(crate) embedding: Option<screenroom_backend::embedding::EmbeddingClient>,
}

impl Catalog {
    pub fn new(
        pool: PgPool,
        origin: String,
        storage: Option<ObjectStorage>,
        user_uri: Option<String>,
    ) -> Result<Self, Status> {
        let uri = origin
            .parse::<tonic::codegen::http::Uri>()
            .map_err(|_| Status::invalid_argument("public origin"))?;
        if !matches!(uri.scheme_str(), Some("http" | "https"))
            || uri.authority().is_none_or(|a| {
                a.as_str().contains('@')
                    || a.host().is_empty()
                    || (a.as_str().contains(':')
                        && !a.as_str().ends_with(']')
                        && a.port_u16().is_none())
            })
            || uri.path_and_query().is_some_and(|p| p.as_str() != "/")
            || origin.contains('#')
        {
            return Err(Status::invalid_argument("public origin"));
        }
        Ok(Self {
            catalog: CatalogRepository::new(pool.clone()),
            media: MediaRepository::new(pool.clone()),
            search: SearchRepository::new(pool),
            storage,
            origin: origin.trim_end_matches('/').into(),
            user_uri,
            embedding: None,
        })
    }
    pub fn with_embedding(
        mut self,
        client: screenroom_backend::embedding::EmbeddingClient,
    ) -> Self {
        self.embedding = Some(client);
        self
    }
}

pub(crate) fn status(error: screenroom_backend::Error) -> Status {
    use screenroom_backend::Error;
    match error {
        Error::Invalid(message) => Status::invalid_argument(message),
        Error::NotFound => Status::not_found("not found"),
        Error::SourceBusy { .. } => Status::failed_precondition("source has an active run"),
        Error::Stale => Status::failed_precondition("stale state"),
        Error::Incompatible(_) => Status::failed_precondition("incompatible upstream response"),
        Error::Database(_) | Error::Unavailable => Status::unavailable("backend unavailable"),
    }
}

pub use operator::Operator;
pub use operator::validate_policy_write;
pub use transport::{serve, serve_operator};

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn public_origin_is_explicit_and_has_no_path_or_credentials() {
        let pool = screenroom_backend::sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://localhost/unused")
            .unwrap();
        for origin in [
            "https://example.com",
            "http://127.0.0.1:50052/",
            "http://[::1]:50052",
        ] {
            assert!(Catalog::new(pool.clone(), origin.into(), None, None).is_ok());
        }
        for origin in [
            "ftp://example.com",
            "https://user:password@example.com",
            "https://example.com/path",
            "https://example.com?query",
            "https://example.com/#fragment",
            "http://localhost:invalid",
        ] {
            assert!(
                Catalog::new(pool.clone(), origin.into(), None, None).is_err(),
                "{origin}"
            );
        }
    }
}
