import http from 'node:http';
import { isIP } from 'node:net';
import { createHash } from 'node:crypto';

export const OPERATOR_PREFIX = '/__operator/api';
const BODY_LIMIT = 65536;
const RESPONSE_LIMIT = 2 * 1024 * 1024;
const UUID = '[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}';
const uuid = new RegExp(`^${UUID}$`);
const CONNECTION_ERRORS = new Map([
  ...['invalid_connection', 'invalid_discovery', 'invalid_credentials', 'credentials_rejected', 'invalid_server_identity',
    'invalid_selection', 'profile_unavailable', 'library_unavailable', 'unsupported_library', 'unsupported_library_scope',
    'selection_capacity', 'local_user_unavailable'].map(code => [code, 400]),
  ...['request_conflict', 'request_expired', 'server_identity_mismatch', 'server_already_connected'].map(code => [code, 409]),
  ...['connection_unavailable', 'credential_store_unavailable', 'discovery_unavailable'].map(code => [code, 503])
]);

export function isLoopback(address) {
  if (address === '::1') return true;
  const v4 = address?.startsWith('::ffff:') ? address.slice(7) : address;
  return isIP(v4 ?? '') === 4 && v4.startsWith('127.');
}

function canonicalOrigin(value) {
  let url;
  try { url = new URL(value); } catch { throw new Error('Invalid Web operator origin configuration'); }
  if (url.protocol !== 'http:' || url.origin !== value || url.port === '0' || url.username || url.password ||
      !isLoopback(url.hostname.replace(/^\[|\]$/g, ''))) {
    throw new Error('Invalid Web operator origin configuration');
  }
  return url;
}

export function readOperatorConfig(env = process.env) {
  const target = env.SCREENROOM_WEB_OPERATOR_TARGET;
  const origin = env.SCREENROOM_WEB_OPERATOR_ORIGIN;
  if (target === undefined && origin === undefined) return null;
  if (!target || !origin) throw new Error('Web operator target and origin must both be configured');
  const targetURL = canonicalOrigin(target);
  canonicalOrigin(origin);
  return Object.freeze({ target: targetURL.origin, origin });
}

export function operatorBrowserConfig(config) {
  return Object.freeze({
    enabled: Boolean(config),
    prefix: OPERATOR_PREFIX,
    attachmentIdentity: config ? `web-operator:${createHash('sha256').update(JSON.stringify([config.target, config.origin])).digest('hex')}` : null
  });
}

function one(req, name) {
  const values = [];
  for (let i = 0; i < req.rawHeaders.length; i += 2) {
    if (req.rawHeaders[i].toLowerCase() === name) values.push(req.rawHeaders[i + 1]);
  }
  return values.length === 1 ? values[0] : undefined;
}

function present(req, name) {
  return req.rawHeaders.some((value, i) => i % 2 === 0 && value.toLowerCase() === name);
}

function route(method, raw) {
  if (raw.length > 2048 || /[\\#]/.test(raw)) return false;
  const [path, query = '', ...extra] = raw.split('?');
  if (path.includes('%') || path.split('/').some(p => p === '.' || p === '..')) return false;
  if (extra.length || (raw.includes('?') && !query)) return false;
  const source = `${OPERATOR_PREFIX}/sources/${UUID}`;
  const indexing = `${OPERATOR_PREFIX}/indexing/sources/${UUID}`;
  const run = `${indexing}/runs/${UUID}`;
  const job = `${run}/jobs/${UUID}`;
  let paged = false;
  let libraries = false;
  let candidates = false;
  let history = null;
  let indexingPage = false;
  if (method === 'GET') {
    paged = path === `${OPERATOR_PREFIX}/sources` || new RegExp(`^(${source}/libraries|${indexing}/runs)$`).test(path);
    libraries = path.endsWith('/libraries');
    candidates = new RegExp(`^${run}/candidates$`).test(path);
    indexingPage = path === `${OPERATOR_PREFIX}/connections/jellyfin` || new RegExp(`^(${indexing}/policies|${run}/jobs)$`).test(path);
    history = new RegExp(`^${job}/(attempts|redrives)$`).exec(path)?.[1] ?? null;
    if (!(paged || candidates || indexingPage || history || path === `${OPERATOR_PREFIX}/health` ||
        new RegExp(`^(${indexing}/policies/${UUID}/progress|${run}(/execution)?)$`).test(path))) return false;
  } else if (method === 'POST') {
    if (!new RegExp(`^${OPERATOR_PREFIX}/connections/jellyfin(/discover)?$`).test(path) &&
        path !== `${OPERATOR_PREFIX}/indexing/queue` &&
        !new RegExp(`^${indexing}/(policies|start)$`).test(path) &&
        !new RegExp(`^${run}/(pause|resume|cancel)$`).test(path) &&
        !new RegExp(`^${job}/redrive$`).test(path)) return false;
  } else return false;
  if (!query) return true;
  if (!paged && !candidates && !indexingPage && !history) return false;
  const seen = new Set();
  for (const field of query.split('&')) {
    const [key, value, ...rest] = field.split('=');
    if (rest.length || value === undefined || seen.has(key)) return false;
    seen.add(key);
    if (key === 'limit') { if (!/^[1-9][0-9]{0,3}$/.test(value) || Number(value) > (indexingPage || history ? 200 : 1000)) return false; }
    else if (key === 'after') {
      let decoded;
      try { decoded = decodeURIComponent(value.replace(/\+/g, ' ')); } catch { return false; }
      if (history) {
        if (!/^(0|[1-9][0-9]{0,18})$/.test(decoded) || BigInt(decoded) > (history === 'attempts' ? 9223372036854775807n : 2147483647n)) return false;
      } else if (!(libraries ? decoded.length > 0 && Buffer.byteLength(decoded) <= 200 && !/[\x00-\x1f\x7f]/.test(decoded) : uuid.test(decoded))) return false;
    } else if (key === 'pending_only') { if (!candidates || !['true', 'false'].includes(value)) return false; }
    else return false;
  }
  return true;
}

function reply(res, status, value) {
  if (res.destroyed || res.writableEnded) return;
  res.writeHead(status, {
    'content-type': 'application/json',
    'cache-control': 'no-store',
    'x-content-type-options': 'nosniff',
    connection: 'close'
  });
  res.end(typeof value === 'string' ? value : JSON.stringify(value));
}

export function createOperatorMiddleware(config, { timeoutMs = 10000, responseLimit = RESPONSE_LIMIT } = {}) {
  return (req, res, next) => {
    const raw = req.url ?? '';
    if (raw !== '/__operator' && !raw.startsWith('/__operator/') && !raw.startsWith('/__operator?')) return next();
    const fail = (status, code = 'invalid') => reply(res, status, { error: { code } });
    if (!config) return fail(404, 'unavailable');
    const mutation = req.method === 'POST';
    if (!isLoopback(req.socket.remoteAddress) || one(req, 'host') !== new URL(config.origin).host ||
        ((mutation || present(req, 'origin')) && one(req, 'origin') !== config.origin) ||
        (present(req, 'sec-fetch-site') && !['none', 'same-origin'].includes(one(req, 'sec-fetch-site')))) return fail(403);
    if (!['GET', 'POST'].includes(req.method)) return fail(405);
    if (!route(req.method, raw)) {
      const path = raw.split('?')[0];
      return route('GET', path) || route('POST', path) ? fail(400) : fail(404, 'not_found');
    }
    if (mutation && one(req, 'content-type') !== 'application/json') return fail(415);
    const length = one(req, 'content-length');
    if (present(req, 'content-length') && (length === undefined || !/^(0|[1-9][0-9]*)$/.test(length) || Number(length) > BODY_LIMIT)) return fail(413);
    if (present(req, 'transfer-encoding') && (length !== undefined || one(req, 'transfer-encoding') !== 'chunked')) return fail(400);
    if (!mutation && (present(req, 'transfer-encoding') || (length !== undefined && length !== '0'))) return fail(400);
    let upstream;
    let size = 0;
    const chunks = [];
    let done = false;
    const cleanup = () => {
      clearTimeout(timer);
      req.off('data', onData);
      req.off('end', onEnd);
      req.off('aborted', disconnected);
      res.off('close', disconnected);
    };
    const finish = (status, code) => {
      if (done) return;
      done = true;
      cleanup();
      upstream?.destroy();
      fail(status, code);
    };
    const disconnected = () => {
      if (done) return;
      done = true;
      cleanup();
      upstream?.destroy();
    };
    const timer = setTimeout(() => finish(504, 'timeout'), timeoutMs);
    const onData = chunk => {
      size += chunk.length;
      if (size > BODY_LIMIT || (!mutation && size)) return finish(413, 'invalid');
      chunks.push(chunk);
    };
    const onEnd = () => {
      if (done) return;
      if (length !== undefined && size !== Number(length)) return finish(400, 'invalid');
      const body = Buffer.concat(chunks);
      const target = new URL(config.target);
      const headers = { host: target.host, origin: target.origin, 'sec-fetch-site': 'none' };
      if (mutation) {
        headers['content-type'] = 'application/json';
        headers['content-length'] = String(body.length);
      }
      upstream = http.request({
        hostname: target.hostname.replace(/^\[|\]$/g, ''), port: target.port || 80,
        method: req.method, path: raw.replace(OPERATOR_PREFIX, '/api'), headers, agent: false
      }, incoming => {
        if (incoming.statusCode >= 300 && incoming.statusCode < 400) {
          incoming.destroy();
          return finish(502, 'unavailable');
        }
        if (incoming.headers['content-type']?.split(';')[0] !== 'application/json') {
          incoming.destroy();
          return finish(502, 'unavailable');
        }
        let received = 0;
        const output = [];
        incoming.on('data', chunk => {
          received += chunk.length;
          if (received > responseLimit) { incoming.destroy(); finish(502, 'unavailable'); }
          else output.push(chunk);
        });
        incoming.on('error', () => finish(502, 'unavailable'));
        incoming.on('end', () => {
          if (done) return;
          let value;
          let payload;
          try {
            payload = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(Buffer.concat(output));
            value = JSON.parse(payload);
          }
          catch { return finish(502, 'unavailable'); }
          const status = incoming.statusCode ?? 502;
          const code = value?.error?.code;
          const connectionStatus = CONNECTION_ERRORS.get(code);
          if (status >= 500 && !(status === 503 && connectionStatus === 503)) return finish(502, 'unavailable');
          if (status >= 400) {
            if (connectionStatus !== status && !['invalid', 'stale', 'not_found', 'source_busy', 'unavailable'].includes(code)) return finish(502, 'unavailable');
            const error = { code };
            if (code === 'source_busy' && typeof value.error.run_id === 'string' && uuid.test(value.error.run_id)) error.run_id = value.error.run_id;
            payload = JSON.stringify({ error });
          }
          done = true;
          cleanup();
          reply(res, status, payload);
        });
      });
      upstream.on('error', () => finish(502, 'unavailable'));
      upstream.end(mutation ? body : undefined);
    };
    req.on('data', onData);
    req.on('end', onEnd);
    req.on('aborted', disconnected);
    req.on('error', disconnected);
    res.on('close', disconnected);
  };
}

export function operatorProxyPlugin(config) {
  const install = server => {
    server.middlewares.use(createOperatorMiddleware(config));
  };
  return { name: 'screenroom-trusted-local-operator', configureServer: install, configurePreviewServer: install };
}
