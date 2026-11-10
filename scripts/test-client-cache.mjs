import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { once } from 'node:events';
import { fileURLToPath } from 'node:url';
import { test } from 'node:test';
import { collectPages, withDeadline } from '../crates/screenroom-client-core/tests/support/traversal.mjs';

const traversalLimits = { pages: 3, items: 4, timeoutMS: 20 };

test('library traversal rejects cached repeated tokens without partial results', async () => {
  let calls = 0;
  await assert.rejects(collectPages('libraries', async () => {
    calls++; return { libraries: [1], next_page_token: 'repeated' };
  }, 'libraries', () => {}, traversalLimits), /libraries: repeated pagination token/);
  assert.equal(calls, 2);
});

test('unique artwork tokens and item floods hit finite budgets', async () => {
  let calls = 0;
  await assert.rejects(collectPages('artwork', async () => ({ artwork: [], next_page_token: String(++calls) }), 'artwork', () => {}, traversalLimits), /artwork: page budget exceeded \(3\)/);
  assert.equal(calls, 3);
  await assert.rejects(collectPages('artwork', async () => ({ artwork: [1, 2, 3, 4, 5], next_page_token: '' }), 'artwork', () => {}, traversalLimits), /artwork: item budget exceeded \(4\)/);
  calls = 0;
  await assert.rejects(collectPages('libraries', async () => ({ libraries: [1, 2, 3], next_page_token: String(++calls) }), 'libraries', () => {}, { ...traversalLimits, pages: 10 }), /libraries: item budget exceeded \(4\)/);
});

test('overall traversal deadline cancels an awaiting request and retains budget diagnostic', async () => {
  let cancelled = 0;
  let rejectRequest;
  const pending = new Promise((_, reject) => { rejectRequest = reject; });
  await assert.rejects(collectPages('libraries', () => pending, 'libraries', () => {
    cancelled++; rejectRequest(new Error('request cancelled'));
  }, traversalLimits), /libraries traversal failed: overall time budget exceeded/);
  assert.equal(cancelled, 1);
  await assert.rejects(withDeadline(() => new Promise(() => {}), () => { cancelled++; }, 5), /overall time budget exceeded/);
  assert.equal(cancelled, 2);
});

test('complete traversal returns every item at exact item budget', async () => {
  let calls = 0;
  const result = await collectPages('artwork', async () => ({ artwork: [1, 2], next_page_token: ++calls === 2 ? '' : 'second' }), 'artwork', () => {}, traversalLimits);
  assert.deepEqual(result, [1, 2, 1, 2]);
});

test('WASM artwork transport double: bounded cache, context isolation, abort, redirects and honest errors', async () => {
  const root = fileURLToPath(new URL('../', import.meta.url));
  const require = createRequire(new URL('../apps/web/package.json', import.meta.url));
  const { chromium } = require('@playwright/test');
  const counts = new Map();
  let cancelled = false;
  const server = createServer(async (req, res) => {
    const path = req.url;
    if (path === '/') { res.setHeader('content-type', 'text/html'); res.end('<!doctype html><title>WASM cache transport unit test</title>'); return; }
    if (['/screenroom_client_core.js', '/screenroom_client_core_bg.wasm'].includes(path)) {
      res.setHeader('content-type', path.endsWith('.wasm') ? 'application/wasm' : 'text/javascript');
      res.end(await readFile(`${root}/target/client-core/wasm${path}`)); return;
    }
    counts.set(path, (counts.get(path) ?? 0) + 1);
    const revision = path.split('/')[4];
    if (revision === '2') { res.writeHead(302, { location: '/redirect-target' }).end(); return; }
    if (revision === '3') { res.writeHead(404).end(); return; }
    if (revision === '4') { res.writeHead(200, { 'content-type': 'text/html' }).end('bad'); return; }
    if (revision === '5') { res.writeHead(200, { 'content-type': 'image/png', 'content-length': '4194305' }).end(); return; }
    if (revision === '6') { res.writeHead(200, { 'content-type': 'image/png' }).end(Buffer.alloc(4194305)); return; }
    if (revision === '7') {
      res.writeHead(200, { 'content-type': 'image/png' }); res.flushHeaders(); res.write('a');
      res.on('close', () => { cancelled = true; }); return;
    }
    if (revision === '8') {
      res.writeHead(200, { 'content-type': 'image/png', 'content-length': '8' }); res.flushHeaders(); res.write('abc');
      setTimeout(() => res.destroy(), 30); return;
    }
    setTimeout(() => { res.writeHead(200, { 'content-type': 'image/png' }).end('abc'); }, 40);
  });
  server.listen(0, '127.0.0.1'); await once(server, 'listening');
  const origin = `http://127.0.0.1:${server.address().port}`;
  let browser;
  try {
    browser = await chromium.launch({ headless: true });
    const page = await browser.newPage();
    await page.goto(origin);
    const result = await page.evaluate(async origin => {
      const { default: init, Client } = await import('/screenroom_client_core.js'); await init();
      const create = user_id => new Client({ endpoint: origin, user_id, timeout_ms: 1000 });
      const a = create('00000000-0000-0000-0000-000000000001');
      const url = n => `${origin}/artwork/00000000-0000-0000-0000-000000000001/revisions/${n}/native`;
      const [one, two] = await Promise.all([a.load_artwork(url(1)), a.load_artwork(url(1))]);
      await a.load_artwork(url(1));
      const b = create('00000000-0000-0000-0000-000000000002'); await b.load_artwork(url(1));
      const errors = [];
      for (const n of [2, 3, 4, 5, 6, 8]) {
        for (let attempt = 0; attempt < 2; attempt++) {
          try { await a.load_artwork(url(n)); errors.push('unexpected success'); } catch (e) { errors.push(e.code); }
        }
      }
      const pending = a.load_artwork(url(7)).then(() => 'unexpected success', e => e.code);
      await new Promise(resolve => setTimeout(resolve, 60)); a.cancel_all();
      return { one: [...one], two: [...two], errors, cancellation: await pending };
    }, origin);
    assert.deepEqual(result.one, [97, 98, 99]); assert.deepEqual(result.two, result.one);
    assert.deepEqual(result.errors, ['unavailable', 'unavailable', 'not_found', 'not_found', 'invalid_response', 'invalid_response', 'resource_exhausted', 'resource_exhausted', 'resource_exhausted', 'resource_exhausted', 'unavailable', 'unavailable']);
    assert.equal(result.cancellation, 'cancelled');
    await new Promise(resolve => setTimeout(resolve, 100));
    assert.equal(cancelled, true, 'dropping the final waiter must abort the browser fetch');
    assert.equal(counts.get('/redirect-target'), undefined, 'redirect must not be followed');
    for (const n of [1, 2, 3, 4, 5, 6, 8]) assert.equal(counts.get(`/artwork/00000000-0000-0000-0000-000000000001/revisions/${n}/native`), 2);
  } finally {
    if (browser) await browser.close();
    server.closeAllConnections(); await new Promise(resolve => server.close(resolve));
  }
});
