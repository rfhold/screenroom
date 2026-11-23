import { spawn, spawnSync } from 'node:child_process';
import { mkdir, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { chromium, expect } from '@playwright/test';
import { isRpcPath } from '../e2e/rpc-paths.ts';
import { attachBackend } from '../e2e/backend.mjs';
import { probe, stopOwnedGroup } from '../e2e/lifecycle.mjs';

const web = fileURLToPath(new URL('../', import.meta.url));
const prepared = spawnSync(process.execPath, ['scripts/prepare-core.mjs'], { cwd: web, stdio: 'inherit', timeout: 120_000 });
if (prepared.status !== 0) throw new Error('Prepared core is missing or stale');
const origin = 'http://127.0.0.1:4176';
const { endpoint, instance_id, healthStatus, user_id } = await attachBackend('web-production');
const processes = [];
let browser;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
async function ready(url) {
  const deadline = Date.now() + 20_000;
  while (Date.now() < deadline) {
    if (processes.some(child => child.exitCode !== null)) throw new Error('Production test server exited before readiness');
    if (await probe(url, { timeoutMs: Math.max(1, Math.min(1000, deadline - Date.now())) }).then(response => response.ok, () => false)) return;
    await delay(100);
  }
  throw new Error(`Readiness timeout: ${url}`);
}
try {
  for (const url of [origin]) {
    try {
      await probe(url, { timeoutMs: 1000 });
      throw new Error(`Refusing to reuse occupied test endpoint: ${url}`);
    } catch (error) {
      if (error.message.startsWith('Refusing')) throw error;
      if (error.message.startsWith('Probe deadline')) throw new Error(`Occupied or unresponsive test endpoint: ${url}`);
    }
  }
  processes.push(spawn('node', ['node_modules/vite/bin/vite.js', 'preview', '--host', '127.0.0.1', '--port', '4176', '--strictPort'], { cwd: web, stdio: 'inherit', detached: true }));
  await ready(origin);
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage();
  page.setDefaultTimeout(10_000);
  page.setDefaultNavigationTimeout(20_000);
  const requests = [], rpcRequests = [], forbidden = [], errors = [];
  page.on('pageerror', () => errors.push('pageerror'));
  page.on('console', message => { if (message.type() === 'error') errors.push('console error'); });
  page.on('request', request => {
    const url = new URL(request.url());
    const app = url.origin === origin && (/^\/(?:libraries(?:\/[^/]+)?|titles\/[^/]+|search|play\/[^/]+|maintenance)?$/.test(url.pathname) || url.pathname.startsWith('/_app/'));
    const api = url.origin === endpoint && (isRpcPath(url.pathname) || /^\/artwork\/[0-9a-f-]{36}\/revisions\/[1-9][0-9]*\/(native|original)$/.test(url.pathname));
    if (url.origin === endpoint && isRpcPath(url.pathname)) rpcRequests.push(url.pathname);
    if (!app && !api && !(url.protocol === 'blob:' && url.origin === origin)) forbidden.push('unexpected request');
  });
  page.on('response', response => {
    const url = new URL(response.url());
    const path = url.origin === endpoint && isRpcPath(url.pathname) ? url.pathname
      : url.origin === endpoint && url.pathname.startsWith('/artwork/') ? '/artwork/'
      : url.origin === origin && url.pathname.startsWith('/_app/') ? '/_app/'
      : url.origin === origin && /^\/(?:libraries(?:\/[^/]+)?|titles\/[^/]+|search|play\/[^/]+|maintenance)?$/.test(url.pathname) ? '/route' : url.protocol === 'blob:' ? 'blob:' : 'unexpected';
    const rawType = (response.headers()['content-type'] ?? '').split(';')[0].trim();
    const contentType = /^(application\/(wasm|grpc-web(?:\+proto|-text(?:\+proto)?)?|javascript)|text\/(html|css|javascript)|image\/(png|jpeg|webp|avif|gif|svg\+xml))$/.test(rawType) ? rawType : 'other';
    requests.push({ path, contentType, status: response.status() });
  });
  await page.goto(origin);
  await expect(page.getByRole('heading', { name: 'Home', exact: true }), 'Production artifact must load Home using the attached installation endpoint and actual user selector; rebuild with SCREENROOM_API_ENDPOINT and SCREENROOM_USER_ID if incompatible.').toBeVisible();
  await expect(page.getByText('Loading Screenroom…', { exact: true })).toHaveCount(0);
  await expect(page.getByRole('alert')).toHaveCount(0);
  await expect(page.locator('.settings-warning')).toHaveCount(0);
  const configurationMatches = await page.evaluate(({ endpoint, user_id }) => Object.keys(localStorage).some(key => {
    if (!key.startsWith('screenroom.settings.v1:')) return false;
    try {
      const [appOrigin, configuredEndpoint, configuredUser] = JSON.parse(key.slice('screenroom.settings.v1:'.length));
      return appOrigin === location.origin && configuredEndpoint === endpoint && configuredUser === user_id;
    } catch { return false; }
  }), { endpoint, user_id });
  expect(configurationMatches, 'Production artifact configuration must match the attached descriptor endpoint and user; rebuild instead of substituting a demo user.').toBe(true);
  await expect(page.getByRole('link', { name: 'Browse libraries', exact: true })).toBeVisible();
  await expect(page.locator('.rows')).toHaveCount(1);
  const empty = page.getByRole('heading', { name: 'Your library starts here', exact: true });
  const emptyInstallation = await empty.count() > 0;
  if (emptyInstallation) {
    await expect(empty).toBeVisible();
    await expect(page.getByText(/No libraries are available|Add your Jellyfin server to bring your library into Screenroom\./)).toBeVisible();
  }
  await expect(page.locator('footer, .provider-attribution')).toHaveCount(0);
  const posters = page.locator('.rows .poster img');
  const artwork = await Promise.all(Array.from({ length: await posters.count() }, async (_, index) => {
    const logo = posters.nth(index);
    await expect(logo).toHaveAttribute('src', /^blob:/);
    expect(await logo.evaluate((image, origin) => image.src.startsWith(`blob:${origin}/`), origin)).toBe(true);
    await expect.poll(() => logo.evaluate(image => image.complete && image.naturalWidth > 0 && image.naturalHeight > 0)).toBe(true);
    const bytes = Buffer.from(await logo.evaluate(async image => {
      await Promise.race([image.decode(), new Promise((_, reject) => setTimeout(() => reject(new Error('Poster image decode timed out')), 5000))]);
      const response = await fetch(image.src, { signal: AbortSignal.timeout(5000) });
      if (!response.ok) throw new Error('Poster blob unavailable');
      return [...new Uint8Array(await response.arrayBuffer())];
    }));
    expect(bytes.length).toBeGreaterThan(0);
    return { bytes: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex') };
  }));
  expect(rpcRequests.some(path => path.endsWith('/ListArtwork'))).toBe(false);
  expect(requests.some(record => record.contentType.includes('application/wasm') && record.status === 200)).toBe(true);
  expect(rpcRequests.some(path => path.endsWith('/GetCatalogInfo'))).toBe(false);
  expect(requests.some(record => record.path.endsWith('.BrowseService/GetHome') && record.contentType.includes('grpc-web') && record.status === 200)).toBe(true);
  if (artwork.length) expect(requests.some(record => record.path === '/artwork/' && record.status === 200)).toBe(true);
  expect(forbidden).toEqual([]); expect(errors).toEqual([]);
  await mkdir(new URL('../e2e/build/requests/', import.meta.url), { recursive: true });
  await writeFile(new URL('../e2e/build/requests/production.json', import.meta.url), JSON.stringify({ endpoint, instance_id, healthStatus, origin, requests, rpcRequests, emptyInstallation, artwork, forbidden, errors }, null, 2));
  console.log('Production WASM/source-driven Home and core-loaded item artwork passed without attribution footers.');
} finally {
  try { await browser?.close(); }
  finally { await Promise.all(processes.map(child => stopOwnedGroup(child))); }
}
