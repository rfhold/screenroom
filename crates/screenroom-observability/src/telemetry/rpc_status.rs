use http::{Request, Response};
use http_body::{Body, Frame, SizeHint};
use opentelemetry::{KeyValue, global, metrics::Histogram};
use std::{
    collections::HashSet,
    future::Future,
    pin::Pin,
    sync::OnceLock,
    task::{Context, Poll},
    time::Instant,
};
use tower::{Layer, Service};

const PROTOS: &[&str] = &[
    include_str!("../../../../proto/screenroom/catalog/v1/catalog.proto"),
    include_str!("../../../../proto/screenroom/user/v1/user.proto"),
    include_str!("../../../../proto/screenroom/user/internal/v1/internal.proto"),
    include_str!("../../../../proto/screenroom/operator/v1/operator.proto"),
    include_str!("../../../../proto/screenroom/playback_gateway/v1/playback_gateway.proto"),
];

fn methods() -> &'static HashSet<String> {
    static METHODS: OnceLock<HashSet<String>> = OnceLock::new();
    METHODS.get_or_init(|| {
        let mut methods = HashSet::new();
        for proto in PROTOS {
            let (mut package, mut service) = ("", "");
            for line in proto.lines().map(str::trim) {
                if let Some(value) = line.strip_prefix("package ") {
                    package = value.trim_end_matches(';');
                } else if let Some(value) = line.strip_prefix("service ") {
                    service = value.split_whitespace().next().unwrap_or_default();
                } else if let Some((method, _)) =
                    line.strip_prefix("rpc ").and_then(|s| s.split_once('('))
                {
                    methods.insert(format!("/{package}.{service}/{}", method.trim()));
                }
            }
        }
        methods
    })
}

fn status(headers: &http::HeaderMap) -> Option<i64> {
    headers
        .get("grpc-status")?
        .to_str()
        .ok()?
        .parse::<i64>()
        .ok()
        .filter(|code| (0..=16).contains(code))
}

struct Observation {
    start: Instant,
    method: &'static str,
    meter: Histogram<f64>,
    finished: bool,
}

impl Observation {
    fn finish(mut self, code: i64) {
        self.finished = true;
        self.record(code);
    }

    fn record(&self, code: i64) {
        const CODES: [&str; 17] = [
            "OK",
            "CANCELLED",
            "UNKNOWN",
            "INVALID_ARGUMENT",
            "DEADLINE_EXCEEDED",
            "NOT_FOUND",
            "ALREADY_EXISTS",
            "PERMISSION_DENIED",
            "RESOURCE_EXHAUSTED",
            "FAILED_PRECONDITION",
            "ABORTED",
            "OUT_OF_RANGE",
            "UNIMPLEMENTED",
            "INTERNAL",
            "UNAVAILABLE",
            "DATA_LOSS",
            "UNAUTHENTICATED",
        ];
        let code = CODES[code as usize];
        self.meter.record(
            self.start.elapsed().as_secs_f64(),
            &[
                KeyValue::new("rpc.system.name", "grpc"),
                KeyValue::new("rpc.method", self.method),
                KeyValue::new("rpc.response.status_code", code),
            ],
        );
    }
}

impl Drop for Observation {
    fn drop(&mut self) {
        if !self.finished {
            self.record(1);
        }
    }
}

/// Install inside gRPC-Web conversion, where status is still in native trailers.
#[derive(Clone, Copy)]
pub struct RpcStatusLayer;

impl<S> Layer<S> for RpcStatusLayer {
    type Service = RpcStatus<S>;
    fn layer(&self, inner: S) -> Self::Service {
        let meter = global::meter("screenroom-rpc")
            .f64_histogram("rpc.server.call.duration")
            .with_unit("s")
            .build();
        RpcStatus { inner, meter }
    }
}

#[derive(Clone)]
pub struct RpcStatus<S> {
    inner: S,
    meter: Histogram<f64>,
}

impl<S, RequestBody, ResponseBody> Service<Request<RequestBody>> for RpcStatus<S>
where
    S: Service<Request<RequestBody>, Response = Response<ResponseBody>>,
    S::Future: Send + 'static,
    ResponseBody: Body + Send + 'static,
{
    type Response = Response<Observed<ResponseBody>>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<RequestBody>) -> Self::Future {
        let mut entry = methods()
            .get(request.uri().path())
            .map(|method| Observation {
                start: Instant::now(),
                method: method.trim_start_matches('/'),
                meter: self.meter.clone(),
                finished: false,
            });
        let response = self.inner.call(request);
        Box::pin(async move {
            let response = match response.await {
                Ok(response) => response,
                Err(error) => {
                    if let Some(entry) = entry.take() {
                        entry.finish(2);
                    }
                    return Err(error);
                }
            };
            if let Some(code) = status(response.headers())
                && let Some(entry) = entry.take()
            {
                entry.finish(code);
            }
            Ok(response.map(|body| Observed {
                body: Box::pin(body),
                entry,
            }))
        })
    }
}

pub struct Observed<B> {
    body: Pin<Box<B>>,
    entry: Option<Observation>,
}

impl<B: Body> Body for Observed<B> {
    type Data = B::Data;
    type Error = B::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let frame = self.body.as_mut().poll_frame(cx);
        let code = match &frame {
            Poll::Ready(Some(Ok(frame))) => frame
                .trailers_ref()
                .map(|headers| status(headers).unwrap_or(2)),
            Poll::Ready(None | Some(Err(_))) => Some(2),
            Poll::Pending => None,
        };
        if let Some(code) = code
            && let Some(entry) = self.entry.take()
        {
            entry.finish(code);
        }
        frame
    }

    fn size_hint(&self) -> SizeHint {
        self.body.size_hint()
    }

    fn is_end_stream(&self) -> bool {
        self.entry.is_none() && self.body.is_end_stream()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_wire_methods_and_terminal_codes() {
        assert!(methods().contains("/screenroom.catalog.v1.BrowseService/GetHome"));
        assert!(
            methods()
                .contains("/screenroom.user.internal.v1.PlaybackStateService/GetContinueWatching")
        );
        assert!(!methods().contains("/screenroom.catalog.v1.BrowseService/x"));
        let mut trailers = http::HeaderMap::new();
        trailers.insert("grpc-status", http::HeaderValue::from_static("4"));
        assert_eq!(status(&trailers), Some(4));
    }

    #[tokio::test]
    async fn reports_terminal_trailers_even_when_http_is_successful() {
        use http_body_util::{BodyExt as _, StreamBody};
        use opentelemetry::metrics::MeterProvider as _;
        use opentelemetry_sdk::metrics::{
            InMemoryMetricExporter, SdkMeterProvider,
            data::{AggregatedMetrics, MetricData},
        };
        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_periodic_exporter(exporter.clone())
            .build();
        let mut headers = http::HeaderMap::new();
        headers.insert("grpc-status", http::HeaderValue::from_static("4"));
        let (_tx, rx) = tokio::sync::mpsc::channel(1);
        _tx.send(Ok::<_, std::convert::Infallible>(
            Frame::<bytes::Bytes>::trailers(headers),
        ))
        .await
        .unwrap();
        drop(_tx);
        let body = Observed {
            body: Box::pin(StreamBody::new(
                tokio_stream::wrappers::ReceiverStream::new(rx),
            )),
            entry: Some(Observation {
                finished: false,
                start: Instant::now(),
                method: "screenroom.catalog.v1.BrowseService/GetHome",
                meter: provider
                    .meter("test")
                    .f64_histogram("rpc.server.call.duration")
                    .with_unit("s")
                    .build(),
            }),
        };
        let response = Response::new(body);
        assert_eq!(response.status(), http::StatusCode::OK);
        response.into_body().collect().await.unwrap();
        provider.force_flush().unwrap();
        let batches = exporter.get_finished_metrics().unwrap();
        let metric = batches[0]
            .scope_metrics()
            .flat_map(|scope| scope.metrics())
            .next()
            .unwrap();
        assert_eq!(metric.name(), "rpc.server.call.duration");
        assert_eq!(metric.unit(), "s");
        let AggregatedMetrics::F64(MetricData::Histogram(histogram)) = metric.data() else {
            panic!("expected a duration histogram");
        };
        let point = histogram.data_points().next().unwrap();
        assert_eq!(point.count(), 1);
        assert!(
            point
                .attributes()
                .any(|kv| kv.key.as_str() == "rpc.response.status_code"
                    && kv.value.as_str() == "DEADLINE_EXCEEDED")
        );
    }
}
