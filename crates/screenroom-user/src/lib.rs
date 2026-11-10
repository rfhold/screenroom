pub mod progress;
pub mod repository;

use repository::{Repository, uuid};
use screenroom_catalog_adapter::MockAdapters;
use screenroom_observability::telemetry::rpc_layer;
use screenroom_proto::user::{internal::v1 as internal, operator::v1 as operator, v1::*};
use std::sync::Arc;
use tonic::{Request, Response, Status};

#[derive(Clone)]
pub struct UserService {
    pub repository: Repository,
    pub adapters: Arc<MockAdapters>,
}

pub fn local_selector<T>(request: &Request<T>) -> Result<screenroom_backend::Uuid, Status> {
    let mut values = request
        .metadata()
        .get_all(screenroom_proto::LOCAL_USER_ID_METADATA_KEY)
        .iter();
    let value = values
        .next()
        .ok_or_else(|| Status::invalid_argument("local user selector required"))?;
    if values.next().is_some() {
        return Err(Status::invalid_argument("duplicate local user selector"));
    }
    uuid(
        value
            .to_str()
            .map_err(|_| Status::invalid_argument("invalid local user selector"))?,
    )
}

#[tonic::async_trait]
impl user_service_server::UserService for UserService {
    async fn get_current_user(
        &self,
        request: Request<GetCurrentUserRequest>,
    ) -> Result<Response<GetCurrentUserResponse>, Status> {
        let user = self.repository.user(local_selector(&request)?).await?;
        Ok(Response::new(GetCurrentUserResponse {
            user: Some(User {
                id: user.id.to_string(),
                display_name: user.display_name,
            }),
        }))
    }
}

#[tonic::async_trait]
impl operator::jellyfin_user_service_server::JellyfinUserService for UserService {
    async fn set_link(
        &self,
        request: Request<operator::SetLinkRequest>,
    ) -> Result<Response<operator::SetLinkResponse>, Status> {
        let mut link = request
            .into_inner()
            .link
            .ok_or_else(|| Status::invalid_argument("link required"))?;
        let id = uuid(&link.user_id)?;
        self.repository
            .set_link(id, &link.source_id, &link.jellyfin_user_id)
            .await?;
        link.user_id = id.to_string();
        Ok(Response::new(operator::SetLinkResponse {
            link: Some(link),
        }))
    }
    async fn clear_link(
        &self,
        request: Request<operator::ClearLinkRequest>,
    ) -> Result<Response<operator::ClearLinkResponse>, Status> {
        let request = request.into_inner();
        self.repository
            .clear_link(uuid(&request.user_id)?, &request.source_id)
            .await?;
        Ok(Response::new(operator::ClearLinkResponse {}))
    }
}

#[tonic::async_trait]
impl internal::playback_state_service_server::PlaybackStateService for UserService {
    async fn get_continue_watching(
        &self,
        request: Request<internal::GetContinueWatchingRequest>,
    ) -> Result<Response<internal::GetContinueWatchingResponse>, Status> {
        Ok(Response::new(self.progress(request.into_inner()).await?))
    }
}

pub async fn serve(
    service: UserService,
    listener: tokio::net::TcpListener,
    shutdown: impl Future<Output = ()> + Send + 'static,
    origins: Vec<tonic::codegen::http::HeaderValue>,
) -> Result<(), tonic::transport::Error> {
    use tonic::codegen::http::{HeaderName, Method};
    let cors = tower_http::cors::CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([Method::POST])
        .allow_headers(
            [
                "content-type",
                "x-grpc-web",
                "x-user-agent",
                "grpc-timeout",
                "traceparent",
                "tracestate",
                screenroom_proto::LOCAL_USER_ID_METADATA_KEY,
            ]
            .map(HeaderName::from_static),
        )
        .expose_headers(["grpc-status", "grpc-message"].map(HeaderName::from_static));
    tonic::transport::Server::builder()
        .accept_http1(true)
        .timeout(std::time::Duration::from_secs(10))
        .layer(cors)
        .layer(tonic_web::GrpcWebLayer::new())
        .layer(rpc_layer())
        .add_service(user_service_server::UserServiceServer::new(service))
        .serve_with_incoming_shutdown(
            tokio_stream::wrappers::TcpListenerStream::new(listener),
            shutdown,
        )
        .await
}

pub async fn serve_private(
    service: UserService,
    listener: tokio::net::TcpListener,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), tonic::transport::Error> {
    tonic::transport::Server::builder()
        .timeout(std::time::Duration::from_secs(10))
        .layer(rpc_layer())
        .add_service(
            operator::jellyfin_user_service_server::JellyfinUserServiceServer::new(service.clone()),
        )
        .add_service(
            internal::playback_state_service_server::PlaybackStateServiceServer::new(service),
        )
        .serve_with_incoming_shutdown(
            tokio_stream::wrappers::TcpListenerStream::new(listener),
            shutdown,
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_is_one_uuid_not_authentication() {
        let mut request = Request::new(());
        assert!(local_selector(&request).is_err());
        request.metadata_mut().insert(
            screenroom_proto::LOCAL_USER_ID_METADATA_KEY,
            "bad".parse().unwrap(),
        );
        assert!(local_selector(&request).is_err());
        request.metadata_mut().insert(
            screenroom_proto::LOCAL_USER_ID_METADATA_KEY,
            "00000000-0000-0000-0000-000000000001".parse().unwrap(),
        );
        assert!(local_selector(&request).is_ok());
        request.metadata_mut().append(
            screenroom_proto::LOCAL_USER_ID_METADATA_KEY,
            "00000000-0000-0000-0000-000000000002".parse().unwrap(),
        );
        assert!(local_selector(&request).is_err());
    }
}
