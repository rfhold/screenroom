use screenroom_observability::telemetry::rpc_layer;

pub async fn serve(
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
            ]
            .map(HeaderName::from_static),
        )
        .expose_headers(["grpc-status", "grpc-message"].map(HeaderName::from_static));
    tonic::transport::Server::builder()
        .accept_http1(true)
        .layer(cors)
        .layer(tonic_web::GrpcWebLayer::new())
        .layer(rpc_layer())
        .add_routes(tonic::service::Routes::default())
        .serve_with_incoming_shutdown(
            tokio_stream::wrappers::TcpListenerStream::new(listener),
            shutdown,
        )
        .await
}
