import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import http from 'node:http';
import http2 from 'node:http2';
import { resolve } from 'node:path';

const targets = [
  ['screenroom-catalog', 'SCREENROOM_CATALOG', '/screenroom.catalog.v1.BrowseService/GetHome'],
  ['screenroom-user', 'SCREENROOM_USER', '/screenroom.user.v1.UserService/GetCurrentUser'],
  ['screenroom-playback-gateway', 'SCREENROOM_PLAYBACK_GATEWAY', null],
].filter(([name]) => !process.env.SCREENROOM_SMOKE_TARGETS || process.env.SCREENROOM_SMOKE_TARGETS.split(',').includes(name));
const unknown = '/screenroom.unknown.v1.Unknown/Unknown';
const children = new Set();
const frame = Buffer.alloc(5);
const traceId = 'b2611246a58fd7ea623d2264c5a1e226';
const parentId = 'b2c9b811f2f424af';
const traceparent = `00-${traceId}-${parentId}-01`;

function deadline(promise) {
  let timer;
  return Promise.race([promise, new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error('smoke test timed out')), 10000);
  })]).finally(() => clearTimeout(timer));
}

function launch(name, prefix, extra = {}) {
  const env = { ...process.env };
  for (const key of Object.keys(env)) {
    if (key.startsWith('OTEL_')) delete env[key];
  }
  Object.assign(env, {
    RUST_LOG: 'info',
    [`${prefix}_LISTEN`]: '127.0.0.1:0',
    [`${prefix}_PRIVATE_LISTEN`]: '127.0.0.1:0',
    [`${prefix}_CORS_ORIGINS`]: 'http://localhost:5173, https://browser.example',
    SCREENROOM_CATALOG_PUBLIC_ORIGIN: 'http://localhost:50052',
    ...extra,
  });
  if (env.RUST_LOG === undefined) delete env.RUST_LOG;
  const child = spawn(resolve(process.env.CARGO_TARGET_DIR ?? 'target', 'debug', name), [], { env });
  children.add(child);
  let output = '';
  let errors = '';
  child.stderr.on('data', chunk => { errors += chunk; });
  const exited = new Promise((resolve, reject) => {
    child.once('error', reject);
    child.once('close', (code, signal) => { children.delete(child); resolve({ code, signal }); });
  });
  const ready = deadline(new Promise((resolve, reject) => {
    child.stdout.on('data', chunk => {
      output += chunk;
      for (const line of output.split('\n')) {
        try {
          const event = JSON.parse(line);
          if (event.fields?.message === 'RPC server listening') resolve(event.fields.address);
        } catch {}
      }
    });
    exited.then(result => reject(new Error(`startup failed: ${JSON.stringify(result)} ${errors}`)), reject);
  }));
  ready.catch(() => {});
  return { child, ready, exited, logs: () => output + errors };
}

async function stop(server) {
  server.child.kill('SIGTERM');
  assert.deepEqual(await deadline(server.exited), { code: 0, signal: null });
}

function web(address, path, { method = 'POST', headers = {}, body = frame } = {}) {
  return deadline(new Promise((resolve, reject) => {
    const req = http.request(`http://${address}${path}`, {
      method,
      agent: false,
      headers: { 'content-type': 'application/grpc-web+proto', 'x-grpc-web': '1', ...headers },
    }, res => {
      const chunks = [];
      res.on('data', chunk => chunks.push(chunk));
      res.once('error', reject);
      res.once('end', () => resolve({ status: res.statusCode, headers: res.headers, body: Buffer.concat(chunks) }));
    });
    req.once('error', reject);
    req.end(body);
  }));
}

async function native(address, path, headers = {}) {
  const session = http2.connect(`http://${address}`);
  session.on('error', () => {});
  try {
    return await deadline(new Promise((resolve, reject) => {
      const req = session.request({ ':method': 'POST', ':path': path, 'content-type': 'application/grpc', te: 'trailers', ...headers });
      const result = { headers: {}, trailers: {}, body: [] };
      req.on('response', headers => { result.headers = headers; });
      req.on('trailers', headers => { result.trailers = headers; });
      req.on('data', chunk => result.body.push(chunk));
      req.once('error', reject);
      req.once('end', () => resolve({ ...result, resetCode: req.rstCode }));
      req.end(frame);
    }));
  } finally {
    session.destroy();
  }
}

function assertWebUnimplemented(response) {
  assert.equal(response.status, 200);
  assert.match(response.headers['content-type'], /^application\/grpc-web/);
  if (response.headers['grpc-status'] !== undefined) {
    assert.equal(response.headers['grpc-status'], '12');
    assert.equal(response.body.length, 0);
  } else {
    assert.equal(response.body[0], 0x80);
    assert.equal(response.body.readUInt32BE(1), response.body.length - 5);
    assert.match(response.body.subarray(5).toString(), /grpc-status:\s*12\r\n/);
  }
}

function webStatus(response) {
  if (response.headers['grpc-status'] !== undefined) return response.headers['grpc-status'];
  let offset = 0;
  while (offset + 5 <= response.body.length) {
    const flags = response.body[offset];
    const size = response.body.readUInt32BE(offset + 1);
    if (flags & 0x80) return response.body.subarray(offset + 5, offset + 5 + size).toString().match(/grpc-status:\s*(\d+)/)?.[1];
    offset += 5 + size;
  }
  return undefined;
}

function fields(buffer) {
  let offset = 0;
  const varint = () => {
    let value = 0n;
    let shift = 0n;
    while (offset < buffer.length) {
      const byte = buffer[offset++];
      value |= BigInt(byte & 127) << shift;
      if (!(byte & 128)) return value;
      shift += 7n;
    }
    throw new Error('truncated protobuf');
  };
  const result = [];
  while (offset < buffer.length) {
    const key = Number(varint());
    const wire = key & 7;
    let value;
    if (wire === 0) value = varint();
    else {
      const size = wire === 2 ? Number(varint()) : wire === 1 ? 8 : wire === 5 ? 4 : assert.fail('unexpected protobuf wire type');
      value = buffer.subarray(offset, offset + size);
      offset += size;
    }
    result.push({ number: key >> 3, value });
  }
  return result;
}
const values = (buffer, number) => fields(buffer).filter(field => field.number === number).map(field => field.value);
const value = (buffer, number) => values(buffer, number)[0];

let collector;
try {
  for (const [name, prefix, known] of targets) {
    const server = launch(name, prefix);
    const address = await server.ready;
    const rpc = await native(address, unknown);
    assert.equal(rpc.headers['grpc-status'] ?? rpc.trailers['grpc-status'], '12');
    assertWebUnimplemented(await web(address, unknown));
    const denied = await web(address, unknown, { headers: { origin: 'http://denied.invalid' } });
    assert.equal(denied.headers['access-control-allow-origin'], undefined);
    const preflight = await web(address, unknown, {
      method: 'OPTIONS',
      headers: { origin: 'http://localhost:5173', 'access-control-request-method': 'POST', 'access-control-request-headers': 'content-type,traceparent,tracestate' },
    });
    assert.equal(preflight.headers['access-control-allow-origin'], 'http://localhost:5173');
    assert.match(preflight.headers['access-control-allow-headers'], /traceparent/);
    assert.match(preflight.headers['access-control-allow-headers'], /tracestate/);
    const secondOrigin = await web(address, unknown, { headers: { origin: 'https://browser.example' } });
    assert.equal(secondOrigin.headers['access-control-allow-origin'], 'https://browser.example');
    if (known) {
      const malformed = await web(address, known, { body: Buffer.from([0, 0, 0, 0, 1, 255]) });
      assert.equal(malformed.headers['grpc-status'], '13');
      const response = await native(address, known);
      const expected = name === 'screenroom-catalog' ? '0' : '3';
      assert.equal(response.headers['grpc-status'] ?? response.trailers['grpc-status'], expected);
      assert.equal(webStatus(await web(address, known)), expected);
      assertWebUnimplemented(await web(address, unknown));
      assert.doesNotMatch(server.logs(), /panicked|not yet implemented/);
    }
    if (name === 'screenroom-catalog') {
      for (const privateRoute of ['/screenroom.catalog.operator.v1.IngestionService/GetPolicy', '/screenroom.user.internal.v1.PlaybackStateService/GetContinueWatching']) {
        assertWebUnimplemented(await web(address, privateRoute));
        const response = await native(address, privateRoute);
        assert.equal(response.headers['grpc-status'] ?? response.trailers['grpc-status'], '12');
      }
      const missing = await web(address, '/artwork/not-a-uuid/revisions/1/native', { method: 'GET' });
      assert.equal(missing.status, 404);
      assert.doesNotMatch(missing.body.toString(), /bucket|secret|access.key|s3/i);
    }
    if (name === 'screenroom-user') {
      for (const selector of ['not-a-uuid', '00000000-0000-0000-0000-000000000000']) {
        const headers = { 'x-screenroom-user-id': selector };
        const expected = selector === 'not-a-uuid' ? '3' : '5';
        const response = await native(address, known, headers);
        assert.equal(response.headers['grpc-status'] ?? response.trailers['grpc-status'], expected);
        assert.equal(webStatus(await web(address, known, { headers })), expected);
      }
    }
    await stop(server);
    console.log(`${name}: HTTP/2 + HTTP/1.1 grpc-web parity, CORS, honest functional statuses, private-route separation, SIGTERM drain passed`);
    for (const origin of [
      'http://127.0.0.1:4173#private-fragment',
      'http://127.0.0.1:4173?private-query',
      'http://127.0.0.1:4173/private-path',
      'http://private-user:private-password@127.0.0.1:4173',
    ]) {
      const invalid = launch(name, prefix, { [`${prefix}_CORS_ORIGINS`]: origin });
      assert.notEqual((await deadline(invalid.exited)).code, 0);
      assert.match(invalid.logs(), new RegExp(`invalid ${prefix}_CORS_ORIGINS origin`));
      assert.doesNotMatch(invalid.logs(), /private-|RPC server listening/);
    }
    const noCors = launch(name, prefix, { [`${prefix}_CORS_ORIGINS`]: '' });
    const noCorsAddress = await noCors.ready;
    const noCorsResponse = await web(noCorsAddress, unknown, { headers: { origin: 'http://localhost:5173' } });
    assertWebUnimplemented(noCorsResponse);
    assert.equal(noCorsResponse.headers['access-control-allow-origin'], undefined);
    await stop(noCors);
  }

  for (const extra of [
    { SCREENROOM_CATALOG_LISTEN: 'invalid-private-listen' },
    { OTEL_EXPORTER_OTLP_TRACES_ENDPOINT: 'private-invalid-endpoint' },
    { SCREENROOM_CATALOG_CORS_ORIGINS: '*' },
    { RUST_LOG: 'invalid[filter' },
  ]) {
    const failed = launch('screenroom-catalog', 'SCREENROOM_CATALOG', extra);
    assert.notEqual((await deadline(failed.exited)).code, 0);
    assert.doesNotMatch(failed.logs(), /invalid-private-listen|private-invalid-endpoint/);
  }
  console.log('invalid listen, endpoint, CORS and log-filter configuration fail closed without echoing values');

  const exports = [];
  const partialSuccessMessages = new Map();
  collector = http.createServer((req, res) => {
    const chunks = [];
    req.on('data', chunk => chunks.push(chunk));
    req.on('end', () => {
      exports.push({ path: req.url, type: req.headers['content-type'], body: Buffer.concat(chunks) });
      if (req.url.startsWith('/private-export-path')) {
        res.writeHead(400);
        res.end('private-collector-response');
        return;
      }
      res.writeHead(200, { 'content-type': 'application/x-protobuf' });
      if (partialSuccessMessages.has(req.url)) {
        const message = Buffer.from(partialSuccessMessages.get(req.url));
        const partial = Buffer.concat([Buffer.from([0x08, 1, 0x12, message.length]), message]);
        res.end(Buffer.concat([Buffer.from([0x0a, partial.length]), partial]));
        return;
      }
      res.end();
    });
  });
  await new Promise(resolve => collector.listen(0, '127.0.0.1', resolve));
  const endpoint = `http://127.0.0.1:${collector.address().port}`;
  const traced = launch('screenroom-catalog', 'SCREENROOM_CATALOG', { OTEL_EXPORTER_OTLP_ENDPOINT: endpoint });
  const address = await traced.ready;
  assertWebUnimplemented(await web(address, unknown, { headers: { traceparent, authorization: 'Bearer private-token', cookie: 'private-cookie' } }));
  await stop(traced);
  assert.ok(exports.length > 0, 'shutdown flush must export');
  const spans = [];
  for (const request of exports) {
    assert.equal(request.path, '/v1/traces');
    assert.equal(request.type, 'application/x-protobuf');
    assert.ok(!request.body.includes('private-token'));
    assert.ok(!request.body.includes('private-cookie'));
    for (const resourceSpans of values(request.body, 1)) {
      const resource = value(resourceSpans, 1);
      const service = values(resource, 1).find(attribute => value(attribute, 1)?.toString() === 'service.name');
      assert.equal(value(value(service, 2), 1).toString(), 'screenroom-catalog');
      for (const scopeSpans of values(resourceSpans, 2)) spans.push(...values(scopeSpans, 2));
    }
  }
  const span = spans.find(span => value(span, 1)?.toString('hex') === traceId);
  assert.ok(span, 'remote W3C trace must be exported');
  assert.equal(value(span, 4).toString('hex'), parentId);
  assert.equal(value(span, 5).toString(), 'screenroom.unknown.v1.Unknown/Unknown');
  const status = values(span, 9).find(attribute => value(attribute, 1)?.toString() === 'rpc.grpc.status_code');
  const statusValue = value(status, 2);
  assert.equal(String(value(statusValue, 3) ?? value(statusValue, 1)?.toString()), '12');

  const exportFailure = launch('screenroom-catalog', 'SCREENROOM_CATALOG', {
    RUST_LOG: 'trace',
    OTEL_EXPORTER_OTLP_ENDPOINT: `${endpoint}/unused-global-path`,
    OTEL_EXPORTER_OTLP_TRACES_ENDPOINT: `${endpoint}/private-export-path?token=private-query-secret`,
  });
  assertWebUnimplemented(await web(await exportFailure.ready, unknown));
  await stop(exportFailure);
  assert.ok(exports.some(request => request.path === '/private-export-path?token=private-query-secret'));
  assert.ok(!exports.some(request => request.path.includes('unused-global-path')));
  assert.doesNotMatch(exportFailure.logs(), /private-export-path|private-query-secret|private-collector-response/);

  // Build with opentelemetry-otlp/internal-logs to exercise the upstream warning.
  for (const filter of [undefined, 'trace']) {
    const mode = filter ?? 'default-info';
    const path = `/partial-success-${mode}`;
    const message = `private-collector-response-partial-success-${mode}-6e1b42`;
    partialSuccessMessages.set(path, message);
    const partialSuccess = launch('screenroom-catalog', 'SCREENROOM_CATALOG', {
      RUST_LOG: filter,
      OTEL_EXPORTER_OTLP_TRACES_ENDPOINT: `${endpoint}${path}`,
    });
    assertWebUnimplemented(await web(await partialSuccess.ready, unknown, { headers: { traceparent } }));
    await stop(partialSuccess);
    const requests = exports.filter(request => request.path === path);
    assert.ok(requests.length > 0, `${mode}: shutdown flush must reach the collector`);
    const receivedSpans = requests.flatMap(request => values(request.body, 1)
      .flatMap(resourceSpans => values(resourceSpans, 2))
      .flatMap(scopeSpans => values(scopeSpans, 2)));
    const receivedSpan = receivedSpans.find(span => value(span, 1)?.toString('hex') === traceId);
    assert.ok(receivedSpan, `${mode}: actual RPC span must reach the collector`);
    assert.equal(value(receivedSpan, 4).toString('hex'), parentId);
    assert.ok(!partialSuccess.logs().includes(message), `${mode}: collector message must not leak`);
    assert.doesNotMatch(partialSuccess.logs(), /HttpTraceClient.PartialSuccess/);
    assert.match(partialSuccess.logs(), /RPC server listening/);
    assert.match(partialSuccess.logs(), /screenroom.unknown.v1.Unknown/);
    console.log(`${mode}: HTTP 200 protobuf partial-success response cannot leak collector text; span received, shutdown flushed, clean exit`);
  }

  const blocker = http.createServer();
  await new Promise(resolve => blocker.listen(0, '127.0.0.1', resolve));
  try {
    const failed = launch('screenroom-catalog', 'SCREENROOM_CATALOG', {
      SCREENROOM_CATALOG_LISTEN: `127.0.0.1:${blocker.address().port}`,
      OTEL_EXPORTER_OTLP_TRACES_ENDPOINT: `${endpoint}/custom-traces`,
    });
    assert.notEqual((await deadline(failed.exited)).code, 0);
    assert.match(failed.logs(), /Address already in use|AddrInUse/);
  } finally {
    await new Promise(resolve => blocker.close(resolve));
  }
  console.log('real OTLP HTTP/protobuf shutdown flush: W3C trace + parent, service.name, RPC status, no auth/cookie capture; bind failure exits');
} finally {
  for (const child of children) child.kill('SIGKILL');
  if (collector) await new Promise(resolve => collector.close(resolve));
}
