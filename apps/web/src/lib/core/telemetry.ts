import { getWebInstrumentations, initializeFaro } from '@grafana/faro-web-sdk';
import { TracingInstrumentation } from '@grafana/faro-web-tracing';
import { context, SpanStatusCode, trace, type Context, type Histogram, type Span } from '@opentelemetry/api';
import { OTLPMetricExporter } from '@opentelemetry/exporter-metrics-otlp-proto';
import { resourceFromAttributes } from '@opentelemetry/resources';
import { MeterProvider, PeriodicExportingMetricReader } from '@opentelemetry/sdk-metrics';

const collector = 'https://faro.holdenitdown.net/collect';
const service = 'screenroom-web';
type Config = { enabled: boolean; environment: string; metricsUrl?: string; sampleRate?: number; version?: string };
type Operation = 'home' | 'catalog' | 'details' | 'playback.create';
let initialized = false;
let tracing = false;
let duration: Histogram | undefined;
let playbackStartup: Histogram | undefined;

export function initializeTelemetry() {
  const config = (window as Window & { __SCREENROOM_TELEMETRY__?: Config }).__SCREENROOM_TELEMETRY__;
  if (initialized || config?.enabled !== true) return;
  initialized = true;
  const environment = config.environment;
  const version = config.version ?? 'unknown';
  const attributes = {
    'service.name': service, 'service.namespace': 'screenroom', 'service.version': version,
    'deployment.environment.name': environment, 'service.instance.id': globalThis.crypto?.randomUUID?.() ??
      Array.from(globalThis.crypto.getRandomValues(new Uint8Array(16)), b => b.toString(16).padStart(2, '0')).join(''),
  };
  try {
    initializeFaro({
      url: collector,
      app: { name: service, version, environment },
      sessionTracking: { samplingRate: config.sampleRate ?? 0.1 },
      instrumentations: [
        ...getWebInstrumentations({ captureConsole: false, enablePerformanceInstrumentation: false,
          enableContentSecurityPolicyInstrumentation: false }),
        // The fetch patch can consume the WASM request body before honoring ignoreUrls.
        new TracingInstrumentation({ resourceAttributes: attributes, instrumentations: [] }),
      ],
      ignoreUrls: [
        /\/playback\/v1\/media(?:\/|[?#]|$)/,
        ...(config.metricsUrl ? [config.metricsUrl] : []),
      ],
      pageTracking: { page: { id: 'screenroom', url: '/' }, generatePageId: () => 'screenroom' },
    });
    tracing = true;
  } catch { /* Telemetry is optional at runtime. */ }
  if (!config.metricsUrl) return;
  try {
    const provider = new MeterProvider({
      resource: resourceFromAttributes(attributes),
      readers: [new PeriodicExportingMetricReader({
        exporter: new OTLPMetricExporter({ url: config.metricsUrl, timeoutMillis: 4000 }),
        exportIntervalMillis: 30_000, exportTimeoutMillis: 5000,
      })],
    });
    const meter = provider.getMeter(service);
    duration = meter.createHistogram('screenroom.ui.operation.duration', { unit: 's' });
    playbackStartup = meter.createHistogram('screenroom.playback.startup.duration', { unit: 's' });
  } catch { /* A bad exporter must not break the app. */ }
}

export function observeFirstFrame(video: HTMLVideoElement) {
  if ((!tracing && !playbackStartup) || typeof video.requestVideoFrameCallback !== 'function') {
    return { parent: undefined, cancel() {} };
  }
  const start = performance.now();
  const span = tracing ? trace.getTracer(service).startSpan('screenroom.playback.first_frame') : undefined;
  let pending = true;
  const frame = video.requestVideoFrameCallback(() => {
    pending = false;
    playbackStartup?.record((performance.now() - start) / 1000);
    span?.end();
  });
  return {
    parent: span ? trace.setSpan(context.active(), span) : undefined,
    cancel() {
      if (!pending) return;
      pending = false;
      video.cancelVideoFrameCallback(frame);
      span?.setAttribute('screenroom.outcome', 'cancelled');
      span?.end();
    },
  };
}

export function observeOperation<T>(operation: Operation, run: (traceParent?: string) => Promise<T>, parent = context.active()): Promise<T> {
  if (!tracing && !duration) return run();
  const observe = async (span?: Span) => {
    const start = performance.now();
    let outcome = 'ok';
    try {
      const cx = span?.spanContext();
      const traceParent = cx && trace.isSpanContextValid(cx) && (cx.traceFlags & 1)
        ? `00-${cx.traceId}-${cx.spanId}-01` : undefined;
      return await run(traceParent);
    } catch (error) {
      const code = typeof error === 'object' && error !== null && 'code' in error ? error.code : undefined;
      outcome = code === 'deadline_exceeded' ? 'deadline_exceeded' :
        code === 'cancelled' ? 'cancelled' : 'error';
      throw error;
    } finally {
      const fields = { 'screenroom.operation': operation, 'screenroom.outcome': outcome };
      duration?.record((performance.now() - start) / 1000, fields);
      span?.setAttributes(fields);
      if (outcome !== 'ok' && outcome !== 'cancelled') span?.setStatus({ code: SpanStatusCode.ERROR });
      span?.end();
    }
  };
  return tracing ? trace.getTracer(service).startActiveSpan(`screenroom.ui.${operation}`, {}, parent, observe) : observe();
}

export type { Context };
