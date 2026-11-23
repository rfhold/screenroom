mod fences;
mod owned;
mod records;
pub mod runtime;
mod service;
mod signed_resources;
pub(crate) mod transport;
mod validation;

pub use service::PlaybackService;

#[cfg(test)]
mod tests;

pub(crate) async fn serve(
    service: PlaybackService,
    listener: tokio::net::TcpListener,
    shutdown: impl Future<Output = ()> + Send + 'static,
    origins: Vec<tonic::codegen::http::HeaderValue>,
) -> Result<(), tonic::transport::Error> {
    tonic::transport::Server::builder()
        .accept_http1(true)
        .serve_with_incoming_shutdown(
            transport::router(service, origins),
            tokio_stream::wrappers::TcpListenerStream::new(listener),
            shutdown,
        )
        .await
}
