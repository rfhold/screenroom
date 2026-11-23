use crate::PlaybackService;
use screenroom_backend::{
    connections::{ConnectionRepository, CredentialDirectory},
    playback::{PlaybackRepository, secrets::PlaybackSecrets},
};
use screenroom_proto::playback_gateway::v1::*;
use std::time::Duration;
use tonic::Request;

mod database;

pub(crate) fn service() -> (PlaybackService, sqlx::PgPool) {
    // No acquisition occurs in these boundary tests. No environment or database target is read.
    let pool = sqlx::postgres::PgPoolOptions::new()
        .min_connections(0)
        .max_connections(1)
        .connect_lazy_with(
            sqlx::postgres::PgConnectOptions::new()
                .socket("/nonexistent-screenroom-isolated-tests"),
        );
    let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
        "playback-unused-{}",
        screenroom_backend::Uuid::new_v4()
    ));
    let credentials = CredentialDirectory::new(root.join("managed")).unwrap();
    let secrets = PlaybackSecrets::new(root.join("playback")).unwrap();
    let repository = PlaybackRepository::new(pool.clone(), Default::default()).unwrap();
    let connections = ConnectionRepository::new(pool.clone(), Some(credentials.clone()));
    (
        PlaybackService::new(
            repository,
            connections,
            credentials,
            secrets,
            "https://api.test".into(),
        )
        .unwrap(),
        pool,
    )
}

#[tokio::test]
async fn five_real_rpc_implementations_reject_missing_selector_before_storage() {
    use playback_service_server::PlaybackService as _;
    let (service, pool) = service();
    let errors = [
        service
            .create_playback_session(Request::new(CreatePlaybackSessionRequest::default()))
            .await
            .unwrap_err(),
        service
            .get_playback_session(Request::new(GetPlaybackSessionRequest::default()))
            .await
            .unwrap_err(),
        service
            .update_playback_session(Request::new(UpdatePlaybackSessionRequest::default()))
            .await
            .unwrap_err(),
        service
            .report_playback_event(Request::new(ReportPlaybackEventRequest::default()))
            .await
            .unwrap_err(),
        service
            .stop_playback_session(Request::new(StopPlaybackSessionRequest::default()))
            .await
            .unwrap_err(),
    ];
    for error in errors {
        assert_eq!(error.code(), tonic::Code::InvalidArgument);
    }
    assert_eq!(pool.size(), 0);
}

#[tokio::test]
async fn five_real_rpc_implementations_validate_bodies_before_storage() {
    use playback_service_server::PlaybackService as _;
    fn selected<T>(body: T) -> Request<T> {
        let mut request = Request::new(body);
        request.metadata_mut().insert(
            "x-screenroom-user-id",
            screenroom_backend::Uuid::new_v4()
                .to_string()
                .parse()
                .unwrap(),
        );
        request
    }
    let (service, pool) = service();
    let errors = [
        service
            .create_playback_session(selected(CreatePlaybackSessionRequest::default()))
            .await
            .unwrap_err(),
        service
            .get_playback_session(selected(GetPlaybackSessionRequest::default()))
            .await
            .unwrap_err(),
        service
            .update_playback_session(selected(UpdatePlaybackSessionRequest::default()))
            .await
            .unwrap_err(),
        service
            .report_playback_event(selected(ReportPlaybackEventRequest::default()))
            .await
            .unwrap_err(),
        service
            .stop_playback_session(selected(StopPlaybackSessionRequest::default()))
            .await
            .unwrap_err(),
    ];
    for error in errors {
        assert_eq!(error.code(), tonic::Code::InvalidArgument);
    }
    assert_eq!(pool.size(), 0);
}

#[tokio::test]
async fn media_and_control_cors_have_exact_distinct_allowlists() {
    use axum::{
        body::Body,
        http::{Method, Request, header},
    };
    use tower::ServiceExt;
    let (service, pool) = service();
    let app = crate::transport::router(
        service,
        vec![
            "https://spa.test".parse().unwrap(),
            "https://other.test".parse().unwrap(),
        ],
    );
    let media = "/playback/v1/media/11111111-1111-4111-8111-111111111111/1/22222222-2222-4222-8222-222222222222";
    let control = "/screenroom.playback_gateway.v1.PlaybackService/CreatePlaybackSession";
    for origin in ["https://spa.test", "https://other.test"] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::OPTIONS)
                    .uri(media)
                    .header(header::ORIGIN, origin)
                    .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                    .header(header::ACCESS_CONTROL_REQUEST_HEADERS, "range")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
            origin
        );
        let methods = response.headers()[header::ACCESS_CONTROL_ALLOW_METHODS]
            .to_str()
            .unwrap();
        assert!(methods.contains("GET") && methods.contains("HEAD") && !methods.contains("POST"));
        assert_eq!(
            response.headers()[header::ACCESS_CONTROL_ALLOW_HEADERS],
            "range,if-range"
        );
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(response.headers()["referrer-policy"], "no-referrer");
        assert!(
            response
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_CREDENTIALS)
                .is_none()
        );

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::OPTIONS)
                    .uri(control)
                    .header(header::ORIGIN, origin)
                    .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                    .header(
                        header::ACCESS_CONTROL_REQUEST_HEADERS,
                        "x-screenroom-user-id,content-type",
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.headers()[header::ACCESS_CONTROL_ALLOW_METHODS],
            "POST"
        );
        assert!(
            response.headers()[header::ACCESS_CONTROL_ALLOW_HEADERS]
                .to_str()
                .unwrap()
                .contains("x-screenroom-user-id")
        );
    }
    for path in [media, control] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::OPTIONS)
                    .uri(path)
                    .header(header::ORIGIN, "https://not-allowed.test")
                    .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 403);
        assert!(
            response
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .is_none()
        );
    }
    assert_eq!(pool.size(), 0);
}

#[tokio::test]
async fn media_authority_query_and_multiranges_are_rejected_before_storage() {
    use axum::{
        body::{Body, to_bytes},
        http::{Method, Request},
    };
    use tower::ServiceExt;
    let (service, pool) = service();
    let app = crate::transport::router(service, vec![]);
    let path = "/playback/v1/media/11111111-1111-4111-8111-111111111111/1/22222222-2222-4222-8222-222222222222";
    for query in [
        "".into(),
        "?cap=wrong".into(),
        format!("?cap={}&other=token", "a".repeat(64)),
        format!("?url=https://private&cap={}", "a".repeat(64)),
    ] {
        for method in [Method::GET, Method::HEAD] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(format!("{path}{query}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), 404);
            let body = to_bytes(response.into_body(), 4096).await.unwrap();
            assert!(!String::from_utf8_lossy(&body).contains("token"));
        }
    }
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("{path}?cap={}", "a".repeat(64)))
                .header("range", "bytes=0-1,5-6")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 416);
    assert_eq!(pool.size(), 0);
}

#[tokio::test]
async fn unknown_and_query_bearing_routes_never_enter_rpc_tracing() {
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;
    let (service, pool) = service();
    let app = crate::transport::router(service, vec![]);
    for path in [
        "/playback/v1/media/wrong?cap=private",
        "/screenroom.playback_gateway.v1.PlaybackService/GetPlaybackSession?cap=private",
        "/arbitrary?cap=private",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 404);
    }
    assert_eq!(pool.size(), 0);
}

#[tokio::test]
async fn actual_tonic_loopback_rejects_all_five_unselected_methods() {
    let (service, pool) = service();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(crate::serve(
        service,
        listener,
        async {
            let _ = stopped.await;
        },
        vec![],
    ));
    let channel = tonic::transport::Endpoint::from_shared(format!("http://{address}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut client = playback_service_client::PlaybackServiceClient::new(channel);
    let errors = [
        client
            .create_playback_session(CreatePlaybackSessionRequest::default())
            .await
            .unwrap_err(),
        client
            .get_playback_session(GetPlaybackSessionRequest::default())
            .await
            .unwrap_err(),
        client
            .update_playback_session(UpdatePlaybackSessionRequest::default())
            .await
            .unwrap_err(),
        client
            .report_playback_event(ReportPlaybackEventRequest::default())
            .await
            .unwrap_err(),
        client
            .stop_playback_session(StopPlaybackSessionRequest::default())
            .await
            .unwrap_err(),
    ];
    for error in errors {
        assert_eq!(error.code(), tonic::Code::InvalidArgument);
    }
    drop(client);
    stop.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(pool.size(), 0);
}
