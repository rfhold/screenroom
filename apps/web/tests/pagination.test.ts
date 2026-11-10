import { test } from 'node:test';
import assert from 'node:assert/strict';
import { collectPages } from '../src/lib/core/pagination.ts';

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
