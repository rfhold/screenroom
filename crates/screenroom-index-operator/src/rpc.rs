use super::{BODY_LIMIT, Command, Error, Operator, Page, default_limit, execute, id};
use http::{HeaderName, HeaderValue, Method};
use screenroom_proto::operator::v1::{
    OperatorRequest, OperatorResponse,
    operator_service_server::{OperatorService, OperatorServiceServer},
};
use tonic::{Request, Response, Status};

const RESPONSE_LIMIT: usize = 2 * 1024 * 1024;

async fn call(
    op: &Operator,
    input: OperatorRequest,
    command: Command,
    read_only: bool,
    paginated: bool,
    pending: bool,
) -> Result<Response<OperatorResponse>, Status> {
    if input.payload_json.len() > BODY_LIMIT
        || read_only && !input.payload_json.is_empty()
        || input.page.is_some() != paginated
        || input.page.as_ref().is_some_and(|p| {
            p.pending_only.is_some() != pending
                || !(1..=1000).contains(&p.limit)
                || p.after
                    .as_ref()
                    .is_some_and(|a| a.is_empty() || a.len() > 200)
        })
    {
        return Err(Status::invalid_argument("invalid"));
    }
    let page = input.page.map_or(
        Page {
            limit: default_limit(),
            after: None,
            pending_only: false,
        },
        |page| Page {
            limit: page.limit,
            after: page.after,
            pending_only: page.pending_only.unwrap_or_default(),
        },
    );
    let result = execute(op, command, page, &input.payload_json)
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
    ($($name:ident($($field:ident),*) => ($command:expr, $read:literal, $page:literal, $pending:literal);)+) => {
        #[tonic::async_trait]
        impl OperatorService for Operator {
            $(async fn $name(&self, request: Request<OperatorRequest>)
                -> Result<Response<OperatorResponse>, Status>
            {
                let input = request.into_inner();
                // Parse before using any caller-supplied selector in the domain dispatcher.
                $(let $field = id(&input.$field).map_err(status)?;)*
                call(self, input, $command, $read, $page, $pending).await
            })+
        }
    };
}

operator_service! {
    health() => (Command::Health, true, false, false);
    discover_connection() => (Command::DiscoverConnection, false, false, false);
    save_connection() => (Command::SaveConnection, false, false, false);
    list_connections() => (Command::ListConnections, true, true, false);
    list_sources() => (Command::ListSources, true, true, false);
    list_libraries(source_id) => (Command::ListLibraries(source_id), true, true, false);
    list_runs(source_id) => (Command::ListRuns(source_id), true, true, false);
    list_candidates(source_id, run_id) => (Command::ListCandidates(source_id, run_id), true, true, true);
    list_policies(source_id) => (Command::ListPolicies(source_id), true, true, false);
    configure_policy(source_id) => (Command::ConfigurePolicy(source_id), false, false, false);
    get_progress(source_id, policy_id) => (Command::GetProgress(source_id, policy_id), true, false, false);
    start_run(source_id) => (Command::StartRun(source_id), false, false, false);
    get_run_status(source_id, run_id) => (Command::GetRunStatus(source_id, run_id), true, false, false);
    get_execution(source_id, run_id) => (Command::GetExecution(source_id, run_id), true, false, false);
    pause_run(source_id, run_id) => (Command::PauseRun(source_id, run_id), false, false, false);
    resume_run(source_id, run_id) => (Command::ResumeRun(source_id, run_id), false, false, false);
    cancel_run(source_id, run_id) => (Command::CancelRun(source_id, run_id), false, false, false);
    list_jobs(source_id, run_id) => (Command::ListJobs(source_id, run_id), true, true, false);
    list_attempts(source_id, run_id, job_id) => (Command::ListAttempts(source_id, run_id, job_id), true, true, false);
    list_redrives(source_id, run_id, job_id) => (Command::ListRedrives(source_id, run_id, job_id), true, true, false);
    redrive_job(source_id, run_id, job_id) => (Command::RedriveJob(source_id, run_id, job_id), false, false, false);
    observe_queue() => (Command::ObserveQueue, false, false, false);
}

pub async fn serve(
    service: Operator,
    listener: tokio::net::TcpListener,
    shutdown: impl Future<Output = ()> + Send + 'static,
    origins: Vec<HeaderValue>,
) -> Result<(), tonic::transport::Error> {
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
        .layer(screenroom_observability::telemetry::rpc_layer())
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
    use screenroom_proto::operator::v1::{
        OperatorPage, operator_service_client::OperatorServiceClient,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn http(address: std::net::SocketAddr, headers: &str, body: &[u8]) -> String {
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
            stream.write_all(headers.as_bytes()).await.unwrap();
            stream.write_all(body).await.unwrap();
            let mut response = Vec::new();
            stream.read_to_end(&mut response).await.unwrap();
            String::from_utf8_lossy(&response).to_ascii_lowercase()
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn shared_rpc_listener_rejects_invalid_requests_before_database_access() {
        let pool = screenroom_backend::sqlx::postgres::PgPoolOptions::new().connect_lazy_with(
            screenroom_backend::sqlx::postgres::PgConnectOptions::new()
                .socket("/nonexistent-screenroom-operator-test"),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let endpoint = format!("http://{address}");
        let (stop, shutdown) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(serve(
            Operator::new_rpc(pool),
            listener,
            async {
                let _ = shutdown.await;
            },
            vec!["http://127.0.0.1:4173".parse().unwrap()],
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
        for limit in [0, 1001] {
            let invalid = client
                .list_sources(OperatorRequest {
                    page: Some(OperatorPage {
                        limit,
                        ..Default::default()
                    }),
                    ..Default::default()
                })
                .await
                .unwrap_err();
            assert_eq!(invalid.code(), tonic::Code::InvalidArgument);
        }
        let invalid = client
            .start_run(OperatorRequest {
                source_id: "00000000-0000-0000-0000-000000000001".into(),
                payload_json: br#"{"trigger":"scheduled"}"#.to_vec(),
                ..Default::default()
            })
            .await
            .unwrap_err();
        assert_eq!(invalid.code(), tonic::Code::InvalidArgument);
        let legacy = http(
            address,
            "POST /api/health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\
             Content-Type: application/grpc-web+proto\r\nX-Grpc-Web: 1\r\nContent-Length: 5\r\n\r\n",
            &[0; 5],
        )
        .await;
        assert!(
            legacy.starts_with("http/1.1 404") || legacy.contains("grpc-status: 12"),
            "{legacy:?}"
        );
        assert!(!legacy.contains("application/json"));

        // An empty ListSources request lacks the required page.
        for origin in ["http://127.0.0.1:4173", "http://127.0.0.1:4174"] {
            let web = http(
                address,
                &format!(
                    "POST /screenroom.operator.v1.OperatorService/ListSources HTTP/1.1\r\n\
                    Host: localhost\r\nConnection: close\r\nOrigin: {origin}\r\n\
                    Content-Type: application/grpc-web+proto\r\nX-Grpc-Web: 1\r\nContent-Length: 5\r\n\r\n"
                ),
                &[0; 5],
            )
            .await;
            assert!(web.contains("grpc-status: 3"), "{web:?}");
            assert_eq!(
                web.contains("access-control-allow-origin: http://127.0.0.1:4173"),
                origin.ends_with(":4173"),
            );
        }
        drop(client);
        stop.send(()).unwrap();
        server.await.unwrap().unwrap();
    }
}
