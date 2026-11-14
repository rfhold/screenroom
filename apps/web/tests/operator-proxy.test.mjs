import assert from 'node:assert/strict';
import http from 'node:http';
import { once } from 'node:events';
import { test } from 'node:test';
import {
  createOperatorMiddleware, isLoopback, operatorBrowserConfig, operatorProxyPlugin, readOperatorConfig
} from '../server/operator-proxy.mjs';

const id = '00000000-0000-0000-0000-000000000001';
const base = `/__operator/api/sources/${id}`;
const indexing = `/__operator/api/indexing/sources/${id}`;
const indexingRun = `${indexing}/runs/${id}`;
const indexingJob = `${indexingRun}/jobs/${id}`;

async function listen(server) {
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  return `http://127.0.0.1:${server.address().port}`;
}

async function fixture(t, options = {}) {
  const calls = [];
  const upstream = http.createServer(async (req, res) => {
    const chunks = [];
    for await (const chunk of req) chunks.push(chunk);
    calls.push({ method: req.method, url: req.url, headers: req.headers, body: Buffer.concat(chunks).toString() });
    if (options.respond) return options.respond(req, res);
    res.writeHead(200, {
      'content-type': 'application/json', 'set-cookie': 'must-not-escape=yes',
      'access-control-allow-origin': '*', 'x-debug': 'must-not-escape'
    });
    res.end('{"ok":true}');
  });
  const target = await listen(upstream);
  let middleware;
  const web = http.createServer((req, res) => {
    if (options.peer) Object.defineProperty(req.socket, 'remoteAddress', { value: options.peer });
    middleware(req, res, () => { res.writeHead(200); res.end('asset'); });
  });
  const origin = await listen(web);
  const config = readOperatorConfig({ SCREENROOM_WEB_OPERATOR_TARGET: target, SCREENROOM_WEB_OPERATOR_ORIGIN: origin });
  middleware = createOperatorMiddleware(options.disabled ? null : config, options);
  t.after(async () => {
    upstream.closeAllConnections();
    web.closeAllConnections();
    await Promise.all([new Promise(resolve => upstream.close(resolve)), new Promise(resolve => web.close(resolve))]);
  });
  const request = (path = '/__operator/api/health', { method = 'GET', headers = {}, body, rawHeaders } = {}) => new Promise((resolve, reject) => {
    const req = http.request(origin + '/', {
      path, method,
      headers: rawHeaders ?? { host: new URL(origin).host, ...headers },
      setHost: false, agent: false
    }, res => {
      const chunks = [];
      res.on('data', chunk => chunks.push(chunk));
      res.on('end', () => resolve({ status: res.statusCode, headers: res.headers, body: Buffer.concat(chunks).toString() }));
      res.on('error', reject);
    });
    req.on('error', reject);
    req.end(body);
  });
  return { target, origin, config, request, calls };
}

test('configuration is explicit, canonical and loopback-only; browser values contain no target', () => {
  assert.equal(readOperatorConfig({}), null);
  assert.deepEqual(operatorBrowserConfig(null), { enabled: false, prefix: '/__operator/api', attachmentIdentity: null });
  const env = { SCREENROOM_WEB_OPERATOR_TARGET: 'http://127.0.0.1:50073', SCREENROOM_WEB_OPERATOR_ORIGIN: 'http://127.0.0.1:4173' };
  const config = readOperatorConfig(env);
  const browser = operatorBrowserConfig(config);
  assert.equal(browser.enabled, true);
  assert.match(browser.attachmentIdentity, /^web-operator:[a-f0-9]{64}$/);
  assert.deepEqual(browser, operatorBrowserConfig(readOperatorConfig(env)));
  assert.notEqual(browser.attachmentIdentity, operatorBrowserConfig({ ...config, target: 'http://127.0.0.1:50074' }).attachmentIdentity);
  assert.ok(!JSON.stringify(browser).includes('50073'));
  for (const value of ['', 'https://127.0.0.1:50073', 'http://localhost:50073', 'http://127.0.0.1:50073/',
    'http://user:pass@127.0.0.1:50073', 'http://127.0.0.1:50073/path', 'http://127.0.0.1:50073?x=1',
    'http://127.0.0.1:50073#x', 'http://192.168.1.1:50073', 'http://2130706433:50073', 'http://127.0.0.1:080', 'http://127.0.0.1:0']) {
    assert.throws(() => readOperatorConfig({ ...env, SCREENROOM_WEB_OPERATOR_TARGET: value }));
    assert.throws(() => readOperatorConfig({ ...env, SCREENROOM_WEB_OPERATOR_ORIGIN: value }));
  }
  assert.throws(() => readOperatorConfig({ SCREENROOM_WEB_OPERATOR_TARGET: env.SCREENROOM_WEB_OPERATOR_TARGET }));
  assert.throws(() => readOperatorConfig({ SCREENROOM_WEB_OPERATOR_ORIGIN: env.SCREENROOM_WEB_OPERATOR_ORIGIN }));
  assert.ok(readOperatorConfig({ ...env, SCREENROOM_WEB_OPERATOR_TARGET: 'http://[::1]:50073' }));
  for (const peer of ['127.0.0.1', '127.1.2.3', '::1', '::ffff:127.0.0.1']) assert.ok(isLoopback(peer));
  for (const peer of ['127.0.0.1.evil', '192.168.1.1', '::ffff:192.168.1.1', undefined]) assert.equal(isLoopback(peer), false);
});

test('both Vite dev and preview install middleware, including disabled handling', () => {
  for (const config of [null, { target: 'http://127.0.0.1:50073', origin: 'http://127.0.0.1:4173' }]) {
    const plugin = operatorProxyPlugin(config);
    const registered = [];
    const connectCanary = () => { throw new Error('Connect middleware must not become a Vite post-hook'); };
    const server = { middlewares: { use: middleware => { registered.push(middleware); return connectCanary; } } };
    assert.equal(plugin.configureServer(server), undefined);
    assert.equal(plugin.configurePreviewServer(server), undefined);
    assert.equal(registered.length, 2);
    assert.ok(registered.every(value => typeof value === 'function'));
  }
});

test('disabled proxy grants no upstream traffic and ordinary assets pass through', async t => {
  const f = await fixture(t, { disabled: true });
  assert.equal((await f.request()).status, 404);
  assert.equal((await f.request(`${base}/start`, { method: 'POST', body: '{}' })).status, 404);
  assert.equal((await f.request('/assets/app.js')).body, 'asset');
  assert.equal(f.calls.length, 0);
});

test('valid GET and POST forward only trusted routing, headers, and exact body', async t => {
  const f = await fixture(t);
  const hostile = {
    origin: f.origin, 'sec-fetch-site': 'same-origin', authorization: 'Bearer secret', cookie: 'session=secret',
    forwarded: 'host=evil', 'x-forwarded-host': 'evil', 'x-forwarded-for': '8.8.8.8',
    'proxy-authorization': 'secret', referer: 'http://evil/', 'x-target': 'http://evil/'
  };
  const get = await f.request('/__operator/api/sources?limit=5', { headers: hostile });
  assert.equal(get.status, 200);
  assert.deepEqual(JSON.parse(get.body), { ok: true });
  for (const key of ['set-cookie', 'access-control-allow-origin', 'x-debug']) assert.equal(get.headers[key], undefined);
  assert.equal(get.headers['cache-control'], 'no-store');
  assert.equal(get.headers['x-content-type-options'], 'nosniff');
  const body = '{"request_id":"00000000-0000-0000-0000-000000000001"}';
  assert.equal((await f.request(`${indexing}/start`, { method: 'POST', headers: { ...hostile, 'content-type': 'application/json' }, body })).status, 200);
  assert.equal(f.calls[0].url, '/api/sources?limit=5');
  assert.equal(f.calls[1].url, `/api/indexing/sources/${id}/start`);
  assert.equal(f.calls[1].body, body);
  for (const call of f.calls) {
    assert.equal(call.headers.host, new URL(f.target).host);
    assert.equal(call.headers.origin, f.target);
    assert.equal(call.headers['sec-fetch-site'], 'none');
    const allowed = ['host', 'origin', 'sec-fetch-site', 'connection'];
    if (call.method === 'POST') allowed.push('content-type', 'content-length');
    assert.deepEqual(Object.keys(call.headers).sort(), allowed.sort());
  }
});

test('successful JSON preserves exact bytes and nested wide integers without leaking upstream headers', async t => {
  const payload = '{\n  "version":9007199254740993,"count":9223372036854775807,\n  "nested":{"minimum":-9223372036854775808,"negative":-9007199254740993,"counts":[9007199254740993,9223372036854775807]},\n  "label":"日本語","escaped":"\\u0061"\n}\n';
  const bytes = Buffer.from(payload, 'utf8');
  const f = await fixture(t, { respond(req, res) {
    res.writeHead(200, {
      'content-type': 'application/json', 'set-cookie': 'must-not-escape=yes',
      'access-control-allow-origin': '*', 'x-debug': 'must-not-escape'
    });
    res.end(bytes);
  } });
  const response = await f.request();
  assert.equal(response.status, 200);
  assert.equal(response.body, payload);
  assert.deepEqual(Buffer.from(response.body, 'utf8'), bytes);
  for (const key of ['set-cookie', 'access-control-allow-origin', 'x-debug']) assert.equal(response.headers[key], undefined);
  assert.equal(response.headers['content-type'], 'application/json');
  assert.equal(response.headers['cache-control'], 'no-store');
  assert.equal(response.headers['x-content-type-options'], 'nosniff');
});

test('all typed API routes and bounded path-appropriate queries work', async t => {
  const f = await fixture(t);
  const paths = ['/__operator/api/health', '/__operator/api/sources', `${base}/libraries?after=library:one&limit=1000`,
    `${base}/libraries?after=library%3Aone&limit=1`,
    `${indexing}/runs?limit=1000`, indexingRun,
    `${indexingRun}/candidates?pending_only=true&after=${id}&limit=1000`, '/__operator/api/connections/jellyfin?limit=200'];
  for (const path of paths) assert.equal((await f.request(path)).status, 200, path);
  for (const path of ['/__operator/api/connections/jellyfin', '/__operator/api/connections/jellyfin/discover']) {
    assert.equal((await f.request(path, { method: 'POST', headers: { origin: f.origin, 'content-type': 'application/json' }, body: '{}' })).status, 200, path);
  }
});

test('canonical indexing routes admit only bounded route-specific cursors and methods', async t => {
  const f = await fixture(t);
  const gets = [`${indexing}/policies?after=${id}&limit=200`, `${indexing}/policies/${id}/progress`,
    indexingRun, `${indexingRun}/execution`, `${indexingRun}/jobs?after=${id}&limit=200`,
    `${indexingJob}/attempts?after=9007199254740993&limit=200`, `${indexingJob}/attempts?after=9223372036854775807`,
    `${indexingJob}/redrives?after=2147483647&limit=1`, `${indexingJob}/redrives?after=0`];
  for (const path of gets) assert.equal((await f.request(path)).status, 200, path);
  const posts = [`${indexing}/policies`, `${indexing}/start`, `${indexingRun}/pause`, `${indexingRun}/resume`,
    `${indexingRun}/cancel`, `${indexingJob}/redrive`, '/__operator/api/indexing/queue'];
  for (const path of posts) assert.equal((await f.request(path, { method: 'POST', headers: { origin: f.origin, 'content-type': 'application/json' }, body: '{}' })).status, 200, path);
  assert.deepEqual(f.calls.slice(0, gets.length).map(call => call.url), gets.map(path => path.replace('/__operator', '')));
  const before = f.calls.length;
  for (const path of [`${indexing}/policies?limit=201`, `${indexing}/policies?after=1`,
    `${indexingRun}/execution?limit=1`, `${indexing}/policies/${id}/progress?after=${id}`,
    `${indexingRun}/jobs?limit=201`, `${indexingRun}/jobs?pending_only=true`,
    `${indexingJob}/attempts?after=9223372036854775808`, `${indexingJob}/attempts?after=01`,
    `${indexingJob}/attempts?after=-1`, `${indexingJob}/attempts?after=1&after=2`, `${indexingJob}/attempts?limit=201`,
    `${indexingJob}/redrives?after=2147483648`, `${indexingJob}/redrives?after=${id}`,
    `${indexingJob}/redrive`, '/__operator/api/indexing/queue']) {
    assert.equal((await f.request(path)).status, 400, path);
  }
  for (const path of [`${indexingJob}/payload`, `${indexingRun}/jobs/%2e%2e/attempts`]) assert.equal((await f.request(path)).status, 404);
  assert.equal(f.calls.length, before);
});

test('canonical indexing preserves exact JSON receipts and wide response fields end to end', async t => {
  const response = '{ "version":9007199254740993,"observed_records":9223372036854775807,"queue":{"claimable":null,"ready":9007199254740993},"attempt":{"fence":9223372036854775807} }\n';
  const f = await fixture(t, { respond(req, res) { res.writeHead(200, { 'content-type': 'application/json' }); res.end(response); } });
  const bodies = [
    [`${indexing}/policies`, `{"request_id":"${id}","existing":{"policy_id":"${id}","expected_version":9007199254740993},"configuration":{"indexing":1}}`],
    [`${indexing}/start`, `{"request_id":"${id}","policy_id":"${id}","policy_version":9223372036854775807,"scope":{"library_ids":["b","a"],"item_kinds":["series","movie"],"item_ids":[]}}`],
    [`${indexingJob}/redrive`, `{"request_id":"${id}"}`],
    ['/__operator/api/indexing/queue', '{"run_id":null,"capabilities":null}'],
    ['/__operator/api/indexing/queue', '{"run_id":null,"capabilities":[]}']
  ];
  for (const [path, body] of bodies) {
    const result = await f.request(path, { method: 'POST', headers: { origin: f.origin, 'content-type': 'application/json' }, body });
    assert.equal(result.status, 200);
    assert.equal(result.body, response);
    assert.equal(f.calls.at(-1).body, body);
    assert.equal(f.calls.at(-1).headers.origin, f.target);
    assert.equal(f.calls.at(-1).headers['content-type'], 'application/json');
  }
});

test('retired maintenance routes have no compatibility forwarding', async t => {
  const f = await fixture(t);
  for (const path of [`${base}/policies`, `${base}/runs`, `${base}/runs/${id}`, `${base}/runs/${id}/candidates`]) {
    assert.equal((await f.request(path)).status, 404, path);
  }
  for (const path of ['status', 'policies', 'preview', 'start', `runs/${id}/pause`, `runs/${id}/resume`, `runs/${id}/cancel`]) {
    const response = await f.request(`${base}/${path}`, { method: 'POST', headers: { origin: f.origin, 'content-type': 'application/json' }, body: '{}' });
    assert.equal(response.status, 404, path);
    assert.equal(response.body, '{"error":{"code":"not_found"}}');
  }
  assert.equal(f.calls.length, 0);
});

test('connection routes preserve transient request bytes and exact saved policy integers', async t => {
  const saved = `{"connection":{"id":"${id}","source_id":"${id}","source_key":"jellyfin_synthetic","display_name":"Synthetic","server_version":"10.10.7","state":"enabled"},"policy_id":"${id}","policy_version":9007199254740993,"run_id":null}\n`;
  const f = await fixture(t, { respond(req, res) { res.writeHead(200, { 'content-type': 'application/json' }); res.end(saved); } });
  const path = '/__operator/api/connections/jellyfin';
  const body = `{"request_id":"${id}","base_url":"http://provider.test/jellyfin","api_key":"synthetic-secret","server_id":"server","profile_id":"profile","library_ids":["shows","movies"],"local_user_id":"${id}","refresh":false}`;
  for (let i = 0; i < 2; i++) {
    const response = await f.request(path, { method: 'POST', headers: { origin: f.origin, 'content-type': 'application/json' }, body });
    assert.equal(response.status, 200); assert.equal(response.body, saved);
    assert.equal(f.calls.at(-1).body, body); assert.equal(f.calls.at(-1).url, '/api/connections/jellyfin');
  }
  for (const query of ['limit=201', 'after=not-a-uuid', 'limit=1&limit=2', 'pending_only=true', 'api_key=synthetic-secret']) assert.equal((await f.request(`${path}?${query}`)).status, 400);
  assert.equal((await f.request(`${path}?after=${id}&limit=200`)).status, 200);
  assert.equal((await f.request(`${path}/discover`)).status, 400);
  for (const suffix of ['/rotate', '/replace', `/${id}`]) assert.equal((await f.request(path + suffix)).status, 404);
  assert.equal(f.calls.length, 3);
});

test('closed connection errors keep only code and never coerce upstream objects', async t => {
  const groups = [
    [400, ['invalid_connection', 'invalid_discovery', 'invalid_credentials', 'credentials_rejected', 'invalid_server_identity', 'invalid_selection', 'profile_unavailable', 'library_unavailable', 'unsupported_library', 'unsupported_library_scope', 'selection_capacity', 'local_user_unavailable']],
    [409, ['request_conflict', 'request_expired', 'server_identity_mismatch', 'server_already_connected']],
    [503, ['connection_unavailable', 'credential_store_unavailable', 'discovery_unavailable']]
  ];
  let code; let status;
  const f = await fixture(t, { respond(req, res) {
    res.writeHead(status, { 'content-type': 'application/json', 'x-private': 'synthetic-secret' });
    res.end(JSON.stringify({ error: { code, api_key: 'synthetic-secret', base_url: 'http://private/locator' } }));
  } });
  for (const group of groups) {
    status = group[0];
    for (code of group[1]) {
      const result = await f.request('/__operator/api/connections/jellyfin', { method: 'POST', headers: { origin: f.origin, 'content-type': 'application/json' }, body: '{}' });
      assert.equal(result.status, status); assert.equal(result.body, JSON.stringify({ error: { code } }));
      assert.equal(result.headers['x-private'], undefined);
    }
  }
  status = 400;
  for (code of [{ toString: null, valueOf: null }, ['invalid_connection'], null, 42, 'connection_unavailable', 'unknown']) {
    const result = await f.request(); assert.equal(result.status, 502); assert.equal(result.body, '{"error":{"code":"unavailable"}}');
  }
});

test('canonical mutations retain authority, MIME, upload and query guards before forwarding', async t => {
  const f = await fixture(t);
  for (const path of [`${indexing}/policies`, `${indexing}/start`, `${indexingJob}/redrive`, '/__operator/api/indexing/queue', '/__operator/api/connections/jellyfin', '/__operator/api/connections/jellyfin/discover']) {
    const headers = { origin: f.origin, 'content-type': 'application/json' };
    assert.equal((await f.request(path, { method: 'POST', headers: { ...headers, host: 'evil.invalid' }, body: '{}' })).status, 403);
    assert.equal((await f.request(path, { method: 'POST', headers: { 'content-type': 'application/json' }, body: '{}' })).status, 403);
    assert.equal((await f.request(path, { method: 'POST', headers: { ...headers, 'content-type': 'application/json; charset=utf-8' }, body: '{}' })).status, 415);
    assert.equal((await f.request(path, { method: 'POST', headers, body: 'x'.repeat(65537) })).status, 413);
    assert.equal((await f.request(`${path}?limit=1`, { method: 'POST', headers, body: '{}' })).status, 400);
  }
  assert.equal(f.calls.length, 0);
});

test('foreign, missing, duplicate authority and fetch metadata reject before upstream', async t => {
  const f = await fixture(t);
  const host = new URL(f.origin).host;
  const cases = [
    { headers: { host: 'evil.test' } },
    { rawHeaders: ['Host', host, 'Host', host] },
    { headers: { origin: 'http://evil.test' } },
    { rawHeaders: ['Host', host, 'Origin', f.origin, 'Origin', f.origin] },
    { headers: { 'sec-fetch-site': 'cross-site' } }, { headers: { 'sec-fetch-site': 'same-site' } },
    { rawHeaders: ['Host', host, 'Sec-Fetch-Site', 'none', 'Sec-Fetch-Site', 'none'] }
  ];
  for (const options of cases) assert.equal((await f.request(undefined, options)).status, 403, JSON.stringify(options));
  assert.equal((await f.request(undefined, { rawHeaders: [] })).status, 400);
  for (const origin of [undefined, 'null', 'http://evil.test', `${f.origin}/`]) {
    const headers = { 'content-type': 'application/json' };
    if (origin !== undefined) headers.origin = origin;
    assert.equal((await f.request(`${indexing}/start`, { method: 'POST', headers, body: '{}' })).status, 403);
  }
  assert.equal(f.calls.length, 0);
});

test('remote peer cannot use trusted Host and Origin', async t => {
  const f = await fixture(t, { peer: '192.168.1.1' });
  assert.equal((await f.request(undefined, { headers: { origin: f.origin } })).status, 403);
  assert.equal(f.calls.length, 0);
});

test('paths, methods, duplicate and unknown query parameters fail closed', async t => {
  const f = await fixture(t);
  const absent = ['/__operator/api/../health', '/__operator/api/%2e%2e/health', '/__operator/api/health%2f',
    '/__operator/api/health\\x', '/__operator/api/health/extra', '/__operator/api/sources/not-a-uuid/runs'];
  for (const path of absent) assert.equal((await f.request(path)).status, 404, path);
  const paths = [
    '/__operator/api/sources?limit=0', '/__operator/api/sources?limit=1001', '/__operator/api/sources?limit=1&limit=2',
    '/__operator/api/sources?target=http://evil', '/__operator/api/sources?unknown=1', '/__operator/api/sources?after=not-a-uuid',
    '/__operator/api/sources?pending_only=true', '/__operator/api/health?limit=1', `${indexingRun}?limit=1`,
    `${indexingRun}/candidates?pending_only=1`, `${base}/libraries?after=%00`, `${base}/libraries?after=%GG`,
    `${base}/libraries?after=${'x'.repeat(201)}`, '/__operator/api/sources?', '/__operator/api/sources?limit=1?limit=2'];
  for (const path of paths) assert.equal((await f.request(path)).status, 400, path);
  for (const method of ['PUT', 'DELETE', 'PATCH', 'HEAD', 'OPTIONS']) assert.equal((await f.request(undefined, { method })).status, 405, method);
  assert.equal((await f.request(`${indexing}/start?limit=1`, { method: 'POST', headers: { origin: f.origin, 'content-type': 'application/json' }, body: '{}' })).status, 400);
  assert.equal(f.calls.length, 0);
});

test('content type, GET bodies, declared and streamed oversized bodies reject before upstream', async t => {
  const f = await fixture(t);
  for (const contentType of [undefined, 'text/plain', 'application/json; charset=utf-8']) {
    const headers = { origin: f.origin };
    if (contentType) headers['content-type'] = contentType;
    assert.equal((await f.request(`${indexing}/start`, { method: 'POST', headers, body: '{}' })).status, 415);
  }
  assert.equal((await f.request(`${indexing}/start`, { method: 'POST', rawHeaders: ['Host', new URL(f.origin).host, 'Origin', f.origin,
    'Content-Type', 'application/json', 'Content-Type', 'application/json'], body: '{}' })).status, 415);
  assert.equal((await f.request(undefined, { headers: { 'content-length': '2' }, body: '{}' })).status, 400);
  for (const headers of [{ 'content-length': '65537' }, { 'transfer-encoding': 'chunked' }]) {
    assert.equal((await f.request(`${indexing}/start`, { method: 'POST', headers: { origin: f.origin, 'content-type': 'application/json', ...headers }, body: 'x'.repeat(65537) })).status, 413);
  }
  assert.equal(f.calls.length, 0);
});

test('redirects are not followed; errors and response headers cannot leak upstream diagnostics', async t => {
  const f = await fixture(t, { respond(req, res) {
    if (req.url === '/api/health') {
      res.writeHead(302, { location: 'http://127.0.0.1:1/private', 'content-type': 'application/json' });
      res.end('{"debug":"secret"}');
    } else {
      res.writeHead(409, { 'content-type': 'application/json' });
      res.end(`{"error":{"code":"source_busy","run_id":"${id}","debug":"secret"},"debug":"secret"}`);
    }
  } });
  const response = await f.request();
  assert.equal(response.status, 502);
  assert.equal(response.body, '{"error":{"code":"unavailable"}}');
  assert.equal(f.calls.length, 1);
  const busy = await f.request('/__operator/api/sources');
  assert.equal(busy.status, 409);
  assert.deepEqual(JSON.parse(busy.body), { error: { code: 'source_busy', run_id: id } });
});

test('non-string upstream source-busy IDs are sanitized without terminating the server', async t => {
  let runId;
  const f = await fixture(t, { respond(req, res) {
    res.writeHead(req.url === '/api/health' ? 200 : 409, { 'content-type': 'application/json' });
    res.end(req.url === '/api/health' ? '{"ready":true}' : JSON.stringify({ error: { code: 'source_busy', run_id: runId } }));
  } });
  for (runId of [{ toString: null, valueOf: null }, [id], {}, 42, true, null]) {
    const busy = await f.request('/__operator/api/sources');
    assert.equal(busy.status, 409);
    assert.equal(busy.body, '{"error":{"code":"source_busy"}}');
    const next = await f.request();
    assert.equal(next.status, 200);
    assert.equal(next.body, '{"ready":true}');
  }
  assert.equal(f.calls.length, 12);
});

test('timeouts close upstream with one attempt and bounded response size is enforced', async t => {
  let closed;
  const upstreamClosed = new Promise(resolve => { closed = resolve; });
  const slow = await fixture(t, { timeoutMs: 100, respond(req, res) { res.on('close', closed); } });
  assert.equal((await slow.request()).status, 504);
  await upstreamClosed;
  assert.equal(slow.calls.length, 1);
  const large = await fixture(t, { responseLimit: 100, respond(req, res) {
    res.writeHead(200, { 'content-type': 'application/json' });
    res.end(JSON.stringify({ huge: 'x'.repeat(200) }));
  } });
  assert.equal((await large.request()).status, 502);
  assert.equal(large.calls.length, 1);
});

test('client disconnect cancels upstream rather than retrying', { timeout: 2000 }, async t => {
  let ready;
  let closed;
  const upstreamReady = new Promise(resolve => { ready = resolve; });
  const upstreamClosed = new Promise(resolve => { closed = resolve; });
  const f = await fixture(t, { respond(req, res) { res.on('close', closed); ready(); } });
  const req = http.get(f.origin + '/__operator/api/health');
  req.on('error', () => {});
  await upstreamReady;
  req.destroy();
  await upstreamClosed;
  assert.equal(f.calls.length, 1);
});

test('malformed upstream JSON, non-JSON and unknown or server errors return sanitized errors', async t => {
  for (const [status, contentType, body] of [
    [200, 'application/json', '{malformed secret'], [200, 'text/html', '<h1>secret</h1>'],
    [200, 'application/json', Buffer.from([0x22, 0xff, 0x22])],
    [400, 'application/json', '{"error":{"code":"private_debug","debug":"secret"}}'],
    [400, 'application/json', '{"error":{"code":{"toString":null,"valueOf":null}}}'],
    [503, 'application/json', '{"error":{"code":"unavailable","debug":"secret"}}']
  ]) {
    const f = await fixture(t, { respond(req, res) { res.writeHead(status, { 'content-type': contentType }); res.end(body); } });
    const response = await f.request();
    assert.equal(response.status, 502);
    assert.equal(response.body, '{"error":{"code":"unavailable"}}');
    assert.equal(f.calls.length, 1);
  }
});

test('request upload deadline prevents forwarding incomplete bodies; exact body limit is accepted', { timeout: 2000 }, async t => {
  const slow = await fixture(t, { timeoutMs: 100 });
  const response = await slow.request(`${indexing}/start`, {
    method: 'POST', headers: { origin: slow.origin, 'content-type': 'application/json', 'content-length': '2' }
  });
  assert.equal(response.status, 504);
  assert.equal(slow.calls.length, 0);
  const f = await fixture(t);
  assert.equal((await f.request(`${indexing}/start`, {
    method: 'POST', headers: { origin: f.origin, 'content-type': 'application/json', 'content-length': '65536' }, body: 'x'.repeat(65536)
  })).status, 200);
  assert.equal(f.calls[0].body.length, 65536);
});
