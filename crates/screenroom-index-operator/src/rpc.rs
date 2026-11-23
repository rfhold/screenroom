use super::{BODY_LIMIT, Error, Operator, api, id};
use axum::http::Method;
use screenroom_proto::operator::v1::{
    OperatorRequest, OperatorResponse,
    operator_service_server::{OperatorService, OperatorServiceServer},
};
use tonic::{Request, Response, Status};

const RESPONSE_LIMIT: usize = 2 * 1024 * 1024;

async fn call(
    op: &Operator,
    input: OperatorRequest,
    method: Method,
    path: &str,
    paginated: bool,
    pending: bool,
) -> Result<Response<OperatorResponse>, Status> {
    if input.payload_json.len() > BODY_LIMIT
        || input.page.is_some() != paginated
        || input.page.as_ref().is_some_and(|p| {
            p.pending_only.is_some() != pending
                || p.after
                    .as_ref()
                    .is_some_and(|a| a.is_empty() || a.len() > 200)
        })
    {
        return Err(Status::invalid_argument("invalid"));
    }
    let mut query = Vec::new();
    if let Some(page) = input.page {
        query.push(("limit", page.limit.to_string()));
        if let Some(after) = page.after {
            query.push(("after", after));
        }
        if let Some(pending_only) = page.pending_only {
            query.push(("pending_only", pending_only.to_string()));
        }
    }
    let query =
        serde_urlencoded::to_string(query).map_err(|_| Status::invalid_argument("invalid"))?;
    let result = api(op, &method, path, &query, &input.payload_json)
        .await
        .map_err(status)?;
    let payload_json =
        serde_json::to_vec(&result).map_err(|_| Status::internal("invalid_response"))?;
    if payload_json.len() > RESPONSE_LIMIT {
        return Err(Status::resource_exhausted("invalid_response"));
    }
    Ok(Response::new(OperatorResponse { payload_json }))
}

fn status(error: Error) -> Status {
    match error {
        Error::Invalid(_) => Status::invalid_argument("invalid"),
        Error::NotFound => Status::not_found("not_found"),
        Error::Stale => Status::failed_precondition("stale"),
        Error::SourceBusy { run_id, .. } => {
            let mut status = Status::aborted("source_busy");
            status.metadata_mut().insert(
                "x-screenroom-run-id",
                run_id.to_string().parse().expect("UUID metadata"),
            );
            status
        }
        Error::Connection(code) => match code {
            "request_conflict"
            | "request_expired"
            | "server_identity_mismatch"
            | "server_already_connected" => Status::already_exists(code),
            "invalid_connection"
            | "invalid_discovery"
            | "invalid_credentials"
            | "credentials_rejected"
            | "invalid_server_identity"
            | "invalid_selection"
            | "profile_unavailable"
            | "library_unavailable"
            | "unsupported_library"
            | "unsupported_library_scope"
            | "selection_capacity"
            | "local_user_unavailable" => Status::invalid_argument(code),
            "connection_unavailable" | "credential_store_unavailable" | "discovery_unavailable" => {
                Status::unavailable(code)
            }
            _ => Status::unavailable("unavailable"),
        },
        _ => Status::unavailable("unavailable"),
    }
}

macro_rules! operator_service {
    ($($name:ident($($field:ident),*) => ($method:ident, $path:literal, $page:literal, $pending:literal);)+) => {
        #[tonic::async_trait]
        impl OperatorService for Operator {
            $(async fn $name(&self, request: Request<OperatorRequest>)
                -> Result<Response<OperatorResponse>, Status>
            {
                let input = request.into_inner();
                // Parse before using any caller-supplied selector in the domain dispatcher.
                $(let $field = id(&input.$field).map_err(status)?;)*
                let path = format!($path $(, $field)*);
                call(self, input, Method::$method, &path, $page, $pending).await
            })+
        }
    };
}

operator_service! {
    health() => (GET, "/api/health", false, false);
    discover_connection() => (POST, "/api/connections/jellyfin/discover", false, false);
    save_connection() => (POST, "/api/connections/jellyfin", false, false);
    list_connections() => (GET, "/api/connections/jellyfin", true, false);
    list_sources() => (GET, "/api/sources", true, false);
    list_libraries(source_id) => (GET, "/api/sources/{}/libraries", true, false);
    list_runs(source_id) => (GET, "/api/indexing/sources/{}/runs", true, false);
    list_candidates(source_id, run_id) => (GET, "/api/indexing/sources/{}/runs/{}/candidates", true, true);
    list_policies(source_id) => (GET, "/api/indexing/sources/{}/policies", true, false);
    configure_policy(source_id) => (POST, "/api/indexing/sources/{}/policies", false, false);
    get_progress(source_id, policy_id) => (GET, "/api/indexing/sources/{}/policies/{}/progress", false, false);
    start_run(source_id) => (POST, "/api/indexing/sources/{}/start", false, false);
    get_run_status(source_id, run_id) => (GET, "/api/indexing/sources/{}/runs/{}", false, false);
    get_execution(source_id, run_id) => (GET, "/api/indexing/sources/{}/runs/{}/execution", false, false);
    pause_run(source_id, run_id) => (POST, "/api/indexing/sources/{}/runs/{}/pause", false, false);
    resume_run(source_id, run_id) => (POST, "/api/indexing/sources/{}/runs/{}/resume", false, false);
    cancel_run(source_id, run_id) => (POST, "/api/indexing/sources/{}/runs/{}/cancel", false, false);
    list_jobs(source_id, run_id) => (GET, "/api/indexing/sources/{}/runs/{}/jobs", true, false);
    list_attempts(source_id, run_id, job_id) => (GET, "/api/indexing/sources/{}/runs/{}/jobs/{}/attempts", true, false);
    list_redrives(source_id, run_id, job_id) => (GET, "/api/indexing/sources/{}/runs/{}/jobs/{}/redrives", true, false);
    redrive_job(source_id, run_id, job_id) => (POST, "/api/indexing/sources/{}/runs/{}/jobs/{}/redrive", false, false);
    observe_queue() => (POST, "/api/indexing/queue", false, false);
}

pub async fn serve(
    service: Operator,
    listener: tokio::net::TcpListener,
    shutdown: impl Future<Output = ()> + Send + 'static,
    origins: Vec<axum::http::HeaderValue>,
) -> Result<(), tonic::transport::Error> {
    use axum::http::HeaderName;
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
        .expose_headers(
            ["grpc-status", "grpc-message", "x-screenroom-run-id"].map(HeaderName::from_static),
        );
    tonic::transport::Server::builder()
        .accept_http1(true)
        .timeout(std::time::Duration::from_secs(30))
        .layer(cors)
        .layer(tonic_web::GrpcWebLayer::new())
        .add_service(
            OperatorServiceServer::new(service)
                .max_decoding_message_size(BODY_LIMIT + 4096)
                .max_encoding_message_size(RESPONSE_LIMIT + 4096),
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
    use screenroom_proto::operator::v1::operator_service_client::OperatorServiceClient;

    #[tokio::test]
    async fn shared_rpc_listener_rejects_invalid_requests_before_database_access() {
        let pool = screenroom_backend::sqlx::postgres::PgPoolOptions::new().connect_lazy_with(
            screenroom_backend::sqlx::postgres::PgConnectOptions::new()
                .socket("/nonexistent-screenroom-operator-test"),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (stop, shutdown) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(serve(
            Operator::new_rpc(pool),
            listener,
            async {
                let _ = shutdown.await;
            },
            vec![],
        ));
        let channel = tonic::transport::Endpoint::from_shared(endpoint)
            .unwrap()
            .connect()
            .await
            .unwrap();
        let mut client = OperatorServiceClient::new(channel);
        let invalid_selector = client
            .list_libraries(OperatorRequest {
                source_id: "not-a-uuid".into(),
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert_eq!(invalid_selector.code(), tonic::Code::InvalidArgument);
        let oversized = client
            .discover_connection(OperatorRequest {
                payload_json: vec![b' '; BODY_LIMIT + 1],
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert_eq!(oversized.code(), tonic::Code::InvalidArgument);
        assert_eq!(oversized.message(), "invalid");
        drop(client);
        stop.send(()).unwrap();
        server.await.unwrap().unwrap();
    }
}
