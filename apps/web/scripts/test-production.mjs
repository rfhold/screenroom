import { spawn, spawnSync } from 'node:child_process';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { chromium, expect } from '@playwright/test';
import { isRpcPath } from '../e2e/rpc-paths.ts';
import { attachBackend } from '../e2e/backend.mjs';
import { probe, stopOwnedGroup } from '../e2e/lifecycle.mjs';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const web = fileURLToPath(new URL('../', import.meta.url));
const prepared = spawnSync(process.execPath, ['scripts/prepare-core.mjs'], { cwd: web, stdio: 'inherit' });
if (prepared.status !== 0) throw new Error('Prepared core is missing or stale');
const origin = 'http://127.0.0.1:4176';
const { endpoint, instance_id, healthStatus } = await attachBackend('web-production');
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
  const requests = [], forbidden = [], errors = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('request', request => {
    const url = new URL(request.url());
    const app = url.origin === origin && (url.pathname === '/' || url.pathname.startsWith('/_app/'));
    const api = url.origin === endpoint && (isRpcPath(url.pathname) || /^\/artwork\/[0-9a-f-]{36}\/revisions\/[1-9][0-9]*\/(native|original)$/.test(url.pathname));
    if (!app && !api && url.protocol !== 'blob:') forbidden.push(url.href);
  });
  page.on('response', response => requests.push({ url: response.url(), contentType: response.headers()['content-type'] ?? '' }));
  await page.goto(origin);
  await expect(page.locator('.rows > section')).toHaveCount(2);
  await page.locator('.credits > summary').click();
  await expect(page.locator('.credits .content > ul').first().locator(':scope > li')).toHaveCount(21);
  await expect(page.locator('.credits section[data-section]')).toHaveCount(7);
  const logo = page.locator('.credits .brand img');
  await expect(logo).toHaveAttribute('src', /^blob:/);
  await logo.evaluate(image => image.decode());
  const bytes = Buffer.from(await logo.evaluate(async image => [...new Uint8Array(await (await fetch(image.src)).arrayBuffer())]));
  expect(bytes.equals(await readFile(new URL('../../../fixtures/catalog/artwork/tmdb.png', import.meta.url)))).toBe(true);
  expect(requests.some(record => record.contentType.includes('application/wasm'))).toBe(true);
  expect(requests.some(record => record.url.endsWith('.CatalogService/GetCatalogInfo') && record.contentType.includes('grpc-web'))).toBe(true);
   expect(requests.some(record => record.url.startsWith(`${endpoint}/artwork/`) && record.contentType === 'image/png')).toBe(true);
  expect(forbidden).toEqual([]); expect(errors).toEqual([]);
  await mkdir(new URL('../e2e/build/requests/', import.meta.url), { recursive: true });
    await writeFile(new URL('../e2e/build/requests/production.json', import.meta.url), JSON.stringify({ endpoint, instance_id, healthStatus, origin, requests, artworkCredits: 21, notices: 7, logo: { bytes: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex') }, forbidden, errors }, null, 2));
  console.log('Production WASM/CatalogInfo/core-loaded logo passed.');
} finally {
  await browser?.close();
  await Promise.all(processes.map(child => stopOwnedGroup(child)));
}
