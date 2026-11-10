import { createRequire } from 'node:module';
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { once } from 'node:events';
import { fileURLToPath } from 'node:url';
import assert from 'node:assert/strict';

// Attach only. The caller owns backend startup, seed, CORS configuration and shutdown.
const endpoint = process.env.SCREENROOM_API_ENDPOINT;
const userID = process.env.SCREENROOM_USER_ID;
if (!endpoint || !userID) throw new Error('Attach to a seeded backend with SCREENROOM_API_ENDPOINT and explicit SCREENROOM_USER_ID');
if (process.env.SCREENROOM_E2E_API_ENDPOINT && process.env.SCREENROOM_E2E_API_ENDPOINT !== endpoint) throw new Error('SCREENROOM_E2E_API_ENDPOINT must equal SCREENROOM_API_ENDPOINT');
const api = new URL(endpoint);
if (!['http:', 'https:'].includes(api.protocol) || api.origin !== endpoint) throw new Error('SCREENROOM_API_ENDPOINT must be an HTTP(S) origin with explicit port and no trailing slash');
if (!api.port) throw new Error('SCREENROOM_API_ENDPOINT requires an explicit port');
const origin = process.env.SCREENROOM_CORE_BROWSER_ORIGIN ?? 'http://127.0.0.1:4173';
const browserOrigin = new URL(origin);
if (browserOrigin.origin !== origin || browserOrigin.protocol !== 'http:' || !['127.0.0.1', 'localhost'].includes(browserOrigin.hostname) || !browserOrigin.port) throw new Error('SCREENROOM_CORE_BROWSER_ORIGIN must be a loopback HTTP origin with explicit port; configure backend CORS for this origin');
const root = fileURLToPath(new URL('../', import.meta.url));
const require = createRequire(new URL('../apps/web/package.json', import.meta.url));
const { chromium } = require('@playwright/test');
const staticServer = createServer(async (req, res) => {
  const name = req.url?.slice(1);
  if (name === '') { res.setHeader('content-type', 'text/html'); res.end('<!doctype html><title>Persisted backend core smoke</title>'); return; }
  if (name === 'traversal.mjs') { res.setHeader('content-type', 'text/javascript'); res.end(await readFile(`${root}/crates/screenroom-client-core/tests/support/traversal.mjs`)); return; }
  if (!['screenroom_client_core.js', 'screenroom_client_core_bg.wasm'].includes(name)) { res.writeHead(404).end(); return; }
  try { res.setHeader('content-type', name.endsWith('.wasm') ? 'application/wasm' : 'text/javascript'); res.end(await readFile(`${root}/target/client-core/wasm/${name}`)); }
  catch { res.writeHead(404).end(); }
});
staticServer.listen(Number(browserOrigin.port), browserOrigin.hostname);
await once(staticServer, 'listening');
let browser;
const smokeDeadline = Date.now() + 120_000;
try {
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage();
  const requests = [];
  const externalRequests = [];
  await page.route('**/*', route => {
    const url = route.request().url();
    if (!url.startsWith(`${origin}/`) && !url.startsWith(`${endpoint}/`)) { externalRequests.push(url); return route.abort(); }
    return route.continue();
  });
  page.on('request', request => { if (request.url().startsWith(`${endpoint}/`)) requests.push({ url: request.url(), contentType: request.headers()['content-type'] }); });
  await page.goto(origin);
  const result = await page.evaluate(async ({ endpoint, userID, remainingMS }) => {
    const { collectPages, withDeadline } = await import('/traversal.mjs');
    let c;
    return withDeadline(async () => {
    const { default: init, Client, default_settings, validate_settings } = await import('/screenroom_client_core.js'); await init();
    c = new Client({ endpoint, user_id: userID, timeout_ms: 5000 });
    const scope = { source_id: '', library_id: '' };
    const page = { size: 2, token: '' };
    const settings = validate_settings(default_settings());
    const currentUser = await c.get_current_user();
    if (currentUser.id.toLowerCase() !== userID.toLowerCase()) throw new Error('local user selector changed');
    const rows = await c.get_home(scope);
    const info = await c.get_catalog_info();
    if (info.notices.length > 32 || info.branding.length > 8 || 'artwork' in info) throw new Error('catalog startup payload is not bounded');
    const libraries = await collectPages('libraries', token => c.list_libraries('', { ...page, token }), 'libraries', () => c.cancel_all());
    const items = await c.list_library_items(scope, page);
    if (!items.items.length) throw new Error('backend seed contains no browseable items');
    const item = await c.get_item(items.items[0].id, scope);
    const children = await c.list_children(item.id, page, scope);
    const credits = await c.list_credits(item.id, page, scope);
    const caps = await c.get_search_capabilities();
    const search = await c.search({ query: item.title, library_id: '', source_id: '', source: 'all', types: null, methods: { title_only: false, text: true, fuzzy: true, semantic: true }, page });
    if (!search.effective_methods || (search.degradation_reason && search.effective_methods.semantic)) throw new Error('semantic fallback status missing or dishonest');
    // Explicit credits action: collect every metadata page, but never eagerly load every image.
    const artwork = await collectPages('artwork', token => c.list_artwork(scope, { ...page, token }), 'artwork', () => c.cancel_all());
    if (!artwork.length || new Set(artwork.map(a => a.id)).size !== artwork.length) throw new Error('empty or duplicate full artwork credits');
    const logo = info.branding.find(a => a.kind === 'logo');
    if (!logo) throw new Error('seeded branding logo missing');
    const logoBytes = await c.load_artwork(logo.url);
    const artBytes = item.artwork ? await c.load_artwork(item.artwork.url) : [];
    let missing; try { await c.get_item('00000000-0000-0000-0000-000000000000', scope); } catch (e) { missing = e.code; }
    globalThis.roundtripClient = c;
    return { currentUser, rows: rows.length, libraries: libraries.length, artwork: artwork.length, notices: info.notices.length, branding: info.branding.length, logoBytes: logoBytes.length, artBytes: artBytes.length, title: item.title, children: children.items.length, credits: credits.credits.length, caps, effectiveMethods: search.effective_methods, degradationReason: search.degradation_reason, settings, missing };
    }, () => c?.cancel_all(), remainingMS);
  }, { endpoint, userID, remainingMS: smokeDeadline - Date.now() });
  assert.ok(result.logoBytes > 0);
  assert.equal(result.missing, 'not_found');
  assert.deepEqual(externalRequests, []);
  await page.route('**/screenroom.catalog.v1.BrowseService/GetHome', async route => { await new Promise(resolve => setTimeout(resolve, 200)); try { await route.continue(); } catch {} });
  const cancellation = await page.evaluate(async remainingMS => {
    const { withDeadline } = await import('/traversal.mjs');
    const c = globalThis.roundtripClient; c.invalidate_reads();
    return withDeadline(async () => {
    const pending = c.get_home({ source_id: '', library_id: '' }).then(() => 'unexpected success', e => e.code);
    await new Promise(resolve => setTimeout(resolve, 20)); c.cancel_all(); return await pending;
    }, () => c.cancel_all(), remainingMS);
  }, smokeDeadline - Date.now());
  assert.equal(cancellation, 'cancelled');
  assert.ok(requests.some(r => r.contentType?.startsWith('application/grpc-web')));
  assert.ok(requests.some(r => /\/artwork\/[\da-f-]+\/revisions\/[1-9]\d*\/(native|original)$/.test(r.url)));
  assert.ok(!requests.some(r => /PreferencesService|mock-profile/.test(r.url)));
  console.log(JSON.stringify({ transport: 'attached persisted backend: Chromium WASM gRPC-Web + artwork', endpoint, cancellation, ...result, requestCount: requests.length }, null, 2));
} finally {
  if (browser) await browser.close();
  await new Promise(resolve => staticServer.close(resolve));
}
