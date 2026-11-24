use crate::metrics::MetricsConfig;
use opentelemetry::metrics::{Meter, MeterProvider as _};
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_otlp::WithExportConfig as _;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use opentelemetry_sdk::{Resource, propagation::TraceContextPropagator, trace::SdkTracerProvider};
use std::{error::Error, fmt};
use tracing_opentelemetry::OpenTelemetrySpanExt as _;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt as _, util::SubscriberInitExt as _};

pub use tonic_tracing_opentelemetry::middleware::server::OtelGrpcLayer;
mod rpc_status;
pub use rpc_status::RpcStatusLayer;

pub const DEFAULT_OTLP_ENDPOINT: &str = "https://telemetry.holdenitdown.net:4318";

#[cfg(target_os = "android")]
pub(crate) fn android_http_client() -> Result<reqwest::blocking::Client, TelemetryError> {
    // The default platform verifier needs JNI initialization. Use the same trust roots as native RPC.
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let tls = rustls::ClientConfig::builder_with_provider(
        rustls::crypto::ring::default_provider().into(),
    )
    .with_safe_default_protocol_versions()
    .map_err(|_| TelemetryError("could not configure OTLP TLS"))?
    .with_root_certificates(roots)
    .with_no_client_auth();
    reqwest::blocking::Client::builder()
        .tls_backend_preconfigured(tls)
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .map_err(|_| TelemetryError("could not configure OTLP HTTP client"))
}

pub(crate) fn resource(service_name: &str) -> Resource {
    use opentelemetry::{Key, KeyValue};
    let resource = Resource::builder().build();
    let defaults = [
        ("service.namespace", "screenroom".to_owned()),
        ("deployment.environment.name", "local".to_owned()),
        ("service.version", env!("CARGO_PKG_VERSION").to_owned()),
        (
            "service.instance.id",
            std::env::var("SCREENROOM_SERVICE_INSTANCE_ID")
                .ok()
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        ),
    ];
    let mut builder = Resource::builder_empty()
        .with_attributes(
            resource
                .iter()
                .map(|(k, v)| KeyValue::new(k.clone(), v.clone())),
        )
        .with_attributes(defaults.into_iter().filter_map(|(k, v)| {
            resource
                .get(&Key::new(k))
                .is_none()
                .then(|| KeyValue::new(k, v))
        }));
    if resource
        .get(&Key::new("service.name"))
        .is_none_or(|name| name.as_str().starts_with("unknown_service"))
    {
        builder = builder.with_service_name(service_name.to_owned());
    }
    builder.build()
}

pub fn inject_trace_context(mut insert: impl FnMut(&str, String)) {
    struct Carrier<F>(F);
    impl<F: FnMut(&str, String)> opentelemetry::propagation::Injector for Carrier<F> {
        fn set(&mut self, key: &str, value: String) {
            if matches!(key, "traceparent" | "tracestate") {
                (self.0)(key, value);
            }
        }
    }
    let cx = tracing::Span::current().context();
    opentelemetry::global::get_text_map_propagator(|p| {
        p.inject_context(&cx, &mut Carrier(&mut insert));
    });
}

pub async fn observe_http_headers<T>(
    method: &http::Method,
    route: &'static str,
    headers: &http::HeaderMap,
    future: impl Future<Output = http::Response<T>>,
) -> http::Response<T> {
    use opentelemetry::{KeyValue, global, propagation::Extractor};
    use tracing::Instrument as _;
    struct Carrier<'a>(&'a http::HeaderMap);
    impl Extractor for Carrier<'_> {
        fn get(&self, key: &str) -> Option<&str> {
            self.0.get(key)?.to_str().ok()
        }
        fn keys(&self) -> Vec<&str> {
            self.0.keys().map(http::HeaderName::as_str).collect()
        }
    }
    let span = tracing::info_span!(
        "http request",
        otel.name = format_args!("{method} {route}"),
        otel.kind = "server",
        otel.status_code = tracing::field::Empty,
        http.request.method = %method,
        http.route = route,
        http.response.status_code = tracing::field::Empty,
    );
    let parent = global::get_text_map_propagator(|p| p.extract(&Carrier(headers)));
    let _ = span.set_parent(parent);
    let start = std::time::Instant::now();
    let response = future.instrument(span.clone()).await;
    let status = response.status();
    span.record("http.response.status_code", status.as_u16());
    if status.is_server_error() {
        span.record("otel.status_code", "ERROR");
    }
    global::meter("screenroom-http")
        .f64_histogram("http.server.request.duration")
        .with_unit("s")
        .build()
        .record(
            start.elapsed().as_secs_f64(),
            &[
                KeyValue::new("http.request.method", method.to_string()),
                KeyValue::new("http.route", route),
                KeyValue::new("http.response.status_code", i64::from(status.as_u16())),
            ],
        );
    response
}

pub fn current_span_context() -> Option<opentelemetry::trace::SpanContext> {
    use opentelemetry::trace::TraceContextExt as _;
    let cx = tracing::Span::current().context();
    let span = cx.span();
    let context = span.span_context();
    context.is_valid().then(|| context.clone())
}

pub fn link_current_span(context: &opentelemetry::trace::SpanContext) {
    tracing::Span::current().add_link(context.clone());
}

#[derive(Debug)]
pub struct TelemetryError(pub(crate) &'static str);

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
        let traces_endpoint = if read("SCREENROOM_TELEMETRY_DISABLED")?.as_deref() == Some("1") {
            None
        } else if let Some(endpoint) = read("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT")? {
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
        self.build_with_resource(resource(service_name))
    }

    fn build_with_resource(&self, resource: Resource) -> Result<SdkTracerProvider, TelemetryError> {
        let mut builder = SdkTracerProvider::builder().with_resource(resource);
        if let Some(endpoint) = &self.traces_endpoint {
            if std::env::var_os("OTEL_TRACES_SAMPLER").is_none() {
                use opentelemetry_sdk::trace::Sampler;
                builder = builder.with_sampler(Sampler::ParentBased(Box::new(
                    Sampler::TraceIdRatioBased(0.1),
                )));
            }
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
                .with_timeout(std::time::Duration::from_secs(3));
            #[cfg(target_os = "android")]
            let exporter = {
                use opentelemetry_otlp::WithHttpConfig as _;
                exporter.with_http_client(android_http_client()?)
            };
            let exporter = exporter
                .build()
                .map_err(|_| TelemetryError("could not configure OTLP trace exporter"))?;
            builder = builder.with_batch_exporter(exporter);
        }
        Ok(builder.build())
    }
}

pub struct Telemetry {
    provider: SdkTracerProvider,
    meter_provider: SdkMeterProvider,
}

impl Telemetry {
    pub fn init(service_name: &str, config: TelemetryConfig) -> Result<Self, TelemetryError> {
        Self::init_with_metrics(service_name, config, MetricsConfig::from_env()?)
    }

    pub fn init_client(service_name: &str, environment: &str) -> Result<Self, TelemetryError> {
        let mut config = TelemetryConfig::from_env()?;
        let mut metrics = MetricsConfig::from_env()?;
        if std::env::var("SCREENROOM_TELEMETRY_DISABLED").as_deref() != Ok("1") {
            config
                .traces_endpoint
                .get_or_insert_with(|| format!("{DEFAULT_OTLP_ENDPOINT}/v1/traces"));
            metrics
                .metrics_endpoint
                .get_or_insert_with(|| format!("{DEFAULT_OTLP_ENDPOINT}/v1/metrics"));
        }
        let resource = Resource::builder_empty()
            .with_attributes(
                resource(service_name)
                    .iter()
                    .map(|(k, v)| opentelemetry::KeyValue::new(k.clone(), v.clone())),
            )
            .with_attribute(opentelemetry::KeyValue::new(
                "deployment.environment.name",
                environment.to_owned(),
            ))
            .build();
        Self::init_with_resource(config, metrics, resource)
    }

    pub fn init_with_metrics(
        service_name: &str,
        config: TelemetryConfig,
        metrics: MetricsConfig,
    ) -> Result<Self, TelemetryError> {
        Self::init_with_resource(config, metrics, resource(service_name))
    }

    fn init_with_resource(
        config: TelemetryConfig,
        metrics: MetricsConfig,
        resource: Resource,
    ) -> Result<Self, TelemetryError> {
        let filter = EnvFilter::try_new(&config.filter)
            .map_err(|_| TelemetryError("invalid RUST_LOG filter"))?;
        let meter_provider = metrics.build_with_resource(resource.clone())?;
        let provider = match config.build_with_resource(resource) {
            Ok(provider) => provider,
            Err(error) => {
                let _ = meter_provider.shutdown();
                return Err(error);
            }
        };
        let subscriber = tracing_subscriber::registry()
            .with(filter)
            .with(tracing_subscriber::filter::filter_fn(|metadata| {
                // This warning includes untrusted collector response text.
                if matches!(
                    metadata.target(),
                    "opentelemetry-otlp" | "opentelemetry_otlp"
                ) && matches!(
                    metadata.name(),
                    "HttpTraceClient.PartialSuccess" | "HttpMetricsClient.PartialSuccess"
                ) {
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
            let _ = meter_provider.shutdown();
            return Err(TelemetryError("tracing subscriber already initialized"));
        }
        opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());
        opentelemetry::global::set_meter_provider(meter_provider.clone());
        Ok(Self {
            provider,
            meter_provider,
        })
    }

    pub fn meter(&self) -> Meter {
        self.meter_provider.meter("screenroom-worker")
    }

    pub fn force_flush(&self) -> Result<(), TelemetryError> {
        let traces = self.provider.force_flush();
        let metrics = self.meter_provider.force_flush();
        traces.map_err(|_| TelemetryError("trace provider flush failed"))?;
        metrics.map_err(|_| TelemetryError("metric provider flush failed"))
    }

    pub fn shutdown(self) -> Result<(), TelemetryError> {
        let traces = self.provider.shutdown();
        let metrics = self.meter_provider.shutdown();
        traces.map_err(|_| TelemetryError("trace provider shutdown failed"))?;
        metrics.map_err(|_| TelemetryError("metric provider shutdown failed"))
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
        let telemetry =
            Telemetry::init_with_metrics("init-test", config(), MetricsConfig::default()).unwrap();
        assert_eq!(
            Telemetry::init_with_metrics("init-test", config(), MetricsConfig::default())
                .err()
                .unwrap()
                .to_string(),
            "tracing subscriber already initialized"
        );
        telemetry.force_flush().unwrap();
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
