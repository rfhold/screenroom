import { spawn } from 'node:child_process';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { chromium, expect } from '@playwright/test';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const web = fileURLToPath(new URL('../', import.meta.url));
const origin = 'http://127.0.0.1:4176';
const endpoint = 'http://127.0.0.1:50064';
const processes = [];
let browser;
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
async function ready(url) {
  for (let attempt = 0; attempt < 200; attempt++) {
    if (processes.some(child => child.exitCode !== null)) throw new Error('Production test server exited before readiness');
    if (await fetch(url).then(response => response.ok, () => false)) return;
    await delay(100);
  }
  throw new Error(`Readiness timeout: ${url}`);
}
async function stop(child) {
  if (child.exitCode !== null) return;
  const exited = new Promise(resolve => child.once('exit', resolve));
  child.kill('SIGTERM');
  const timeout = setTimeout(() => child.kill('SIGKILL'), 5000);
  await exited;
  clearTimeout(timeout);
}
try {
  for (const url of [`${endpoint}/health`, origin]) {
    if (await fetch(url).then(response => response.ok, () => false)) throw new Error(`Refusing to reuse occupied test endpoint: ${url}`);
  }
  processes.push(spawn('bash', ['scripts/mock-server.sh'], { cwd: root, stdio: 'inherit', env: { ...process.env, SCREENROOM_MOCK_LISTEN: '127.0.0.1:50064', SCREENROOM_MOCK_PUBLIC_BASE: endpoint, SCREENROOM_MOCK_CORS_ORIGINS: origin } }));
  processes.push(spawn('node', ['node_modules/vite/bin/vite.js', 'preview', '--host', '127.0.0.1', '--port', '4176', '--strictPort'], { cwd: web, stdio: 'inherit' }));
  await Promise.all([ready(`${endpoint}/health`), ready(origin)]);
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage();
  const requests = [], forbidden = [], errors = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('request', request => {
    const url = new URL(request.url());
    const app = url.origin === origin && (url.pathname === '/' || url.pathname.startsWith('/_app/'));
    const api = url.origin === endpoint && (/^\/screenroom\.client\.v1\.(Browse|Catalog|Search|Preferences)Service\/(GetHome|ListLibraries|ListLibraryItems|GetItem|ListChildren|ListCredits|GetCatalogInfo|Search|GetSearchCapabilities|GetPreferences|UpdatePreferences)$/.test(url.pathname) || /^\/artwork\/[a-z0-9-]+\.(jpg|png)$/.test(url.pathname));
    if (!app && !api && url.protocol !== 'blob:') forbidden.push(url.href);
  });
  page.on('response', response => requests.push({ url: response.url(), contentType: response.headers()['content-type'] ?? '' }));
  await page.goto(origin);
  await expect(page.locator('.rows > section')).toHaveCount(5);
  await page.locator('.credits > summary').click();
  const logo = page.locator('.credits .brand img');
  await expect(logo).toHaveAttribute('src', /^blob:/);
  await logo.evaluate(image => image.decode());
  const bytes = Buffer.from(await logo.evaluate(async image => [...new Uint8Array(await (await fetch(image.src)).arrayBuffer())]));
  expect(bytes.equals(await readFile(new URL('../../../fixtures/catalog/artwork/tmdb.png', import.meta.url)))).toBe(true);
  expect(requests.some(record => record.contentType.includes('application/wasm'))).toBe(true);
  expect(requests.some(record => record.url.endsWith('.CatalogService/GetCatalogInfo') && record.contentType.includes('grpc-web'))).toBe(true);
  expect(requests.some(record => record.url === `${endpoint}/artwork/tmdb.png` && record.contentType === 'image/png')).toBe(true);
  expect(forbidden).toEqual([]); expect(errors).toEqual([]);
  await mkdir(new URL('../e2e/build/requests/', import.meta.url), { recursive: true });
  await writeFile(new URL('../e2e/build/requests/production.json', import.meta.url), JSON.stringify({ endpoint, origin, requests, logo: { bytes: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex') }, forbidden, errors }, null, 2));
  console.log('Production WASM/CatalogInfo/core-loaded logo passed.');
} finally {
  await browser?.close();
  await Promise.all(processes.map(stop));
}
