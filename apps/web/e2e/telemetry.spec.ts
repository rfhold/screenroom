import { expect, test } from '@playwright/test';
import { maintenanceServer, userId } from './maintenance-server';
import { consumerWire, msg, text } from './public-consumer-wire';

type Span = { name: string; traceId: string; spanId: string; parentSpanId?: string };
function spans(value: unknown): Span[] {
  if (value === null || typeof value !== 'object') return [];
  if ('traceId' in value && 'spanId' in value && 'name' in value) return [value as Span];
  return Object.values(value).flatMap(spans);
}

test('Faro emits the span carried by the actual WebAssembly Home request', async ({ page }) => {
  const server = await maintenanceServer();
  try {
    await page.clock.install();
    await consumerWire(page, server.origin, call => call.method === 'GetHome' ? msg(text(1, msg(text(1, 'catalog'), text(2, 'Catalog')))) : undefined);
    const payloads: string[] = [];
    const metricsUrl = 'https://screenroom-telemetry.invalid/v1/metrics';
    await page.route(metricsUrl, route => route.fulfill({
      status: 200, headers: {
        'access-control-allow-origin': '*', 'access-control-allow-methods': 'POST,OPTIONS',
        'access-control-allow-headers': 'content-type',
      },
    }));
    await page.route('https://faro.holdenitdown.net/collect', async route => {
      const request = route.request();
      if (request.method() === 'POST') payloads.push(request.postData() ?? '');
      await route.fulfill({
        status: 202, headers: {
          'access-control-allow-origin': '*', 'access-control-allow-methods': 'POST,OPTIONS',
          'access-control-allow-headers': 'content-type,content-encoding,x-faro-session-id',
        },
      });
    });
    await page.route(`${server.origin}/screenroom-config.js`, route => route.fulfill({
      contentType: 'text/javascript',
      body: `window.__SCREENROOM_USER_ID__ = "${userId}"; window.__SCREENROOM_TELEMETRY__ = {enabled:true,environment:"preview",version:"test-revision",sampleRate:1,metricsUrl:${JSON.stringify(metricsUrl)}};`,
    }));
    const request = page.waitForRequest('**/screenroom.catalog.v1.BrowseService/GetHome');
    await page.goto(server.origin);
    const header = (await request).headers().traceparent;
    expect(header).toMatch(/^00-[0-9a-f]{32}-[0-9a-f]{16}-01$/);
    const [, traceId, spanId] = header.split('-');
    await expect.poll(() => {
      const all = payloads.filter(Boolean).flatMap(payload => spans(JSON.parse(payload)));
      return all.some(span => span.traceId === traceId && span.spanId === spanId && span.name === 'screenroom.ui.home');
    }, { timeout: 12_000 }).toBe(true);
    expect(payloads.join('')).toContain('deployment.environment.name');
    expect(payloads.join('')).toContain('test-revision');
    const metricExport = page.waitForRequest(request => request.url() === metricsUrl && request.method() === 'POST', { timeout: 10_000 });
    await page.clock.fastForward(31_000);
    const metric = await metricExport;
    expect(metric.headers()['content-type']).toBe('application/x-protobuf');
    const body = metric.postDataBuffer()!;
    for (const field of ['screenroom.ui.operation.duration', 'deployment.environment.name', 'service.instance.id', 'test-revision']) {
      expect(body.includes(Buffer.from(field))).toBe(true);
    }
  } finally {
    await server.close();
  }
});
