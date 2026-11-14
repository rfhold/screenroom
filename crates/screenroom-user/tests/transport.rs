use screenroom_backend::sqlx::postgres::PgPoolOptions;
use screenroom_proto::user::v1::{GetCurrentUserRequest, user_service_client::UserServiceClient};
use screenroom_user::{UserService, repository::Repository};
use tonic::{Code, Request};

#[tokio::test]
async fn native_and_grpcweb_public_routes_exclude_private_services() {
    let pool = PgPoolOptions::new()
        .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
        .unwrap();
    let service = UserService {
        connections: screenroom_backend::connections::ConnectionRepository::new(pool.clone(), None),
        repository: Repository::new(pool),
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(screenroom_user::serve(
        service.clone(),
        listener,
        async {
            let _ = stopped.await;
        },
        vec!["http://localhost:5173".parse().unwrap()],
    ));
    let endpoint = format!("http://{address}");
    let channel = tonic::transport::Endpoint::from_shared(endpoint.clone())
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut public = UserServiceClient::new(channel.clone());
    assert_eq!(
        public
            .get_current_user(Request::new(GetCurrentUserRequest {}))
            .await
            .unwrap_err()
            .code(),
        Code::InvalidArgument
    );
    let http = reqwest::Client::new();
    let response = http
        .post(format!(
            "{endpoint}/screenroom.user.v1.UserService/GetCurrentUser"
        ))
        .header("content-type", "application/grpc-web+proto")
        .header("x-grpc-web", "1")
        .body(vec![0u8; 5])
        .send()
        .await
        .unwrap();
    assert!(response.status().is_success());
    let headers = response.headers().clone();
    let body = response.bytes().await.unwrap();
    assert!(
        headers.get("grpc-status").is_some_and(|v| v == "3")
            || String::from_utf8_lossy(&body).contains("grpc-status:3")
            || String::from_utf8_lossy(&body).contains("grpc-status: 3")
    );
    let response = http
        .request(
            reqwest::Method::OPTIONS,
            format!("{endpoint}/screenroom.user.v1.UserService/GetCurrentUser"),
        )
        .header("origin", "http://localhost:5173")
        .header("access-control-request-method", "POST")
        .header(
            "access-control-request-headers",
            "x-screenroom-user-id,content-type",
        )
        .send()
        .await
        .unwrap();
    assert!(
        response.headers()["access-control-allow-headers"]
            .to_str()
            .unwrap()
            .contains("x-screenroom-user-id")
    );
    for route in [
        "screenroom.user.operator.v1.JellyfinUserService/SetLink",
        "screenroom.user.operator.v1.JellyfinUserService/ClearLink",
        "screenroom.user.internal.v1.PlaybackStateService/GetContinueWatching",
    ] {
        let response = http
            .post(format!("{endpoint}/{route}"))
            .header("content-type", "application/grpc-web+proto")
            .header("x-grpc-web", "1")
            .body(vec![0u8; 5])
            .send()
            .await
            .unwrap();
        let headers = response.headers().clone();
        let body = response.bytes().await.unwrap();
        assert!(
            headers.get("grpc-status").is_some_and(|v| v == "12")
                || String::from_utf8_lossy(&body).contains("grpc-status:12")
                || String::from_utf8_lossy(&body).contains("grpc-status: 12")
        );
    }
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(screenroom_user::serve_private(service, listener, async {
        let _ = stopped.await;
    }));
    let private_http = reqwest::Client::builder()
        .http2_prior_knowledge()
        .build()
        .unwrap();
    for method in ["SetLink", "ClearLink"] {
        let response = private_http
            .post(format!(
                "http://{address}/screenroom.user.operator.v1.JellyfinUserService/{method}"
            ))
            .header("content-type", "application/grpc")
            .header("te", "trailers")
            .body(vec![0u8; 5])
            .send()
            .await
            .unwrap();
        assert_eq!(response.headers()["grpc-status"], "12");
    }
    let channel = tonic::transport::Endpoint::from_shared(format!("http://{address}"))
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut public = UserServiceClient::new(channel);
    assert_eq!(
        public
            .get_current_user(GetCurrentUserRequest {})
            .await
            .unwrap_err()
            .code(),
        Code::Unimplemented
    );
    stop.send(()).unwrap();
    task.await.unwrap().unwrap();
}
