use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig as _;
use opentelemetry_sdk::{Resource, propagation::TraceContextPropagator, trace::SdkTracerProvider};
use std::{error::Error, fmt};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt as _, util::SubscriberInitExt as _};

pub use tonic_tracing_opentelemetry::middleware::server::OtelGrpcLayer;

#[derive(Debug)]
pub struct TelemetryError(&'static str);

impl fmt::Display for TelemetryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl Error for TelemetryError {}

pub struct TelemetryConfig {
    pub filter: String,
    pub traces_endpoint: Option<String>,
}

impl TelemetryConfig {
    pub fn from_env() -> Result<Self, TelemetryError> {
        let read = |name| match std::env::var(name) {
            Ok(value) => Ok(Some(value)),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(_) => Err(TelemetryError(
                "telemetry environment contains invalid Unicode",
            )),
        };
        let traces_endpoint = if let Some(endpoint) = read("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT")? {
            Some(endpoint)
        } else {
            read("OTEL_EXPORTER_OTLP_ENDPOINT")?
                .map(|endpoint| format!("{}/v1/traces", endpoint.trim_end_matches('/')))
        };
        Ok(Self {
            filter: read("RUST_LOG")?.unwrap_or_else(|| "info".into()),
            traces_endpoint,
        })
    }

    pub fn build_provider(&self, service_name: &str) -> Result<SdkTracerProvider, TelemetryError> {
        let mut builder = SdkTracerProvider::builder().with_resource(
            Resource::builder()
                .with_service_name(service_name.to_owned())
                .build(),
        );
        if let Some(endpoint) = &self.traces_endpoint {
            if endpoint.contains('#') {
                return Err(TelemetryError("invalid OTLP traces endpoint"));
            }
            let uri = endpoint
                .parse::<http::Uri>()
                .map_err(|_| TelemetryError("invalid OTLP traces endpoint"))?;
            if !matches!(uri.scheme_str(), Some("http" | "https"))
                || uri.authority().is_none()
                || uri
                    .authority()
                    .is_some_and(|authority| authority.as_str().contains('@'))
            {
                return Err(TelemetryError("invalid OTLP traces endpoint"));
            }
            let exporter = opentelemetry_otlp::SpanExporter::builder()
                .with_http()
                .with_protocol(opentelemetry_otlp::Protocol::HttpBinary)
                .with_endpoint(endpoint)
                .build()
                .map_err(|_| TelemetryError("could not configure OTLP trace exporter"))?;
            builder = builder.with_batch_exporter(exporter);
        }
        Ok(builder.build())
    }
}

pub struct Telemetry {
    provider: SdkTracerProvider,
}

impl Telemetry {
    pub fn init(service_name: &str, config: TelemetryConfig) -> Result<Self, TelemetryError> {
        let filter = EnvFilter::try_new(&config.filter)
            .map_err(|_| TelemetryError("invalid RUST_LOG filter"))?;
        let provider = config.build_provider(service_name)?;
        let subscriber = tracing_subscriber::registry()
            .with(filter)
            .with(tracing_subscriber::filter::filter_fn(|metadata| {
                // This warning includes untrusted collector response text.
                if matches!(
                    metadata.target(),
                    "opentelemetry-otlp" | "opentelemetry_otlp"
                ) && metadata.name() == "HttpTraceClient.PartialSuccess"
                {
                    return false;
                }
                // Transport debug output can contain endpoints, headers, and response bodies.
                let sensitive = [
                    "opentelemetry-otlp",
                    "opentelemetry_otlp",
                    "opentelemetry-http",
                    "opentelemetry_http",
                    "reqwest",
                    "hyper",
                    "h2",
                ]
                .iter()
                .any(|prefix| metadata.target().starts_with(prefix));
                !sensitive || *metadata.level() <= tracing::Level::INFO
            }))
            .with(
                tracing_subscriber::fmt::layer()
                    .json()
                    .with_span_events(tracing_subscriber::fmt::format::FmtSpan::CLOSE),
            )
            .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("screenroom-rpc")));
        if subscriber.try_init().is_err() {
            let _ = provider.shutdown();
            return Err(TelemetryError("tracing subscriber already initialized"));
        }
        opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());
        Ok(Self { provider })
    }

    pub fn shutdown(self) -> Result<(), TelemetryError> {
        self.provider
            .shutdown()
            .map_err(|_| TelemetryError("trace provider shutdown failed"))
    }
}

pub fn rpc_layer() -> OtelGrpcLayer {
    OtelGrpcLayer::default()
}

pub async fn shutdown_signal() {
    #[cfg(unix)]
    {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut terminate) => tokio::select! {
                result = tokio::signal::ctrl_c() => {
                    if result.is_err() { tracing::error!("could not receive Ctrl-C"); }
                },
                _ = terminate.recv() => {},
            },
            Err(_) => {
                tracing::error!("could not install SIGTERM handler");
            }
        }
    }
    #[cfg(not(unix))]
    if tokio::signal::ctrl_c().await.is_err() {
        tracing::error!("could not receive Ctrl-C");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::instrument::WithSubscriber as _;

    #[test]
    fn provider_needs_no_collector_and_rejects_invalid_endpoint() {
        let config = TelemetryConfig {
            filter: "info".into(),
            traces_endpoint: None,
        };
        config.build_provider("test").unwrap().shutdown().unwrap();
        for endpoint in [
            "",
            "secret",
            "ftp://localhost",
            "http://user:secret@localhost",
        ] {
            let config = TelemetryConfig {
                filter: "info".into(),
                traces_endpoint: Some(endpoint.into()),
            };
            let error = config.build_provider("test").unwrap_err().to_string();
            assert_eq!(error, "invalid OTLP traces endpoint");
        }
    }

    #[test]
    fn repeated_initialization_is_an_error_not_a_panic() {
        let config = || TelemetryConfig {
            filter: "info".into(),
            traces_endpoint: None,
        };
        let telemetry = Telemetry::init("init-test", config()).unwrap();
        assert_eq!(
            Telemetry::init("init-test", config())
                .err()
                .unwrap()
                .to_string(),
            "tracing subscriber already initialized"
        );
        telemetry.shutdown().unwrap();
    }

    #[tokio::test]
    async fn rpc_parent_privacy_header_status_and_trailer_limit() {
        use http::{Request, Response};
        use http_body_util::BodyExt as _;
        use opentelemetry_sdk::trace::InMemorySpanExporter;
        use tower::{Layer as _, ServiceExt as _};

        #[derive(Clone, Debug)]
        struct RecordingExporter {
            inner: InMemorySpanExporter,
            resource: std::sync::Arc<std::sync::Mutex<Option<Resource>>>,
        }
        impl opentelemetry_sdk::trace::SpanExporter for RecordingExporter {
            async fn export(
                &self,
                batch: Vec<opentelemetry_sdk::trace::SpanData>,
            ) -> opentelemetry_sdk::error::OTelSdkResult {
                self.inner.export(batch).await
            }
            fn set_resource(&mut self, resource: &Resource) {
                *self.resource.lock().unwrap() = Some(resource.clone());
            }
        }

        opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());
        let exporter = InMemorySpanExporter::default();
        let resource = std::sync::Arc::new(std::sync::Mutex::new(None));
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(RecordingExporter {
                inner: exporter.clone(),
                resource: resource.clone(),
            })
            .with_resource(Resource::builder().with_service_name("rpc-test").build())
            .build();
        let subscriber = tracing_subscriber::registry()
            .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("rpc-test")));
        async {
            for header_status in [true, false] {
                let service = tower::service_fn(move |_req: Request<()>| async move {
                    let mut trailers = http::HeaderMap::new();
                    trailers.insert("grpc-status", "7".parse().unwrap());
                    let body = http_body_util::StreamBody::new(tokio_stream::iter([Ok::<
                        _,
                        std::convert::Infallible,
                    >(
                        http_body::Frame::<bytes::Bytes>::trailers(trailers),
                    )]));
                    let mut response = Response::new(body);
                    if header_status {
                        response
                            .headers_mut()
                            .insert("grpc-status", "12".parse().unwrap());
                    }
                    Ok::<_, std::convert::Infallible>(response)
                });
                let request = Request::builder()
                    .uri("http://localhost/screenroom.catalog.v1.BrowseService/GetHome")
                    .header(
                        "traceparent",
                        "00-b2611246a58fd7ea623d2264c5a1e226-b2c9b811f2f424af-01",
                    )
                    .header("authorization", "Bearer private-token")
                    .header("cookie", "private-cookie")
                    .body(())
                    .unwrap();
                let response = rpc_layer().layer(service).oneshot(request).await.unwrap();
                let body = response.into_body().collect().await.unwrap();
                assert_eq!(body.trailers().unwrap()["grpc-status"], "7");
            }
        }
        .with_subscriber(subscriber)
        .await;
        provider.force_flush().unwrap();
        let spans = exporter.get_finished_spans().unwrap();
        assert_eq!(spans.len(), 2);
        for (index, span) in spans.iter().enumerate() {
            assert_eq!(
                span.span_context.trace_id().to_string(),
                "b2611246a58fd7ea623d2264c5a1e226"
            );
            assert_eq!(span.parent_span_id.to_string(), "b2c9b811f2f424af");
            let attributes = format!("{:?}", span.attributes);
            assert!(attributes.contains("screenroom.catalog.v1.BrowseService"));
            assert!(attributes.contains("GetHome"));
            assert!(!attributes.contains("private-token"));
            assert!(!attributes.contains("private-cookie"));
            assert!(!attributes.to_lowercase().contains("authorization"));
            assert!(!attributes.to_lowercase().contains("cookie"));
            let status = span
                .attributes
                .iter()
                .find(|kv| kv.key.as_str() == "rpc.grpc.status_code")
                .unwrap();
            assert_eq!(
                status.value.to_string(),
                if index == 0 { "12" } else { "0" }
            );
        }
        assert_eq!(
            resource
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .get(&opentelemetry::Key::new("service.name"))
                .unwrap()
                .to_string(),
            "rpc-test"
        );
        provider.shutdown().unwrap();
    }
}
