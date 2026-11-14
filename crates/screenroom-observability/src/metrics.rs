//! Caller-observed queue and worker metrics with closed, bounded label sets.

use crate::telemetry::TelemetryError;
use opentelemetry::{
    KeyValue,
    metrics::{Counter, Gauge, Histogram, Meter},
};
use opentelemetry_otlp::WithExportConfig as _;
use opentelemetry_sdk::{Resource, metrics::SdkMeterProvider};
use std::time::Duration;

/// OTLP/HTTP protobuf export is disabled unless an endpoint is configured.
#[derive(Default)]
pub struct MetricsConfig {
    pub metrics_endpoint: Option<String>,
}

impl MetricsConfig {
    pub fn from_env() -> Result<Self, TelemetryError> {
        Self::from_lookup(|name| match std::env::var(name) {
            Ok(value) => Ok(Some(value)),
            Err(std::env::VarError::NotPresent) => Ok(None),
            Err(_) => Err(TelemetryError(
                "telemetry environment contains invalid Unicode",
            )),
        })
    }

    fn from_lookup(
        mut read: impl FnMut(&str) -> Result<Option<String>, TelemetryError>,
    ) -> Result<Self, TelemetryError> {
        let metrics_endpoint = if let Some(endpoint) = read("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT")?
        {
            Some(endpoint)
        } else {
            read("OTEL_EXPORTER_OTLP_ENDPOINT")?
                .map(|endpoint| format!("{}/v1/metrics", endpoint.trim_end_matches('/')))
        };
        Ok(Self { metrics_endpoint })
    }

    pub fn build_provider(&self, service_name: &str) -> Result<SdkMeterProvider, TelemetryError> {
        let mut builder = SdkMeterProvider::builder().with_resource(
            Resource::builder()
                .with_service_name(service_name.to_owned())
                .build(),
        );
        if let Some(endpoint) = &self.metrics_endpoint {
            let uri = endpoint
                .parse::<http::Uri>()
                .map_err(|_| TelemetryError("invalid OTLP metrics endpoint"))?;
            if endpoint.contains('#')
                || !matches!(uri.scheme_str(), Some("http" | "https"))
                || uri.authority().is_none()
                || uri.authority().is_some_and(|a| a.as_str().contains('@'))
            {
                return Err(TelemetryError("invalid OTLP metrics endpoint"));
            }
            let exporter = opentelemetry_otlp::MetricExporter::builder()
                .with_http()
                .with_protocol(opentelemetry_otlp::Protocol::HttpBinary)
                .with_endpoint(endpoint)
                .build()
                .map_err(|_| TelemetryError("could not configure OTLP metric exporter"))?;
            builder = builder.with_periodic_exporter(exporter);
        }
        Ok(builder.build())
    }
}

macro_rules! labels {
    ($name:ident { $($variant:ident => $label:literal),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum $name { $($variant),+ }
        impl $name {
            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $label),+ }
            }
        }
    };
}

labels!(JobKind {
    Ingest => "ingest", Embedding => "embedding", Artwork => "artwork",
    Maintenance => "maintenance", Other => "other",
});
labels!(Transition {
    Enqueued => "enqueued", Claimed => "claimed", Committed => "committed",
    Deferred => "deferred", Retried => "retried", Failed => "failed",
    Stale => "stale", Cancelled => "cancelled", LeaseExpired => "lease_expired",
    Reclaimed => "reclaimed", Redriven => "redriven",
});
labels!(Outcome {
    Succeeded => "succeeded",
    Committed => "committed", Deferred => "deferred", Retried => "retried",
    Failed => "failed", Stale => "stale", Cancelled => "cancelled",
});
labels!(ErrorClass {
    Provider => "provider", Storage => "storage", Database => "database",
    InvalidInput => "invalid_input", Timeout => "timeout", Other => "other",
});

fn kind_label(kind: JobKind) -> KeyValue {
    KeyValue::new("job.kind", kind.as_str())
}

#[derive(Clone)]
pub struct QueueMetrics {
    transitions: Counter<u64>,
    latency: Histogram<f64>,
    ready: Gauge<u64>,
    leased: Gauge<u64>,
    dead: Gauge<u64>,
}

impl QueueMetrics {
    pub fn new(meter: &Meter) -> Self {
        Self {
            transitions: meter
                .u64_counter("screenroom.queue.transitions")
                .with_unit("{job}")
                .build(),
            latency: meter
                .f64_histogram("screenroom.queue.latency")
                .with_unit("s")
                .build(),
            ready: meter
                .u64_gauge("screenroom.queue.ready")
                .with_unit("{job}")
                .build(),
            leased: meter
                .u64_gauge("screenroom.queue.leased")
                .with_unit("{job}")
                .build(),
            dead: meter
                .u64_gauge("screenroom.queue.dead")
                .with_unit("{job}")
                .build(),
        }
    }

    /// Record only after the corresponding database transaction commits.
    pub fn transition(&self, kind: JobKind, transition: Transition, count: u64) {
        self.transitions.add(
            count,
            &[
                kind_label(kind),
                KeyValue::new("transition", transition.as_str()),
            ],
        );
    }

    pub fn latency(&self, kind: JobKind, elapsed: Duration) {
        self.latency
            .record(elapsed.as_secs_f64(), &[kind_label(kind)]);
    }

    /// Supply a database snapshot; this crate does not query or infer queue depth.
    pub fn counts(&self, kind: JobKind, ready: u64, leased: u64, dead: u64) {
        let labels = [kind_label(kind)];
        self.ready.record(ready, &labels);
        self.leased.record(leased, &labels);
        self.dead.record(dead, &labels);
    }
}

#[derive(Clone)]
pub struct WorkerMetrics {
    processing: Histogram<f64>,
    errors: Counter<u64>,
    provider: Histogram<f64>,
    storage: Histogram<f64>,
    provider_bytes: Counter<u64>,
    storage_bytes: Counter<u64>,
}

impl WorkerMetrics {
    /// Attach per operation/job; do not also record the same transport via legacy byte/timing hooks.
    pub fn transport_observer(&self, kind: JobKind) -> crate::transport::TransportObservation {
        crate::transport::TransportObservation::new(std::sync::Arc::new(WorkerTransportObserver {
            metrics: self.clone(),
            kind,
        }))
    }
    pub fn new(meter: &Meter) -> Self {
        Self {
            processing: meter
                .f64_histogram("screenroom.worker.processing.duration")
                .with_unit("s")
                .build(),
            errors: meter
                .u64_counter("screenroom.worker.errors")
                .with_unit("{error}")
                .build(),
            provider: meter
                .f64_histogram("screenroom.worker.provider.duration")
                .with_unit("s")
                .build(),
            storage: meter
                .f64_histogram("screenroom.worker.storage.duration")
                .with_unit("s")
                .build(),
            provider_bytes: meter
                .u64_counter("screenroom.worker.provider.bytes")
                .with_unit("By")
                .build(),
            storage_bytes: meter
                .u64_counter("screenroom.worker.storage.bytes")
                .with_unit("By")
                .build(),
        }
    }

    pub fn processing(&self, kind: JobKind, outcome: Outcome, elapsed: Duration) {
        self.processing.record(
            elapsed.as_secs_f64(),
            &[kind_label(kind), KeyValue::new("outcome", outcome.as_str())],
        );
    }

    pub fn error(&self, kind: JobKind, class: ErrorClass) {
        self.errors.add(
            1,
            &[
                kind_label(kind),
                KeyValue::new("error.class", class.as_str()),
            ],
        );
    }

    pub fn provider_duration(&self, kind: JobKind, outcome: Outcome, elapsed: Duration) {
        self.provider.record(
            elapsed.as_secs_f64(),
            &[kind_label(kind), KeyValue::new("outcome", outcome.as_str())],
        );
    }

    pub fn storage_duration(&self, kind: JobKind, outcome: Outcome, elapsed: Duration) {
        self.storage.record(
            elapsed.as_secs_f64(),
            &[kind_label(kind), KeyValue::new("outcome", outcome.as_str())],
        );
    }

    pub fn provider_bytes(&self, kind: JobKind, bytes: u64) {
        self.provider_bytes.add(bytes, &[kind_label(kind)]);
    }

    pub fn storage_bytes(&self, kind: JobKind, bytes: u64) {
        self.storage_bytes.add(bytes, &[kind_label(kind)]);
    }
}

struct WorkerTransportObserver {
    metrics: WorkerMetrics,
    kind: JobKind,
}
impl crate::transport::TransportObserver for WorkerTransportObserver {
    fn body_chunk(
        &self,
        request: crate::transport::RequestKind,
        direction: crate::transport::BodyDirection,
        bytes: u64,
    ) {
        use crate::transport::RequestKind;
        let counter = match request {
            RequestKind::Provider | RequestKind::Embedding => &self.metrics.provider_bytes,
            _ => &self.metrics.storage_bytes,
        };
        counter.add(
            bytes,
            &[
                kind_label(self.kind),
                KeyValue::new("request.kind", request.as_str()),
                KeyValue::new("body.direction", direction.as_str()),
            ],
        );
    }
    fn finished(
        &self,
        request: crate::transport::RequestKind,
        outcome: crate::transport::RequestOutcome,
        elapsed: Duration,
    ) {
        use crate::transport::RequestKind;
        let histogram = match request {
            RequestKind::Provider | RequestKind::Embedding => &self.metrics.provider,
            _ => &self.metrics.storage,
        };
        histogram.record(
            elapsed.as_secs_f64(),
            &[
                kind_label(self.kind),
                KeyValue::new("request.kind", request.as_str()),
                KeyValue::new("outcome", outcome.as_str()),
            ],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opentelemetry::metrics::MeterProvider as _;
    use opentelemetry_sdk::metrics::{
        InMemoryMetricExporter,
        data::{AggregatedMetrics, MetricData},
    };

    #[test]
    fn environment_precedence_and_absent_exporter() {
        let config = MetricsConfig::from_lookup(|_| Ok(None)).unwrap();
        assert!(config.metrics_endpoint.is_none());
        let provider = config.build_provider("no-collector").unwrap();
        provider.force_flush().unwrap();
        provider.shutdown().unwrap();
        let config = MetricsConfig::from_lookup(|name| {
            Ok(match name {
                "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT" => Some("https://collector/custom".into()),
                _ => panic!("signal-specific endpoint must take precedence"),
            })
        })
        .unwrap();
        assert_eq!(
            config.metrics_endpoint.as_deref(),
            Some("https://collector/custom")
        );
        let config = MetricsConfig::from_lookup(|name| {
            Ok((name == "OTEL_EXPORTER_OTLP_ENDPOINT").then(|| "https://collector/base///".into()))
        })
        .unwrap();
        assert_eq!(
            config.metrics_endpoint.as_deref(),
            Some("https://collector/base/v1/metrics")
        );
        let error = MetricsConfig::from_lookup(|_| {
            Err(TelemetryError(
                "telemetry environment contains invalid Unicode",
            ))
        })
        .err()
        .unwrap();
        assert_eq!(
            error.to_string(),
            "telemetry environment contains invalid Unicode"
        );
    }

    #[test]
    fn invalid_endpoints_have_sanitized_errors() {
        for endpoint in [
            "",
            "secret",
            "ftp://localhost",
            "http://user:secret@localhost",
            "http://localhost/#secret",
            "http:///",
            "https://",
        ] {
            let error = MetricsConfig {
                metrics_endpoint: Some(endpoint.into()),
            }
            .build_provider("test")
            .unwrap_err();
            assert_eq!(error.to_string(), "invalid OTLP metrics endpoint");
        }
    }

    #[test]
    fn transport_adapter_preserves_direction_request_kind_and_terminal_outcome() {
        use crate::transport::{BodyDirection, RequestKind, RequestOutcome};
        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_periodic_exporter(exporter.clone())
            .build();
        let metrics = WorkerMetrics::new(&provider.meter("transport-test"));
        let observer = metrics.transport_observer(JobKind::Embedding);
        let mut request = observer.start(RequestKind::Embedding);
        request.body().chunk(BodyDirection::UploadBodyConsumed, 13);
        request.body().chunk(BodyDirection::DownloadConsumed, 7);
        request.finish(RequestOutcome::Failed);
        drop(request);
        drop(observer.start(RequestKind::StorageRead));
        provider.force_flush().unwrap();
        let batches = exporter.get_finished_metrics().unwrap();
        let mut directions = std::collections::BTreeSet::new();
        let mut outcomes = std::collections::BTreeSet::new();
        for metric in batches[0].scope_metrics().flat_map(|scope| scope.metrics()) {
            let mut attributes = Vec::new();
            match metric.data() {
                AggregatedMetrics::U64(MetricData::Sum(sum)) => {
                    assert_eq!(metric.name(), "screenroom.worker.provider.bytes");
                    for point in sum.data_points() {
                        let direction = point
                            .attributes()
                            .find(|kv| kv.key.as_str() == "body.direction")
                            .unwrap()
                            .value
                            .to_string();
                        assert_eq!(
                            point.value(),
                            if direction == "upload_body_consumed" {
                                13
                            } else {
                                7
                            }
                        );
                        directions.insert(direction);
                        attributes.extend(point.attributes());
                    }
                }
                AggregatedMetrics::F64(MetricData::Histogram(histogram)) => {
                    for point in histogram.data_points() {
                        assert_eq!(point.count(), 1);
                        outcomes.insert(
                            point
                                .attributes()
                                .find(|kv| kv.key.as_str() == "outcome")
                                .unwrap()
                                .value
                                .to_string(),
                        );
                        attributes.extend(point.attributes());
                    }
                }
                _ => panic!("unexpected metric"),
            }
            assert!(attributes.iter().all(|kv| matches!(
                kv.key.as_str(),
                "job.kind" | "request.kind" | "body.direction" | "outcome"
            )));
        }
        assert_eq!(
            directions,
            [
                "download_consumed".to_owned(),
                "upload_body_consumed".to_owned()
            ]
            .into()
        );
        assert_eq!(
            outcomes,
            ["cancelled".to_owned(), "failed".to_owned()].into()
        );
        provider.shutdown().unwrap();
    }

    #[test]
    fn records_real_observations_with_bounded_labels_and_service_resource() {
        let exporter = InMemoryMetricExporter::default();
        let provider = SdkMeterProvider::builder()
            .with_resource(Resource::builder().with_service_name("worker-test").build())
            .with_periodic_exporter(exporter.clone())
            .build();
        let meter = provider.meter("screenroom-worker");
        let queue = QueueMetrics::new(&meter);
        let worker = WorkerMetrics::new(&meter);
        let transitions = [
            Transition::Enqueued,
            Transition::Claimed,
            Transition::Committed,
            Transition::Deferred,
            Transition::Retried,
            Transition::Failed,
            Transition::Stale,
            Transition::Cancelled,
            Transition::LeaseExpired,
            Transition::Reclaimed,
            Transition::Redriven,
        ];
        for transition in transitions {
            queue.transition(JobKind::Ingest, transition, 2);
        }
        queue.latency(JobKind::Ingest, Duration::from_millis(250));
        queue.counts(JobKind::Ingest, 5, 2, 1);
        worker.processing(JobKind::Ingest, Outcome::Committed, Duration::from_secs(3));
        worker.error(JobKind::Ingest, ErrorClass::Timeout);
        worker.provider_duration(JobKind::Ingest, Outcome::Succeeded, Duration::from_secs(1));
        worker.storage_duration(JobKind::Ingest, Outcome::Failed, Duration::from_secs(2));
        worker.provider_bytes(JobKind::Ingest, 123);
        worker.storage_bytes(JobKind::Ingest, 456);
        provider.force_flush().unwrap();
        let batches = exporter.get_finished_metrics().unwrap();
        assert_eq!(batches.len(), 1);
        let batch = &batches[0];
        assert_eq!(
            batch
                .resource()
                .get(&opentelemetry::Key::new("service.name"))
                .unwrap()
                .to_string(),
            "worker-test"
        );
        let metrics: Vec<_> = batch
            .scope_metrics()
            .flat_map(|scope| scope.metrics())
            .collect();
        assert_eq!(metrics.len(), 11);
        for metric in metrics {
            let mut attributes = Vec::new();
            match metric.data() {
                AggregatedMetrics::U64(MetricData::Sum(sum)) => {
                    for point in sum.data_points() {
                        let expected = match metric.name() {
                            "screenroom.queue.transitions" => 2,
                            "screenroom.worker.errors" => 1,
                            "screenroom.worker.provider.bytes" => 123,
                            "screenroom.worker.storage.bytes" => 456,
                            name => panic!("unexpected counter {name}"),
                        };
                        assert_eq!(point.value(), expected);
                        attributes.extend(point.attributes());
                    }
                    if metric.name() == "screenroom.queue.transitions" {
                        assert_eq!(sum.data_points().count(), transitions.len());
                        let recorded: std::collections::BTreeSet<_> = sum
                            .data_points()
                            .flat_map(|p| p.attributes())
                            .filter(|kv| kv.key.as_str() == "transition")
                            .map(|kv| kv.value.to_string())
                            .collect();
                        assert_eq!(
                            recorded,
                            transitions.iter().map(|t| t.as_str().to_owned()).collect()
                        );
                    }
                }
                AggregatedMetrics::U64(MetricData::Gauge(gauge)) => {
                    for point in gauge.data_points() {
                        let expected = match metric.name() {
                            "screenroom.queue.ready" => 5,
                            "screenroom.queue.leased" => 2,
                            "screenroom.queue.dead" => 1,
                            name => panic!("unexpected gauge {name}"),
                        };
                        assert_eq!(point.value(), expected);
                        attributes.extend(point.attributes());
                    }
                }
                AggregatedMetrics::F64(MetricData::Histogram(histogram)) => {
                    assert_eq!(metric.unit(), "s");
                    for point in histogram.data_points() {
                        let expected = match metric.name() {
                            "screenroom.queue.latency" => 0.25,
                            "screenroom.worker.processing.duration" => 3.0,
                            "screenroom.worker.provider.duration" => 1.0,
                            "screenroom.worker.storage.duration" => 2.0,
                            name => panic!("unexpected histogram {name}"),
                        };
                        assert_eq!(point.count(), 1);
                        assert_eq!(point.sum(), expected);
                        attributes.extend(point.attributes());
                    }
                }
                data => panic!("unexpected aggregation {data:?}"),
            }
            assert!(!attributes.is_empty());
            assert!(attributes.iter().all(|kv| matches!(
                kv.key.as_str(),
                "job.kind" | "transition" | "outcome" | "error.class"
            )));
            assert!(
                attributes
                    .iter()
                    .any(|kv| kv.key.as_str() == "job.kind" && kv.value.to_string() == "ingest")
            );
        }
        provider.shutdown().unwrap();
    }
}
