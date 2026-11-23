#![cfg(not(target_arch = "wasm32"))]
use screenroom_client_core::*;
use screenroom_proto::playback_gateway::v1 as w;
use std::sync::{Arc, Mutex};
use tonic::{Request, Response, Status};

const ID: &str = "11111111-1111-4111-8111-111111111111";
#[derive(Clone, Default)]
struct Double {
    creates: Arc<Mutex<Vec<w::CreatePlaybackSessionRequest>>>,
}
fn session() -> w::PlaybackSession {
    w::PlaybackSession {
        session_id: ID.into(),
        control_capability: "a".repeat(64),
        state: 2,
        report_outcome: 1,
        transport_revision: 1,
        ..Default::default()
    }
}
#[tonic::async_trait]
impl w::playback_service_server::PlaybackService for Double {
    async fn create_playback_session(
        &self,
        r: Request<w::CreatePlaybackSessionRequest>,
    ) -> Result<Response<w::CreatePlaybackSessionResponse>, Status> {
        assert!(r.metadata().contains_key("x-screenroom-user-id"));
        let request = r.into_inner();
        self.creates.lock().unwrap().push(request.clone());
        if request.item_id == "ambiguous" {
            return Err(Status::unavailable("private-secret"));
        }
        if request.item_id == "authorization_unavailable" {
            return Err(Status::unavailable("playback_authorization_unavailable"));
        }
        tokio::time::sleep(std::time::Duration::from_millis(80)).await;
        Ok(Response::new(w::CreatePlaybackSessionResponse {
            session: Some(session()),
        }))
    }
    async fn get_playback_session(
        &self,
        _: Request<w::GetPlaybackSessionRequest>,
    ) -> Result<Response<w::GetPlaybackSessionResponse>, Status> {
        Ok(Response::new(w::GetPlaybackSessionResponse {
            session: None,
        }))
    }
    async fn update_playback_session(
        &self,
        _: Request<w::UpdatePlaybackSessionRequest>,
    ) -> Result<Response<w::UpdatePlaybackSessionResponse>, Status> {
        Err(Status::failed_precondition("profile_changed"))
    }
    async fn report_playback_event(
        &self,
        _: Request<w::ReportPlaybackEventRequest>,
    ) -> Result<Response<w::ReportPlaybackEventResponse>, Status> {
        Err(Status::internal("private provider URL"))
    }
    async fn stop_playback_session(
        &self,
        _: Request<w::StopPlaybackSessionRequest>,
    ) -> Result<Response<w::StopPlaybackSessionResponse>, Status> {
        Ok(Response::new(w::StopPlaybackSessionResponse {
            session: Some(session()),
        }))
    }
}
#[tokio::test]
async fn dedicated_deadline_no_mutation_replay_exact_explicit_retry_and_cancellation() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let double = Double::default();
    let server = double.clone();
    let incoming = futures::stream::unfold(listener, |listener| async {
        Some((listener.accept().await.map(|(s, _)| s), listener))
    });
    let task = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(w::playback_service_server::PlaybackServiceServer::new(
                server,
            ))
            .serve_with_incoming(incoming)
            .await
            .unwrap();
    });
    let client = Client::new(ClientConfig {
        endpoint,
        user_id: ID.into(),
        timeout_ms: 10,
    })
    .unwrap();
    let mut request = CreatePlaybackSessionRequest {
        request_id: ID.into(),
        item_id: "movie".into(),
        client_capabilities: Some(PlaybackClientCapabilities {
            mp4: true,
            h264: true,
            aac: true,
            ..Default::default()
        }),
        start_policy: PlaybackStartPolicy::Position,
        position_ms: Some("922337203685477".into()),
        ..Default::default()
    };
    assert!(
        client
            .clone()
            .create_playback_session(request.clone())
            .await
            .unwrap()
            .session
            .is_some()
    );
    assert_eq!(
        double.creates.lock().unwrap()[0].position_ms.as_deref(),
        Some("922337203685477")
    );
    assert_eq!(double.creates.lock().unwrap()[0].start_policy, 3);
    request.item_id = "ambiguous".into();
    for _ in 0..2 {
        assert!(matches!(
            client
                .clone()
                .create_playback_session(request.clone())
                .await,
            Err(ClientError::PlaybackUncertain)
        ));
    }
    let requests = double.creates.lock().unwrap().clone();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[1], requests[2]);
    assert!(matches!(
        client
            .clone()
            .get_playback_session(GetPlaybackSessionRequest {
                session_id: ID.into(),
                control_capability: "a".repeat(64)
            })
            .await,
        Err(ClientError::InvalidResponse { .. })
    ));
    assert!(matches!(
        client
            .clone()
            .update_playback_session(UpdatePlaybackSessionRequest {
                session_id: ID.into(),
                control_capability: "a".repeat(64),
                request_id: ID.into(),
                expected_transport_revision: 1,
                position_ms: "0".into(),
                ..Default::default()
            })
            .await,
        Err(ClientError::Playback {
            code: PlaybackErrorCode::ProfileChanged
        })
    ));
    request.item_id = "cancel".into();
    let pending = tokio::spawn(client.clone().create_playback_session(request.clone()));
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    client.cancel_all();
    assert!(matches!(
        pending.await.unwrap(),
        Err(ClientError::PlaybackUncertain)
    ));
    let pending = tokio::spawn(client.clone().create_playback_session(request));
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    let _ = client.clone().set_jellyfin_link(ID.into(), ID.into()).await;
    assert!(matches!(
        pending.await.unwrap(),
        Err(ClientError::PlaybackUncertain)
    ));
    task.abort();
}

#[tokio::test]
async fn create_unavailable_prerequisite_returns_typed_error_not_mutation_uncertainty() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let double = Double::default();
    let server = double.clone();
    let incoming = futures::stream::unfold(listener, |listener| async {
        Some((listener.accept().await.map(|(s, _)| s), listener))
    });
    let task = tokio::spawn(async move {
        tonic::transport::Server::builder()
            .add_service(w::playback_service_server::PlaybackServiceServer::new(
                server,
            ))
            .serve_with_incoming(incoming)
            .await
            .unwrap();
    });
    let client = Client::new(ClientConfig {
        endpoint,
        user_id: ID.into(),
        timeout_ms: 10,
    })
    .unwrap();
    let error = client
        .create_playback_session(CreatePlaybackSessionRequest {
            request_id: ID.into(),
            item_id: "authorization_unavailable".into(),
            client_capabilities: Some(PlaybackClientCapabilities {
                mp4: true,
                h264: true,
                aac: true,
                ..Default::default()
            }),
            start_policy: PlaybackStartPolicy::Resume,
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert!(
        matches!(
            error,
            ClientError::Playback {
                code: PlaybackErrorCode::PlaybackAuthorizationUnavailable
            }
        ),
        "{error:?}"
    );
    assert_eq!(double.creates.lock().unwrap().len(), 1);
    task.abort();
}
