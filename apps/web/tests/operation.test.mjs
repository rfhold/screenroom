import { test } from 'node:test';
import assert from 'node:assert/strict';
import { MetadataOperation, MetadataCancelled } from '../src/lib/core/operation.mjs';
import { collectPages } from '../src/lib/core/pagination.ts';

const deferred = () => {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};

test('consecutive 70-second metadata stages share one monotonic 120-second deadline', async t => {
  let now = 0;
  t.mock.method(performance, 'now', () => now);
  const operation = new MetadataOperation();
  let reads = 0;
  const page = async () => { reads++; now += 70_000; return { entries: [{ id: 'record' }], next_page_token: '' }; };
  await collectPages(page, undefined, undefined, operation);
  await assert.rejects(collectPages(page, undefined, undefined, operation), /deadline/);
  await assert.rejects(operation.read(() => { reads++; }), /deadline/);
  assert.equal(reads, 2);
  assert.equal(operation.waiting.size, 0);
});

test('final single-response await expires without late publication, commit, or subsequent reads', async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  let now = 0;
  const operation = new MetadataOperation(() => true, { now: () => now });
  const final = deferred();
  let publications = 0, commits = 0, reads = 0;
  await operation.read(() => { now = 70_000; });
  const completion = (async () => {
    await operation.read(() => { reads++; return final.promise; });
    operation.check(); commits++; publications++;
    await operation.read(() => { reads++; });
  })();
  const rejected = assert.rejects(completion, /deadline/);
  await Promise.resolve();
  now = 120_000;
  t.mock.timers.tick(50_000);
  await rejected;
  final.resolve('too late');
  await Promise.resolve(); await Promise.resolve();
  assert.deepEqual({ publications, commits, reads }, { publications: 0, commits: 0, reads: 1 });
  assert.equal(operation.waiting.size, 0);
});

test('expiry aborts sibling pagination follow-ups and handles unused late rejections', async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  let now = 0;
  const operation = new MetadataOperation(() => true, { now: () => now });
  const first = deferred(), unused = deferred();
  let reads = 0;
  const pages = collectPages(async () => { reads++; return first.promise; }, undefined, undefined, operation);
  const other = operation.read(() => unused.promise);
  const rejected = assert.rejects(Promise.all([pages, other]), /deadline/);
  await Promise.resolve();
  now = 120_000; t.mock.timers.tick(120_000);
  await rejected;
  first.resolve({ entries: [{ id: 'late' }], next_page_token: 'never-read' });
  unused.reject(new Error('Late unused RPC failure'));
  await Promise.resolve(); await Promise.resolve();
  assert.equal(reads, 1);
  assert.equal(operation.waiting.size, 0);
});

test('navigation cancellation releases old waiters without cancelling a newer operation', async () => {
  let current = 1;
  const old = new MetadataOperation(() => current === 1);
  const pending = deferred();
  const completion = old.read(() => pending.promise);
  const rejected = assert.rejects(completion, MetadataCancelled);
  await Promise.resolve();
  current = 2; old.cancel();
  const newer = new MetadataOperation(() => current === 2);
  assert.equal(await newer.read(() => 'new navigation'), 'new navigation');
  await rejected;
  pending.resolve('old navigation');
  assert.equal(await newer.read(() => 'still healthy'), 'still healthy');
  assert.equal(old.waiting.size, 0);
});

test('expiry before initialization prevents the first request and queued durable commit', async t => {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  let now = 0;
  const operation = new MetadataOperation(() => true, { now: () => now });
  const init = deferred();
  let reads = 0, commits = 0;
  const completion = (async () => {
    await operation.read(() => init.promise);
    await operation.read(() => { reads++; });
    operation.check(); commits++;
  })();
  const rejected = assert.rejects(completion, /deadline/);
  await Promise.resolve();
  now = 120_000; t.mock.timers.tick(120_000);
  await rejected;
  init.resolve();
  await Promise.resolve(); await Promise.resolve();
  assert.deepEqual({ reads, commits }, { reads: 0, commits: 0 });
  assert.equal(operation.waiting.size, 0);
});
