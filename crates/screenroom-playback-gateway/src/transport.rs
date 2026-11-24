use crate::{PlaybackService, fences::MediaFence, validation as v};
use axum::{
    Router,
    body::Body,
    extract::{Path, RawQuery, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use screenroom_catalog_adapter::jellyfin::playback as provider;
use std::{sync::Arc, time::Duration};
use tonic::Status;

#[derive(Debug)]
pub(crate) struct MediaDiagnostic {
    start: tokio::time::Instant,
    stage: &'static str,
    code: &'static str,
    sqlstate: Option<String>,
    children: usize,
    registered: usize,
}

pub(crate) type MediaDiagnostics = Arc<std::sync::Mutex<MediaDiagnostic>>;

impl MediaDiagnostic {
    fn new() -> Self {
        Self {
            start: tokio::time::Instant::now(),
            stage: "request_validation",
            code: "pending",
            sqlstate: None,
            children: 0,
            registered: 0,
        }
    }

    fn log(&self, status: StatusCode) {
        let code = if self.code == "pending" {
            match status {
                StatusCode::NOT_FOUND => "not_found",
                StatusCode::TOO_MANY_REQUESTS => "capacity",
                StatusCode::BAD_REQUEST | StatusCode::RANGE_NOT_SATISFIABLE => "invalid_request",
                _ => "unavailable",
            }
        } else {
            self.code
        };
        tracing::warn!(
            component = "playback_media",
            stage = self.stage,
            error_code = code,
            sqlstate = self.sqlstate.as_deref().unwrap_or(""),
            status = status.as_u16(),
            elapsed_ms = crate::runtime::elapsed_ms(self.start),
            children_total = self.children.min(8193),
            children_registered = self.registered.min(8193),
            "playback media unavailable"
        );
    }
}

fn media_stage(diagnostic: &MediaDiagnostics, stage: &'static str) {
    let mut diagnostic = diagnostic.lock().expect("media diagnostic");
    diagnostic.stage = stage;
    diagnostic.code = "pending";
    diagnostic.sqlstate = None;
}

pub(crate) fn media_failure(
    diagnostic: &MediaDiagnostics,
    stage: &'static str,
    code: &'static str,
) {
    let mut diagnostic = diagnostic.lock().expect("media diagnostic");
    diagnostic.stage = stage;
    diagnostic.code = code;
    diagnostic.sqlstate = None;
}

pub(crate) fn media_backend_failure(
    diagnostic: &MediaDiagnostics,
    stage: &'static str,
    error: &screenroom_backend::Error,
) {
    let (code, sqlstate) = crate::runtime::backend_failure(error);
    let mut diagnostic = diagnostic.lock().expect("media diagnostic");
    diagnostic.stage = stage;
    diagnostic.code = code;
    diagnostic.sqlstate = sqlstate;
}

fn media_provider_failure(diagnostic: &MediaDiagnostics, failure: &provider::Failure) {
    use provider::Failure;
    let code = match failure {
        Failure::InvalidInput => "provider_invalid_input",
        Failure::Identity => "provider_identity",
        Failure::AuthorizationUnavailable => "provider_authorization_unavailable",
        Failure::Unauthorized => "provider_unauthorized",
        Failure::Unsupported => "provider_unsupported",
        Failure::ResourceRejected => "provider_resource_rejected",
        Failure::Bounds => "provider_bounds",
        Failure::Deadline => "provider_deadline",
        Failure::Transport => "provider_transport",
        Failure::Http(_) => "provider_http",
        Failure::InvalidResponse => "provider_invalid_response",
        Failure::FenceDenied | Failure::FenceUnavailable => {
            let mut diagnostic = diagnostic.lock().expect("media diagnostic");
            if diagnostic.code == "pending" {
                diagnostic.stage = "dispatch_fence";
                diagnostic.code = if *failure == Failure::FenceDenied {
                    "revoked"
                } else {
                    "unavailable"
                };
            }
            return;
        }
    };
    diagnostic.lock().expect("media diagnostic").code = code;
}

pub(crate) fn validate_origin(value: &str) -> Result<(), Status> {
    let uri = value.parse::<axum::http::Uri>().map_err(|_| v::invalid())?;
    if !matches!(uri.scheme_str(), Some("http" | "https"))
        || uri.authority().is_none_or(|a| a.as_str().contains('@'))
        || uri.path_and_query().is_some_and(|p| p.as_str() != "/")
        || value.contains(['#', '?'])
    {
        return Err(v::invalid());
    }
    Ok(())
}

fn cap(query: Option<String>) -> Result<crate::signed_resources::ResourceQuery, StatusCode> {
    crate::signed_resources::ResourceQuery::parse(query.as_deref())
}

async fn origin_guard(
    State(origins): State<Arc<Vec<HeaderValue>>>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let mut supplied = request.headers().get_all(header::ORIGIN).iter();
    if let Some(origin) = supplied.next()
        && (supplied.next().is_some() || !origins.contains(origin))
    {
        return secured(StatusCode::FORBIDDEN.into_response());
    }
    secured(next.run(request).await)
}

fn secured(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response
}

async fn control_boundary(request: axum::extract::Request, next: Next) -> Response {
    let path = request.uri().path();
    let method = path.strip_prefix("/screenroom.playback_gateway.v1.PlaybackService/");
    if request.uri().query().is_some()
        || !matches!(
            method,
            Some(
                "CreatePlaybackSession"
                    | "GetPlaybackSession"
                    | "UpdatePlaybackSession"
                    | "ReportPlaybackEvent"
                    | "StopPlaybackSession"
            )
        )
        || !matches!(*request.method(), Method::POST | Method::OPTIONS)
    {
        return StatusCode::NOT_FOUND.into_response();
    }
    next.run(request).await
}

async fn control_deadline(mut request: axum::extract::Request, next: Next) -> Response {
    let duration = Duration::from_secs(
        if request.uri().path().ends_with("/CreatePlaybackSession") {
            30
        } else {
            10
        },
    );
    request
        .extensions_mut()
        .insert(v::Deadline(tokio::time::Instant::now() + duration));
    tokio::time::timeout(duration, next.run(request))
        .await
        .unwrap_or_else(|_| Status::deadline_exceeded("upstream_unavailable").into_http::<Body>())
}

pub fn router(service: PlaybackService, origins: Vec<HeaderValue>) -> Router {
    use screenroom_proto::playback_gateway::v1::playback_service_server::PlaybackServiceServer;
    let control_cors = tower_http::cors::CorsLayer::new()
        .allow_origin(origins.clone())
        .allow_methods([Method::POST])
        .allow_headers(
            [
                "content-type",
                "x-grpc-web",
                "x-user-agent",
                "grpc-timeout",
                "traceparent",
                "tracestate",
                "x-screenroom-user-id",
            ]
            .map(header::HeaderName::from_static),
        )
        .expose_headers(["grpc-status", "grpc-message"].map(header::HeaderName::from_static));
    let media_cors = tower_http::cors::CorsLayer::new()
        .allow_origin(origins.clone())
        .allow_methods([Method::GET, Method::HEAD, Method::OPTIONS])
        .allow_headers(["range", "if-range"].map(header::HeaderName::from_static))
        .expose_headers(
            [
                "content-type",
                "content-length",
                "content-range",
                "accept-ranges",
                "etag",
                "last-modified",
            ]
            .map(header::HeaderName::from_static),
        );
    let grpc = tonic::service::Routes::new(
        PlaybackServiceServer::new(service.clone())
            .max_decoding_message_size(64 * 1024)
            .max_encoding_message_size(256 * 1024),
    )
    .into_axum_router()
    .layer(screenroom_observability::telemetry::rpc_layer())
    .layer(middleware::from_fn(control_deadline))
    .layer(screenroom_observability::telemetry::RpcStatusLayer)
    .layer(tonic_web::GrpcWebLayer::new())
    .layer(control_cors)
    .layer(middleware::from_fn(control_boundary));
    let media = Router::new()
        .route(
            "/playback/v1/media/{session}/{revision}/{resource}",
            get(media),
        )
        .layer(media_cors)
        .with_state(service.clone());
    Router::new()
        .merge(media)
        .route(
            "/ready",
            get(move || {
                let service = service.clone();
                async move {
                    if service.stopping.load(std::sync::atomic::Ordering::Acquire) {
                        StatusCode::SERVICE_UNAVAILABLE
                    } else {
                        StatusCode::OK
                    }
                }
            }),
        )
        .fallback_service(grpc)
        .layer(middleware::from_fn_with_state(
            Arc::new(origins),
            origin_guard,
        ))
}

async fn media(
    State(service): State<PlaybackService>,
    Path((session, revision, resource)): Path<(String, String, String)>,
    RawQuery(query): RawQuery,
    method: Method,
    headers: HeaderMap,
) -> Response {
    screenroom_observability::telemetry::observe_http_headers(
        &method,
        "/playback/v1/media/{session}/{revision}/{resource}",
        &headers,
        media_response(
            service,
            session,
            revision,
            resource,
            query,
            method.clone(),
            headers.clone(),
        ),
    )
    .await
}

async fn media_response(
    service: PlaybackService,
    session: String,
    revision: String,
    resource: String,
    query: Option<String>,
    method: Method,
    headers: HeaderMap,
) -> Response {
    let diagnostic = Arc::new(std::sync::Mutex::new(MediaDiagnostic::new()));
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        media_inner(
            service,
            session,
            revision,
            resource,
            query,
            (method, headers),
            diagnostic.clone(),
        ),
    )
    .await
    .unwrap_or_else(|_| {
        diagnostic.lock().expect("media diagnostic").code = "operation_timeout";
        Err(StatusCode::GATEWAY_TIMEOUT)
    });
    if let Err(status) = &result {
        diagnostic.lock().expect("media diagnostic").log(*status);
    }
    secured(result.unwrap_or_else(|status| status.into_response()))
}

async fn media_inner(
    service: PlaybackService,
    session: String,
    revision: String,
    resource: String,
    query: Option<String>,
    method_and_headers: (Method, HeaderMap),
    diagnostic: MediaDiagnostics,
) -> Result<Response, StatusCode> {
    let (method, headers) = method_and_headers;
    if service.stopping.load(std::sync::atomic::Ordering::Acquire) {
        media_failure(&diagnostic, "admission", "gateway_stopping");
        return Err(StatusCode::SERVICE_UNAVAILABLE);
    }
    let session = v::uuid(&session).map_err(|_| StatusCode::NOT_FOUND)?;
    let resource = v::uuid(&resource).map_err(|_| StatusCode::NOT_FOUND)?;
    let revision = revision
        .parse::<u32>()
        .ok()
        .filter(|r| *r > 0)
        .ok_or(StatusCode::NOT_FOUND)?;
    let query = cap(query)?;
    let capability = &query.capability;
    media_stage(&diagnostic, "fetch_capacity");
    let permit = service
        .fetches
        .clone()
        .try_acquire_owned()
        .map_err(|_| StatusCode::TOO_MANY_REQUESTS)?;
    media_stage(&diagnostic, "range_validation");
    let range_header = single_header(&headers, header::RANGE)?;
    let range = range_header
        .as_deref()
        .map(|value| {
            provider::ByteRange::parse(value).map_err(|_| StatusCode::RANGE_NOT_SATISFIABLE)
        })
        .transpose()?;
    let if_range = single_header(&headers, header::IF_RANGE)?;
    media_stage(&diagnostic, "runtime_lookup");
    let runtime = service
        .transports
        .read()
        .await
        .get(&(session, revision))
        .cloned()
        .ok_or(StatusCode::NOT_FOUND)?;
    media_stage(&diagnostic, "resource_lookup");
    let (anchor, reference) = runtime.resources.read().await.resolve(
        &runtime.signing_key,
        session,
        revision,
        resource,
        &query,
    )?;
    media_stage(&diagnostic, "repository");
    let (record, _) = service
        .repository
        .media(session, revision, capability, anchor)
        .await
        .map_err(|error| {
            media_backend_failure(&diagnostic, "repository", &error);
            StatusCode::NOT_FOUND
        })?;
    if record.play_session_id.as_deref() != Some(runtime.negotiated.private_play_session_id()) {
        media_failure(&diagnostic, "play_session_match", "mismatch");
        return Err(StatusCode::NOT_FOUND);
    }
    if let Some(context) = &runtime.trace_context {
        screenroom_observability::telemetry::link_current_span(context);
    }
    let fence = Arc::new(MediaFence {
        repository: service.repository.clone(),
        session,
        revision,
        capability: capability.clone(),
        anchor,
        server: runtime.intent.server_id().into(),
        ownership: service.ownership.clone(),
        diagnostic: diagnostic.clone(),
    });
    media_stage(&diagnostic, "adapter_assembly");
    let adapter = runtime
        .provider
        .playback_adapter_fenced(
            runtime.intent.clone(),
            provider::PlaybackTimeouts::default(),
            fence.clone(),
        )
        .map_err(|error| {
            media_provider_failure(&diagnostic, &error.failure);
            StatusCode::BAD_GATEWAY
        })?;
    match reference.kind() {
        provider::ResourceKind::Playlist => {
            media_stage(&diagnostic, "playlist_lock");
            let mut state = runtime.playlists.lock().await;
            if !state.rewritten.contains_key(&resource) {
                let graph = runtime.resources.read().await;
                let mut draft = crate::signed_resources::PlaylistDraft::new(
                    &graph,
                    &runtime.signing_key,
                    crate::signed_resources::Context {
                        session,
                        revision,
                        anchor,
                    },
                    capability,
                );
                let mut budget = state.budget.clone();
                media_stage(&diagnostic, "playlist_fetch_parse");
                let rewritten = checked(
                    fence.clone(),
                    adapter.playlist(&runtime.auth, &reference, &mut budget, |reference| {
                        draft.map(reference)
                    }),
                )
                .await?;
                let pending = draft.finish(rewritten, budget);
                diagnostic.lock().expect("media diagnostic").children = pending.children();
                drop(graph);
                let mut graph = runtime.resources.write().await;
                media_stage(&diagnostic, "playlist_final_fence");
                finalize_playlist(
                    async {
                        fence
                            .validate()
                            .await
                            .map(|_| ())
                            .map_err(|_| StatusCode::NOT_FOUND)
                    },
                    || {
                        media_stage(&diagnostic, "playlist_graph_publication");
                        state.publish(&mut graph, resource, pending)
                    },
                )
                .await?;
            } else {
                media_stage(&diagnostic, "playlist_final_fence");
                finalize_playlist(
                    async {
                        fence
                            .validate()
                            .await
                            .map(|_| ())
                            .map_err(|_| StatusCode::NOT_FOUND)
                    },
                    || Ok(()),
                )
                .await?;
            }
            let bytes = state.rewritten.get(&resource).unwrap().as_bytes();
            let response = buffered(
                bytes,
                method == Method::HEAD,
                "application/vnd.apple.mpegurl",
                range_header.as_deref(),
                if_range.as_deref(),
            );
            drop(permit);
            Ok(response)
        }
        provider::ResourceKind::Subtitle => {
            media_stage(&diagnostic, "subtitle");
            let body = checked(fence.clone(), adapter.subtitle(&runtime.auth, &reference)).await?;
            Ok(buffered(
                &body,
                method == Method::HEAD,
                "text/vtt",
                range_header.as_deref(),
                if_range.as_deref(),
            ))
        }
        _ => {
            media_stage(&diagnostic, "stream_headers");
            let mut upstream = checked(
                fence.clone(),
                adapter.stream(
                    &runtime.auth,
                    &reference,
                    provider::MediaRequest {
                        head: method == Method::HEAD,
                        range,
                        if_range,
                    },
                ),
            )
            .await?;
            let status =
                StatusCode::from_u16(upstream.status).map_err(|_| StatusCode::BAD_GATEWAY)?;
            let response_headers = upstream.headers.clone();
            let (tx, rx) = tokio::sync::mpsc::channel::<Result<bytes::Bytes, std::io::Error>>(1);
            // One task owns upstream consumption and the permit, including while the browser
            // stops reading. Its periodic fence runs during upstream and downstream stalls.
            tokio::spawn(async move {
                let _permit = permit;
                loop {
                    media_stage(&diagnostic, "stream_body");
                    let chunk = checked(fence.clone(), upstream.next_chunk()).await;
                    match chunk {
                        Ok(Some(chunk)) => {
                            media_stage(&diagnostic, "stream_downstream");
                            let send = tx.send(Ok(bytes::Bytes::from(chunk)));
                            if let Err(status) = checked_local(fence.clone(), send).await {
                                diagnostic.lock().expect("media diagnostic").log(status);
                                break;
                            }
                        }
                        Ok(None) => break,
                        Err(status) => {
                            diagnostic.lock().expect("media diagnostic").log(status);
                            let _ = tx.try_send(Err(std::io::Error::other("media unavailable")));
                            break;
                        }
                    }
                }
            });
            let mut response = Response::new(Body::from_stream(
                tokio_stream::wrappers::ReceiverStream::new(rx),
            ));
            *response.status_mut() = status;
            *response.headers_mut() = response_headers;
            Ok(response)
        }
    }
}

pub(crate) async fn finalize_playlist(
    check: impl Future<Output = Result<(), StatusCode>>,
    publish: impl FnOnce() -> Result<(), StatusCode>,
) -> Result<(), StatusCode> {
    check.await?;
    publish()
}

fn single_header(
    headers: &HeaderMap,
    name: header::HeaderName,
) -> Result<Option<String>, StatusCode> {
    let mut values = headers.get_all(name).iter();
    let value = values
        .next()
        .map(|v| {
            v.to_str()
                .map(str::to_owned)
                .map_err(|_| StatusCode::BAD_REQUEST)
        })
        .transpose()?;
    if values.next().is_some() {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok(value)
}

fn buffered(
    bytes: &[u8],
    head: bool,
    mime: &'static str,
    range: Option<&str>,
    if_range: Option<&str>,
) -> Response {
    let (status, selected) = match range {
        _ if head => (StatusCode::OK, 0..bytes.len()),
        Some(value) if provider::ByteRange::parse(value).is_err() => {
            (StatusCode::RANGE_NOT_SATISFIABLE, 0..0)
        }
        // No locally validated strong validator: If-Range requires the full representation.
        Some(value) if if_range.is_none() => match buffered_range(value, bytes.len()) {
            Some(selected) => (StatusCode::PARTIAL_CONTENT, selected),
            None => (StatusCode::RANGE_NOT_SATISFIABLE, 0..0),
        },
        _ => (StatusCode::OK, 0..bytes.len()),
    };
    let mut response = Response::new(if head {
        Body::empty()
    } else {
        Body::from(bytes[selected.clone()].to_vec())
    });
    *response.status_mut() = status;
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    response
        .headers_mut()
        .insert(header::CONTENT_LENGTH, HeaderValue::from(selected.len()));
    response
        .headers_mut()
        .insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    let content_range = match status {
        StatusCode::PARTIAL_CONTENT => Some(format!(
            "bytes {}-{}/{}",
            selected.start,
            selected.end - 1,
            bytes.len()
        )),
        StatusCode::RANGE_NOT_SATISFIABLE => Some(format!("bytes */{}", bytes.len())),
        _ => None,
    };
    if let Some(value) = content_range {
        response.headers_mut().insert(
            header::CONTENT_RANGE,
            HeaderValue::from_str(&value).unwrap(),
        );
    }
    response
}

fn buffered_range(value: &str, total: usize) -> Option<std::ops::Range<usize>> {
    let (start, end) = value.strip_prefix("bytes=")?.split_once('-')?;
    let total = u64::try_from(total).ok()?;
    let (start, end) = if start.is_empty() {
        (total.saturating_sub(end.parse::<u64>().ok()?), total)
    } else {
        let start = start.parse::<u64>().ok()?;
        let end = if end.is_empty() {
            total
        } else {
            end.parse::<u64>().ok()?.saturating_add(1).min(total)
        };
        (start, end)
    };
    if start >= end {
        return None;
    }
    Some(usize::try_from(start).ok()?..usize::try_from(end).ok()?)
}

async fn checked<T>(
    fence: Arc<MediaFence>,
    future: impl Future<Output = provider::Result<T>>,
) -> Result<T, StatusCode> {
    let diagnostic = fence.diagnostic.clone();
    checked_local(fence, async move {
        future.await.map_err(|error| {
            media_provider_failure(&diagnostic, &error.failure);
        })
    })
    .await
}

pub(crate) trait StreamFence: Send + Sync {
    fn check(&self) -> std::pin::Pin<Box<dyn Future<Output = Result<(), StatusCode>> + Send + '_>>;
    fn failed(&self, _code: &'static str) {}
}

impl StreamFence for MediaFence {
    fn failed(&self, code: &'static str) {
        media_failure(&self.diagnostic, "dispatch_fence", code);
    }
    fn check(&self) -> std::pin::Pin<Box<dyn Future<Output = Result<(), StatusCode>> + Send + '_>> {
        Box::pin(async move {
            self.validate()
                .await
                .map(|_| ())
                .map_err(|_| StatusCode::NOT_FOUND)
        })
    }
}

pub(crate) async fn checked_local<T, E, F: StreamFence + ?Sized>(
    fence: Arc<F>,
    future: impl Future<Output = Result<T, E>>,
) -> Result<T, StatusCode> {
    let period = Duration::from_secs(4);
    let interval = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
    checked_local_with_ticks(fence, future, interval).await
}

trait FenceTicks: Send {
    fn tick(&mut self) -> impl Future<Output = ()> + Send;
}

impl FenceTicks for tokio::time::Interval {
    async fn tick(&mut self) {
        tokio::time::Interval::tick(self).await;
    }
}

async fn check_fence<F: StreamFence + ?Sized>(fence: &F) -> Result<(), StatusCode> {
    tokio::time::timeout(Duration::from_millis(750), fence.check())
        .await
        .map_err(|_| {
            fence.failed("check_timeout");
            StatusCode::SERVICE_UNAVAILABLE
        })?
}

async fn checked_local_with_ticks<T, E, F: StreamFence + ?Sized>(
    fence: Arc<F>,
    future: impl Future<Output = Result<T, E>>,
    mut ticks: impl FenceTicks,
) -> Result<T, StatusCode> {
    tokio::pin!(future);
    check_fence(fence.as_ref()).await?;
    let mut checking = None;
    let mut completed = None;
    loop {
        tokio::select! {
            biased;
            result=async { checking.as_mut().expect("pending fence").await }, if checking.is_some()=>{
                checking=None;
                result?;
            }
            _=ticks.tick(), if checking.is_none()=>{
                checking=Some(Box::pin(check_fence(fence.as_ref())));
            }
            result=&mut future, if completed.is_none()=>{
                completed=Some(result);
            }
            _=std::future::ready(()), if completed.is_some() && checking.is_none()=>{
                return completed.take().expect("completed work").map_err(|_|StatusCode::BAD_GATEWAY);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_failure_diagnostics_preserve_stage_and_never_format_private_errors() {
        let capture = crate::runtime::diagnostic_capture::Capture::default();
        let _subscriber = tracing::subscriber::set_default(capture.clone());
        let diagnostic = Arc::new(std::sync::Mutex::new(MediaDiagnostic::new()));
        let private = "http://private-provider/?cap=private-media-token";
        media_backend_failure(
            &diagnostic,
            "playlist_child_registration",
            &screenroom_backend::Error::Connection(private),
        );
        {
            let mut diagnostic = diagnostic.lock().unwrap();
            diagnostic.children = 2859;
            diagnostic.registered = 2049;
            diagnostic.log(StatusCode::NOT_FOUND);
        }
        media_stage(&diagnostic, "playlist_fetch_parse");
        media_provider_failure(&diagnostic, &provider::Failure::Bounds);
        diagnostic.lock().unwrap().log(StatusCode::BAD_GATEWAY);
        media_failure(&diagnostic, "dispatch_fence_repository", "stale");
        media_provider_failure(&diagnostic, &provider::Failure::FenceDenied);
        diagnostic.lock().unwrap().log(StatusCode::NOT_FOUND);
        let events = capture.0.lock().unwrap();
        assert_eq!(events.len(), 3);
        assert!(events[0].contains("playlist_child_registration"));
        assert!(events[0].contains("children_total=2859"));
        assert!(events[0].contains("children_registered=2049"));
        assert!(events[1].contains("playlist_fetch_parse"));
        assert!(events[1].contains("provider_bounds"));
        assert!(events[2].contains("dispatch_fence_repository"));
        assert!(events[2].contains("stale"));
        for event in events.iter() {
            for forbidden in [
                private,
                "private-media-token",
                "session_id",
                "resource_id",
                "profile_id",
                "item_id",
                "device_id",
                "control_capability",
                "media_capability",
                "http://",
                "?cap=",
            ] {
                assert!(!event.contains(forbidden));
            }
        }
    }

    #[tokio::test]
    async fn buffered_playlist_and_vtt_ranges_use_final_representation_for_get_and_head() {
        for (bytes, mime) in [
            (
                b"#EXTM3U\n#EXTINF:5,\n/playback/v1/media/scoped\n#EXT-X-ENDLIST\n".as_slice(),
                "application/vnd.apple.mpegurl",
            ),
            (
                b"WEBVTT\n\n00:00.000 --> 00:05.000\nCaption\n".as_slice(),
                "text/vtt",
            ),
        ] {
            let total = bytes.len();
            for (range, selected) in [
                (None, 0..total),
                (Some("bytes=0-"), 0..total),
                (Some("bytes=2-6"), 2..7),
                (Some("bytes=2-"), 2..total),
                (Some("bytes=-5"), total - 5..total),
                (Some("bytes=-18446744073709551615"), 0..total),
                (Some("bytes=3-18446744073709551615"), 3..total),
                (Some("bytes=0-999999"), 0..total),
            ] {
                for head in [false, true] {
                    let selected = if head { 0..total } else { selected.clone() };
                    let response = secured(buffered(bytes, head, mime, range, None));
                    assert_eq!(
                        response.status(),
                        if range.is_some() && !head {
                            StatusCode::PARTIAL_CONTENT
                        } else {
                            StatusCode::OK
                        }
                    );
                    let headers = response.headers();
                    assert_eq!(headers[header::CONTENT_TYPE], mime);
                    assert_eq!(headers[header::CONTENT_LENGTH], selected.len().to_string());
                    assert_eq!(headers[header::ACCEPT_RANGES], "bytes");
                    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
                    assert_eq!(headers["referrer-policy"], "no-referrer");
                    assert_eq!(headers["x-content-type-options"], "nosniff");
                    if range.is_some() && !head {
                        assert_eq!(
                            headers[header::CONTENT_RANGE],
                            format!("bytes {}-{}/{total}", selected.start, selected.end - 1)
                        );
                    } else {
                        assert!(!headers.contains_key(header::CONTENT_RANGE));
                    }
                    let body = axum::body::to_bytes(response.into_body(), 1024)
                        .await
                        .unwrap();
                    assert_eq!(
                        body.as_ref(),
                        if head { &[] } else { &bytes[selected.clone()] }
                    );
                }
            }
        }
    }

    #[tokio::test]
    async fn buffered_unsatisfiable_ranges_and_empty_representations_have_correct_lengths() {
        for bytes in [b"".as_slice(), b"abc".as_slice()] {
            for mime in ["application/vnd.apple.mpegurl", "text/vtt"] {
                for head in [false, true] {
                    let range = format!("bytes={}-", bytes.len());
                    let response = buffered(bytes, head, mime, Some(&range), None);
                    if head {
                        assert_eq!(response.status(), StatusCode::OK);
                        assert!(!response.headers().contains_key(header::CONTENT_RANGE));
                        assert_eq!(
                            response.headers()[header::CONTENT_LENGTH],
                            bytes.len().to_string()
                        );
                    } else {
                        assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
                        assert_eq!(
                            response.headers()[header::CONTENT_RANGE],
                            format!("bytes */{}", bytes.len())
                        );
                        assert_eq!(response.headers()[header::CONTENT_LENGTH], "0");
                    }
                    assert_eq!(response.headers()[header::ACCEPT_RANGES], "bytes");
                    assert!(
                        axum::body::to_bytes(response.into_body(), 1024)
                            .await
                            .unwrap()
                            .is_empty()
                    );
                }
            }
        }
        for head in [false, true] {
            let response = buffered(b"", head, "text/vtt", None, None);
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::CONTENT_LENGTH], "0");
            assert!(!response.headers().contains_key(header::CONTENT_RANGE));
            assert!(
                axum::body::to_bytes(response.into_body(), 1024)
                    .await
                    .unwrap()
                    .is_empty()
            );
        }
    }

    #[tokio::test]
    async fn buffered_if_range_without_local_validator_returns_full_representation() {
        for mime in ["application/vnd.apple.mpegurl", "text/vtt"] {
            for validator in [
                "\"unverified\"",
                "W/\"weak\"",
                "Wed, 21 Oct 2015 07:28:00 GMT",
            ] {
                for range in [
                    None,
                    Some("bytes=0-"),
                    Some("bytes=2-4"),
                    Some("bytes=999-"),
                ] {
                    for head in [false, true] {
                        let response =
                            buffered(b"representation", head, mime, range, Some(validator));
                        assert_eq!(response.status(), StatusCode::OK);
                        assert_eq!(response.headers()[header::CONTENT_LENGTH], "14");
                        assert!(!response.headers().contains_key(header::CONTENT_RANGE));
                        assert!(!response.headers().contains_key(header::ETAG));
                        assert!(!response.headers().contains_key(header::LAST_MODIFIED));
                        let body = axum::body::to_bytes(response.into_body(), 1024)
                            .await
                            .unwrap();
                        assert_eq!(
                            body.as_ref(),
                            if head {
                                b"".as_slice()
                            } else {
                                b"representation".as_slice()
                            }
                        );
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn buffered_ranges_preserve_malformed_multirange_and_duplicate_header_rejection() {
        for value in [
            "bytes=0-1,3-4",
            "bytes=-0",
            "bytes=-",
            "bytes=5-3",
            "items=0-1",
            "bytes= 0-1",
            "bytes=0-18446744073709551616",
        ] {
            assert!(provider::ByteRange::parse(value).is_err());
            for if_range in [None, Some("\"unverified\"")] {
                for head in [false, true] {
                    let response =
                        buffered(b"representation", head, "text/vtt", Some(value), if_range);
                    if head {
                        assert_eq!(response.status(), StatusCode::OK);
                        assert!(!response.headers().contains_key(header::CONTENT_RANGE));
                        assert_eq!(response.headers()[header::CONTENT_LENGTH], "14");
                    } else {
                        assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
                        assert_eq!(response.headers()[header::CONTENT_RANGE], "bytes */14");
                        assert_eq!(response.headers()[header::CONTENT_LENGTH], "0");
                    }
                    assert!(
                        axum::body::to_bytes(response.into_body(), 1024)
                            .await
                            .unwrap()
                            .is_empty()
                    );
                }
            }
        }
        for name in [header::RANGE, header::IF_RANGE] {
            let mut headers = HeaderMap::new();
            headers.append(name.clone(), HeaderValue::from_static("bytes=0-"));
            headers.append(name.clone(), HeaderValue::from_static("bytes=1-"));
            assert_eq!(single_header(&headers, name), Err(StatusCode::BAD_REQUEST));
        }
    }

    #[test]
    fn api_origin_and_query_are_not_url_injection_surfaces() {
        for origin in ["http://127.0.0.1:50054", "https://api.example.test"] {
            assert!(validate_origin(origin).is_ok());
        }
        for origin in [
            "http://user:pass@host",
            "https://host/path",
            "https://host?x=y",
            "https://host#fragment",
            "file:///private",
            "*",
        ] {
            assert!(validate_origin(origin).is_err(), "{origin}");
        }
        assert!(cap(Some(format!("cap={}", "a".repeat(64)))).is_ok());
        for query in [
            format!("cap={}&cap={}", "a".repeat(64), "b".repeat(64)),
            format!("url=http://provider&cap={}", "a".repeat(64)),
            format!("cap={}&x=y", "a".repeat(64)),
        ] {
            assert!(cap(Some(query)).is_err());
        }
    }

    struct FaultFence {
        revoked: std::sync::atomic::AtomicBool,
        calls: std::sync::atomic::AtomicUsize,
    }
    impl StreamFence for FaultFence {
        fn check(
            &self,
        ) -> std::pin::Pin<Box<dyn Future<Output = Result<(), StatusCode>> + Send + '_>> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let revoked = self.revoked.load(std::sync::atomic::Ordering::SeqCst);
            Box::pin(async move {
                if revoked {
                    Err(StatusCode::NOT_FOUND)
                } else {
                    Ok(())
                }
            })
        }
    }
    fn fault_fence() -> Arc<FaultFence> {
        Arc::new(FaultFence {
            revoked: Default::default(),
            calls: Default::default(),
        })
    }
    async fn revoke(fence: &FaultFence) {
        while fence.calls.load(std::sync::atomic::Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
        fence
            .revoked
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    struct ControlledTicks(tokio::sync::mpsc::UnboundedReceiver<()>);

    impl FenceTicks for ControlledTicks {
        async fn tick(&mut self) {
            if self.0.recv().await.is_none() {
                std::future::pending::<()>().await;
            }
        }
    }

    fn controlled_ticks() -> (tokio::sync::mpsc::UnboundedSender<()>, ControlledTicks) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (tx, ControlledTicks(rx))
    }

    async fn notification(notify: &tokio::sync::Notify) {
        tokio::time::timeout(Duration::from_secs(2), notify.notified())
            .await
            .expect("test notification");
    }

    struct OwnershipMutexFence {
        mutex: Arc<tokio::sync::Mutex<()>>,
        calls: std::sync::atomic::AtomicUsize,
        checking: tokio::sync::Notify,
        stopped: std::sync::atomic::AtomicBool,
    }

    impl StreamFence for OwnershipMutexFence {
        fn check(
            &self,
        ) -> std::pin::Pin<Box<dyn Future<Output = Result<(), StatusCode>> + Send + '_>> {
            Box::pin(async move {
                use std::sync::atomic::Ordering::SeqCst;
                if self.calls.fetch_add(1, SeqCst) > 0 {
                    self.checking.notify_one();
                }
                match tokio::time::timeout(Duration::from_millis(500), self.mutex.lock()).await {
                    Ok(_guard) => Ok(()),
                    Err(_) => {
                        self.stopped.store(true, SeqCst);
                        Err(StatusCode::SERVICE_UNAVAILABLE)
                    }
                }
            })
        }
    }

    async fn inline_check_baseline<T, E, F: StreamFence + ?Sized>(
        fence: Arc<F>,
        future: impl Future<Output = Result<T, E>>,
        mut ticks: impl FenceTicks,
    ) -> Result<T, StatusCode> {
        tokio::pin!(future);
        check_fence(fence.as_ref()).await?;
        loop {
            tokio::select! {
                biased;
                _=ticks.tick()=>check_fence(fence.as_ref()).await?,
                result=&mut future=>return result.map_err(|_|StatusCode::BAD_GATEWAY),
            }
        }
    }

    #[tokio::test]
    async fn periodic_check_keeps_polling_ownership_holder_that_inline_baseline_suspends() {
        use std::sync::atomic::Ordering::SeqCst;
        for baseline in [true, false] {
            let fence = Arc::new(OwnershipMutexFence {
                mutex: Default::default(),
                calls: Default::default(),
                checking: Default::default(),
                stopped: Default::default(),
            });
            let holding = Arc::new(tokio::sync::Notify::new());
            let query_ready = Arc::new(tokio::sync::Notify::new());
            let work = {
                let mutex = fence.mutex.clone();
                let holding = holding.clone();
                let query_ready = query_ready.clone();
                async move {
                    let _guard = mutex.lock().await;
                    holding.notify_one();
                    query_ready.notified().await;
                    Ok::<_, ()>(42)
                }
            };
            let (tick, ticks) = controlled_ticks();
            let task = {
                let fence = fence.clone();
                tokio::spawn(async move {
                    if baseline {
                        inline_check_baseline(fence, work, ticks).await
                    } else {
                        checked_local_with_ticks(fence, work, ticks).await
                    }
                })
            };
            notification(&holding).await;
            tick.send(()).unwrap();
            notification(&fence.checking).await;
            query_ready.notify_one();
            let result = tokio::time::timeout(Duration::from_secs(2), task)
                .await
                .unwrap()
                .unwrap();
            if baseline {
                assert_eq!(result, Err(StatusCode::SERVICE_UNAVAILABLE));
                assert!(fence.stopped.load(SeqCst));
            } else {
                assert_eq!(result, Ok(42));
                assert!(!fence.stopped.load(SeqCst));
            }
        }
    }

    struct GateFence {
        calls: std::sync::atomic::AtomicUsize,
        checking: tokio::sync::Notify,
        release: tokio::sync::Notify,
        revoked: std::sync::atomic::AtomicBool,
    }

    impl StreamFence for GateFence {
        fn check(
            &self,
        ) -> std::pin::Pin<Box<dyn Future<Output = Result<(), StatusCode>> + Send + '_>> {
            Box::pin(async move {
                use std::sync::atomic::Ordering::SeqCst;
                if self.calls.fetch_add(1, SeqCst) > 0 {
                    self.checking.notify_one();
                    self.release.notified().await;
                }
                if self.revoked.load(SeqCst) {
                    Err(StatusCode::NOT_FOUND)
                } else {
                    Ok(())
                }
            })
        }
    }

    fn gate_fence() -> Arc<GateFence> {
        Arc::new(GateFence {
            calls: Default::default(),
            checking: Default::default(),
            release: Default::default(),
            revoked: Default::default(),
        })
    }

    struct DropCount(Arc<std::sync::atomic::AtomicUsize>);

    impl Drop for DropCount {
        fn drop(&mut self) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn completed_work_stays_private_until_started_fence_succeeds() {
        use std::sync::atomic::Ordering::SeqCst;
        let fence = gate_fence();
        let work_started = Arc::new(tokio::sync::Notify::new());
        let work_ready = Arc::new(tokio::sync::Notify::new());
        let work_done = Arc::new(tokio::sync::Notify::new());
        let drops = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let work = {
            let started = work_started.clone();
            let ready = work_ready.clone();
            let done = work_done.clone();
            let drops = drops.clone();
            async move {
                started.notify_one();
                ready.notified().await;
                done.notify_one();
                Ok::<_, ()>(DropCount(drops))
            }
        };
        let (tick, ticks) = controlled_ticks();
        let task = tokio::spawn(checked_local_with_ticks(fence.clone(), work, ticks));
        notification(&work_started).await;
        tick.send(()).unwrap();
        notification(&fence.checking).await;
        work_ready.notify_one();
        notification(&work_done).await;
        assert!(!task.is_finished());
        assert_eq!(drops.load(SeqCst), 0);
        assert_eq!(fence.calls.load(SeqCst), 2);
        tick.send(()).unwrap();
        fence.release.notify_one();
        notification(&fence.checking).await;
        assert_eq!(fence.calls.load(SeqCst), 3);
        assert!(!task.is_finished());
        fence.release.notify_one();
        let result = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(drops.load(SeqCst), 0);
        drop(result);
        assert_eq!(drops.load(SeqCst), 1);
    }

    #[tokio::test]
    async fn failed_and_timed_out_fences_drop_work_and_private_results_without_later_polling() {
        use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
        for timeout in [false, true] {
            for ready in [false, true] {
                let fence = gate_fence();
                let started = Arc::new(tokio::sync::Notify::new());
                let release_work = Arc::new(tokio::sync::Notify::new());
                let done = Arc::new(tokio::sync::Notify::new());
                let work_drops = Arc::new(AtomicUsize::new(0));
                let result_drops = Arc::new(AtomicUsize::new(0));
                let polls = Arc::new(AtomicUsize::new(0));
                let work = {
                    let started = started.clone();
                    let release_work = release_work.clone();
                    let done = done.clone();
                    let guard = DropCount(work_drops.clone());
                    let result_drops = result_drops.clone();
                    let polls = polls.clone();
                    async move {
                        let _guard = guard;
                        started.notify_one();
                        let wait = release_work.notified();
                        tokio::pin!(wait);
                        std::future::poll_fn(|cx| {
                            polls.fetch_add(1, SeqCst);
                            wait.as_mut().poll(cx)
                        })
                        .await;
                        done.notify_one();
                        Ok::<_, ()>(DropCount(result_drops))
                    }
                };
                let (tick, ticks) = controlled_ticks();
                let task = tokio::spawn(checked_local_with_ticks(fence.clone(), work, ticks));
                notification(&started).await;
                tick.send(()).unwrap();
                notification(&fence.checking).await;
                if ready {
                    release_work.notify_one();
                    notification(&done).await;
                }
                assert!(!task.is_finished());
                assert_eq!(result_drops.load(SeqCst), 0);
                if !timeout {
                    fence.revoked.store(true, SeqCst);
                    fence.release.notify_one();
                }
                let result = tokio::time::timeout(Duration::from_secs(2), task)
                    .await
                    .unwrap()
                    .unwrap();
                let expected = if timeout {
                    StatusCode::SERVICE_UNAVAILABLE
                } else {
                    StatusCode::NOT_FOUND
                };
                assert!(matches!(result, Err(status) if status == expected));
                assert_eq!(work_drops.load(SeqCst), 1);
                assert_eq!(result_drops.load(SeqCst), usize::from(ready));
                let stopped_polls = polls.load(SeqCst);
                release_work.notify_one();
                tokio::task::yield_now().await;
                assert_eq!(polls.load(SeqCst), stopped_polls);
            }
        }
    }

    #[tokio::test]
    async fn initial_denial_precedes_work_and_queued_ticks_precede_ready_results() {
        use std::sync::atomic::Ordering::SeqCst;
        let fence = gate_fence();
        fence.revoked.store(true, SeqCst);
        let (_tick, ticks) = controlled_ticks();
        let result = checked_local_with_ticks(
            fence,
            std::future::poll_fn(|_| -> std::task::Poll<Result<(), ()>> {
                panic!("initial denial must precede work polling");
            }),
            ticks,
        )
        .await;
        assert_eq!(result, Err(StatusCode::NOT_FOUND));

        let fence = gate_fence();
        let (tick, ticks) = controlled_ticks();
        tick.send(()).unwrap();
        let task = tokio::spawn(checked_local_with_ticks(
            fence.clone(),
            std::future::ready(Ok::<_, ()>(42)),
            ticks,
        ));
        notification(&fence.checking).await;
        assert!(!task.is_finished());
        fence.revoked.store(true, SeqCst);
        fence.release.notify_one();
        assert_eq!(task.await.unwrap(), Err(StatusCode::NOT_FOUND));
    }

    #[tokio::test]
    async fn genuine_ownership_contention_still_times_out_and_cancellation_releases_holder() {
        use std::sync::atomic::Ordering::SeqCst;
        for cancel in [false, true] {
            let fence = Arc::new(OwnershipMutexFence {
                mutex: Default::default(),
                calls: Default::default(),
                checking: Default::default(),
                stopped: Default::default(),
            });
            let started = Arc::new(tokio::sync::Notify::new());
            let drops = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let work = {
                let fence = fence.clone();
                let started = started.clone();
                let guard = DropCount(drops.clone());
                async move {
                    let _guard = guard;
                    let _ownership = if cancel {
                        Some(fence.mutex.lock().await)
                    } else {
                        None
                    };
                    started.notify_one();
                    std::future::pending::<Result<(), ()>>().await
                }
            };
            let (tick, ticks) = controlled_ticks();
            let task = tokio::spawn(checked_local_with_ticks(fence.clone(), work, ticks));
            notification(&started).await;
            let external_holder = if cancel {
                None
            } else {
                Some(fence.mutex.lock().await)
            };
            tick.send(()).unwrap();
            notification(&fence.checking).await;
            if cancel {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
                assert!(!fence.stopped.load(SeqCst));
                assert!(fence.mutex.try_lock().is_ok());
            } else {
                assert_eq!(
                    tokio::time::timeout(Duration::from_secs(2), task)
                        .await
                        .unwrap()
                        .unwrap(),
                    Err(StatusCode::SERVICE_UNAVAILABLE)
                );
                assert!(fence.stopped.load(SeqCst));
            }
            drop(external_holder);
            assert_eq!(drops.load(SeqCst), 1);
        }
    }

    #[tokio::test]
    async fn upstream_stall_is_revoked_within_five_seconds() {
        let fence = fault_fence();
        let stream_fence = fence.clone();
        let task = tokio::spawn(async move {
            checked_local(stream_fence, std::future::pending::<Result<(), ()>>()).await
        });
        revoke(&fence).await;
        let result = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result, Err(StatusCode::NOT_FOUND));
    }

    #[tokio::test]
    async fn downstream_backpressure_is_revoked_within_five_seconds() {
        let fence = fault_fence();
        let stream_fence = fence.clone();
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        tx.send(vec![1u8; 256 * 1024]).await.unwrap();
        let task = tokio::spawn(async move {
            checked_local(stream_fence, tx.send(vec![2u8; 256 * 1024])).await
        });
        revoke(&fence).await;
        let result = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result, Err(StatusCode::NOT_FOUND));
        assert_eq!(rx.recv().await.unwrap(), vec![1u8; 256 * 1024]);
        assert!(rx.recv().await.is_none());
    }
}
