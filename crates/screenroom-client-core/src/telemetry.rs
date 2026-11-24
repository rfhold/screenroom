use opentelemetry::{KeyValue, metrics::Histogram};
use screenroom_observability::telemetry::Telemetry;
use std::{sync::OnceLock, time::Instant};
use tracing::Instrument as _;

struct ClientTelemetry {
    _telemetry: Telemetry,
    duration: Histogram<f64>,
    playback_startup: Histogram<f64>,
}

static ACTIVE: OnceLock<Option<ClientTelemetry>> = OnceLock::new();

#[uniffi::export]
pub fn initialize_client_telemetry(platform: String, environment: String) -> bool {
    let service = match platform.as_str() {
        "ios" => "screenroom-ios",
        "macos" => "screenroom-macos",
        "tvos" => "screenroom-tvos",
        "android" => "screenroom-android",
        "android-tv" => "screenroom-android-tv",
        "linux" => "screenroom-linux",
        _ => return false,
    };
    if !matches!(environment.as_str(), "local" | "preview" | "production")
        || std::env::var("SCREENROOM_TELEMETRY_DISABLED").as_deref() == Ok("1")
    {
        return false;
    }
    ACTIVE
        .get_or_init(|| {
            let telemetry = Telemetry::init_client(service, &environment).ok()?;
            let meter = opentelemetry::global::meter("screenroom-client-core");
            let duration = meter
                .f64_histogram("screenroom.client.operation.duration")
                .with_unit("s")
                .build();
            Some(ClientTelemetry {
                _telemetry: telemetry,
                duration,
                playback_startup: meter
                    .f64_histogram("screenroom.playback.startup.duration")
                    .with_unit("s")
                    .build(),
            })
        })
        .is_some()
}

#[uniffi::export]
pub fn record_playback_first_frame(elapsed_ms: u64) {
    if let Some(Some(telemetry)) = ACTIVE.get() {
        telemetry
            .playback_startup
            .record(elapsed_ms as f64 / 1000.0, &[]);
    }
}

pub(crate) async fn observe<T, E>(
    operation: &str,
    future: impl Future<Output = Result<T, E>>,
    classify: impl FnOnce(&E) -> &'static str,
) -> Result<T, E> {
    let Some(Some(telemetry)) = ACTIVE.get() else {
        return future.await;
    };
    let span = tracing::info_span!(
        "client_operation",
        otel.name = operation,
        screenroom.operation = operation,
        screenroom.outcome = tracing::field::Empty,
    );
    let start = Instant::now();
    let result = future.instrument(span.clone()).await;
    let outcome = result.as_ref().err().map_or("ok", classify);
    span.record("screenroom.outcome", outcome);
    telemetry.duration.record(
        start.elapsed().as_secs_f64(),
        &[
            KeyValue::new("screenroom.operation", operation.to_owned()),
            KeyValue::new("screenroom.outcome", outcome),
        ],
    );
    result
}

pub(crate) fn inject<T>(request: &mut tonic::Request<T>) {
    if !matches!(ACTIVE.get(), Some(Some(_))) {
        return;
    }
    screenroom_observability::telemetry::inject_trace_context(|key, value| {
        if let (Ok(key), Ok(value)) = (
            key.parse::<tonic::metadata::MetadataKey<_>>(),
            value.parse(),
        ) {
            request.metadata_mut().insert(key, value);
        }
    });
}
