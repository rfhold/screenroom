use crate::{Catalog, Operator};
use axum::{
    Router,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use screenroom_backend::{Uuid, catalog::Scope};
use screenroom_observability::telemetry::rpc_layer;
use screenroom_proto::catalog::{
    operator::v1::ingestion_service_server::IngestionServiceServer, v1::*,
};
use std::time::Duration;

pub(crate) const RPC_MAXIMUM: Duration = Duration::from_secs(30);
const SHUTDOWN_DRAIN: Duration = Duration::from_secs(5);

async fn bounded_serve<F, E>(
    server: impl FnOnce(tokio::sync::oneshot::Receiver<()>) -> F,
    shutdown: impl Future<Output = ()>,
    cancel: tokio::sync::watch::Sender<bool>,
) -> Result<(), E>
where
    F: Future<Output = Result<(), E>>,
{
    let (tx, rx) = tokio::sync::oneshot::channel();
    let server = server(rx);
    tokio::pin!(server);
    tokio::select! {
        result = &mut server => result,
        () = shutdown => {
            let _ = tx.send(());
            match tokio::time::timeout(SHUTDOWN_DRAIN, &mut server).await {
                Ok(result) => result,
                Err(_) => {
                    let _ = cancel.send(true);
                    tokio::time::timeout(Duration::from_millis(100), &mut server)
                        .await.unwrap_or(Ok(()))
                }
            }
        }
    }
}

pub async fn serve(
    listener: tokio::net::TcpListener,
    shutdown: impl Future<Output = ()> + Send + 'static,
    origins: Vec<tonic::codegen::http::HeaderValue>,
    catalog: Catalog,
) -> Result<(), std::io::Error> {
    let (cancel, cancelled) = tokio::sync::watch::channel(false);
    let grpc = tonic::service::Routes::new(browse_service_server::BrowseServiceServer::new(
        catalog.clone(),
    ))
    .add_service(catalog_service_server::CatalogServiceServer::new(
        catalog.clone(),
    ))
    .add_service(search_service_server::SearchServiceServer::new(
        catalog.clone(),
    ))
    .into_axum_router()
    .layer(rpc_layer())
    .layer(axum::middleware::from_fn(move |request, next: axum::middleware::Next| {
        let mut cancelled = cancelled.clone();
        async move {
            tokio::select! {
                response = next.run(request) => response,
                _ = cancelled.changed() => tonic::Status::cancelled("server shutting down").into_http::<Body>(),
            }
        }
    }))
    .layer(tonic_web::GrpcWebLayer::new());
    let cors = tower_http::cors::CorsLayer::new()
        .allow_origin(origins)
        .allow_methods([Method::POST, Method::GET, Method::HEAD])
        .allow_headers(
            [
                "content-type",
                "x-grpc-web",
                "x-user-agent",
                "grpc-timeout",
                "traceparent",
                "tracestate",
                "x-screenroom-user-id",
                "if-none-match",
            ]
            .map(header::HeaderName::from_static),
        )
        .expose_headers(
            ["grpc-status", "grpc-message", "etag", "content-length"]
                .map(header::HeaderName::from_static),
        );
    let app = Router::new()
        .route("/artwork/{id}/revisions/{revision}/{variant}", get(artwork))
        .route("/ready", get(ready))
        .with_state(catalog)
        .fallback_service(
            axum::routing::post_service(grpc).fallback(|| async { StatusCode::NOT_FOUND }),
        )
        .layer(cors);
    bounded_serve(
        |signal| {
            tonic::transport::Server::builder()
                .accept_http1(true)
                .timeout(RPC_MAXIMUM)
                .serve_with_incoming_shutdown(
                    app,
                    tokio_stream::wrappers::TcpListenerStream::new(listener),
                    async {
                        let _ = signal.await;
                    },
                )
        },
        shutdown,
        cancel,
    )
    .await
    .map_err(std::io::Error::other)
}
pub async fn serve_operator(
    listener: tokio::net::TcpListener,
    shutdown: impl Future<Output = ()> + Send + 'static,
    operator: Operator,
) -> Result<(), tonic::transport::Error> {
    let (cancel, _) = tokio::sync::watch::channel(false);
    let server = tonic::transport::Server::builder()
        .timeout(RPC_MAXIMUM)
        .layer(rpc_layer())
        .add_service(IngestionServiceServer::new(operator));
    bounded_serve(
        |signal| {
            server.serve_with_incoming_shutdown(
                tokio_stream::wrappers::TcpListenerStream::new(listener),
                async {
                    let _ = signal.await;
                },
            )
        },
        shutdown,
        cancel,
    )
    .await
}
async fn ready(State(catalog): State<Catalog>) -> StatusCode {
    match screenroom_backend::db::ready(catalog.catalog.pool()).await {
        Ok(()) => StatusCode::OK,
        Err(_) => StatusCode::SERVICE_UNAVAILABLE,
    }
}
async fn artwork(
    State(catalog): State<Catalog>,
    Path((id, revision, variant)): Path<(String, String, String)>,
    method: Method,
    headers: HeaderMap,
) -> Response {
    let Ok(id) = Uuid::parse_str(&id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Ok(revision) = revision.parse::<i64>() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if revision < 1 || !matches!(variant.as_str(), "native" | "original") {
        return StatusCode::NOT_FOUND.into_response();
    }
    let object = match catalog
        .media
        .resolve(&Scope::default(), id, revision, &variant)
        .await
    {
        Ok(Some(o)) => o,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
    };
    if object.location.byte_length > 4 * 1024 * 1024 || object.location.byte_length < 1 {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let etag = format!(
        "\"{}\"",
        object
            .location
            .sha256
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    );
    let not_modified = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|s| {
            s.split(',')
                .map(str::trim)
                .any(|v| v == etag || v.strip_prefix("W/") == Some(etag.as_str()) || v == "*")
        });
    let code = if not_modified {
        StatusCode::NOT_MODIFIED
    } else {
        StatusCode::OK
    };
    let body = if not_modified {
        Body::empty()
    } else {
        let Some(storage) = catalog.storage.as_ref() else {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        };
        if method == Method::HEAD {
            if storage.head(&object.location).await.is_err() {
                return StatusCode::SERVICE_UNAVAILABLE.into_response();
            }
            Body::empty()
        } else {
            match storage.read_stream(&object.location, 4 * 1024 * 1024).await {
                Ok(read) => Body::from_stream(read.body),
                Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
            }
        }
    };
    let mut response = Response::new(body);
    *response.status_mut() = code;
    let h = response.headers_mut();
    for (key, value) in [
        (header::CONTENT_TYPE, object.location.mime),
        (header::ETAG, etag),
        (
            header::CACHE_CONTROL,
            "private, max-age=0, must-revalidate".into(),
        ),
        (header::X_CONTENT_TYPE_OPTIONS, "nosniff".into()),
    ] {
        match value.parse() {
            Ok(value) => {
                h.insert(key, value);
            }
            Err(_) => return StatusCode::SERVICE_UNAVAILABLE.into_response(),
        }
    }
    if !not_modified && let Ok(value) = object.location.byte_length.to_string().parse() {
        h.insert(header::CONTENT_LENGTH, value);
    }
    response
}
