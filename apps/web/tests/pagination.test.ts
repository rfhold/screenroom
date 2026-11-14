import { test } from 'node:test';
import assert from 'node:assert/strict';
import { collectPages, IncrementalPages } from '../src/lib/core/pagination.ts';
import { MetadataOperation } from '../src/lib/core/operation.mjs';
import { OwnedReads } from '../src/lib/core/owned-reads.ts';

test('rejects cycles without returning partial entries', async () => {
  let calls = 0;
  await assert.rejects(collectPages(async () => ({ entries: [{ id: String(calls++) }], next_page_token: 'cycle' })), /repeated/);
  assert.equal(calls, 2);
});
test('bounds pages at 512', async () => {
  let calls = 0;
  await assert.rejects(collectPages(async () => ({ entries: [], next_page_token: String(++calls) })), /limit/);
  assert.equal(calls, 512);
});
test('bounds total entries even when IDs repeat', async () => {
  await assert.rejects(collectPages(async () => ({ entries: Array.from({ length: 10_001 }, () => ({ id: 'same' })), next_page_token: '' })), /item limit/);
});
test('bounds the whole collection duration', async t => {
  let now = 0;
  t.mock.method(performance, 'now', () => now);
  await assert.rejects(collectPages(async () => { now += 120_001; return { entries: [{ id: 'partial' }], next_page_token: '' }; }), /deadline/);
});
test('incremental pages single-flight, retry same token, stable dedupe and end', async () => {
  const tokens: string[] = [];
  let fail = true;
  const pages = new IncrementalPages(async page => {
    tokens.push(page.token);
    if (!page.token) return { entries: [{ id: 'a' }], next_page_token: 'next' };
    if (fail) { fail = false; throw new Error('offline'); }
    return { entries: [{ id: 'a' }, { id: 'b' }], next_page_token: '' };
  });
  const first = pages.load();
  assert.equal(pages.load(), first);
  await first;
  await assert.rejects(pages.load(), /offline/);
  assert.deepEqual(pages.entries, [{ id: 'a' }]);
  await pages.load(); await pages.load();
  assert.deepEqual(tokens, ['', 'next', 'next']);
  assert.deepEqual(pages.entries, [{ id: 'a' }, { id: 'b' }]);
  assert.equal(pages.more, false);
});
test('obsolete query cannot append or affect its replacement', async () => {
  let resolve!: (value: { entries: { id: string }[]; next_page_token: string }) => void;
  let current = true;
  const old = new IncrementalPages(() => new Promise(r => resolve = r), () => current);
  const pending = old.load();
  await Promise.resolve();
  current = false;
  resolve({ entries: [{ id: 'stale' }], next_page_token: 'next' });
  await assert.rejects(pending, /cancelled/);
  assert.deepEqual(old.entries, []);
});
test('next page gets a fresh budget after idle, initial page shares navigation budget', async t => {
  let now = 0;
  t.mock.method(performance, 'now', () => now);
  const pages = new IncrementalPages(async page => ({ entries: [{ id: page.token || 'a' }], next_page_token: page.token ? '' : 'next' }));
  await pages.load(new MetadataOperation());
  now = 300_000;
  await pages.load();
  assert.equal(pages.entries.length, 2);
  const expired = new MetadataOperation(); now += 120_001;
  await assert.rejects(new IncrementalPages(async () => ({ entries: [], next_page_token: '' })).load(expired), /deadline/);
});
test('incremental cursor cycles and item bounds stop further reads', async () => {
  const pages = new IncrementalPages(async () => ({ entries: [{ id: 'a' }], next_page_token: 'cycle' }));
  await pages.load();
  await assert.rejects(pages.load(), /cycle/);
  assert.equal(pages.more, false);
  assert.deepEqual(pages.entries, [{ id: 'a' }]);
  const huge = new IncrementalPages(async () => ({ entries: Array.from({ length: 10_001 }, () => ({ id: 'a' })), next_page_token: '' }));
  await assert.rejects(huge.load(), /limit/);
});
test('incremental pages bound empty-page chains', async () => {
  let calls = 0;
  const pages = new IncrementalPages(async () => ({ entries: [], next_page_token: String(++calls) }));
  for (let index = 0; index < 511; index++) await pages.load();
  await assert.rejects(pages.load(), /limit/);
  await pages.load();
  assert.equal(calls, 512);
});
test('owned cancellation waits for borrowed WASM reads before freeing', async () => {
  const events: string[] = [];
  let settle!: () => void;
  const owner = new OwnedReads({ cancel_all: () => events.push('cancel'), free: () => { events.push('free'); } });
  const read = owner.read(() => new Promise<void>(resolve => settle = resolve));
  owner.dispose(); owner.dispose();
  assert.deepEqual(events, ['cancel']);
  await assert.rejects(owner.read(async () => {}), /disposed/);
  settle(); await read; await Promise.resolve();
  assert.deepEqual(events, ['cancel', 'free']);
});
