import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createArtworkLoader } from '../src/lib/core/artwork.ts';

function deferred() {
  let resolve!: (bytes: Uint8Array) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<Uint8Array>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve: () => resolve(new Uint8Array([0x89])), reject };
}

test('all 80 unique artworks finish with at most eight core loads in flight', async () => {
  let running = 0;
  let peak = 0;
  const calls: string[] = [];
  const acquire = createArtworkLoader(async url => {
    calls.push(url);
    peak = Math.max(peak, ++running);
    await new Promise<void>(resolve => setImmediate(resolve));
    running--;
    return new Uint8Array([0x89]);
  });
  const images = Array.from({ length: 80 }, (_, i) => acquire(`image-${i}`));
  const urls = await Promise.all(images.map(image => image.loaded));
  assert.equal(peak, 8);
  assert.equal(new Set(calls).size, 80);
  assert.equal(new Set(urls).size, 80);
  assert.ok(urls.every(url => url.startsWith('blob:')));
  images.forEach(image => image.release());
});

test('last queued release skips IO and does not affect a replacement owner', async () => {
  const gate = deferred();
  const calls: string[] = [];
  const acquire = createArtworkLoader(url => { calls.push(url); return gate.promise; });
  const busy = Array.from({ length: 8 }, (_, i) => acquire(`busy-${i}`));
  const removed = acquire('removed');
  removed.release(); removed.release();
  const old = acquire('replacement');
  old.release();
  const replacement = acquire('replacement');
  old.release();
  gate.resolve();
  await Promise.all([...busy, removed, old, replacement].map(image => image.loaded));
  assert.ok(!calls.includes('removed'));
  assert.equal(calls.filter(url => url === 'replacement').length, 1);
  assert.ok((await replacement.loaded).startsWith('blob:'));
  [...busy, replacement].forEach(image => image.release());
});

test('failed and synchronously throwing loads free slots without retries', async () => {
  const calls: string[] = [];
  const acquire = createArtworkLoader(url => {
    calls.push(url);
    if (url === '0') throw new Error('sync failure');
    if (Number(url) < 12) return Promise.reject(new Error('offline'));
    return Promise.resolve(new Uint8Array([0xff]));
  });
  const images = Array.from({ length: 48 }, (_, i) => acquire(String(i)));
  const results = await Promise.allSettled(images.map(image => image.loaded));
  assert.equal(results.filter(result => result.status === 'rejected').length, 12);
  assert.equal(results.filter(result => result.status === 'fulfilled').length, 36);
  assert.equal(calls.length, 48);
  images.forEach(image => image.release());
});

test('duplicates and in-flight reacquisition retain ownership until the last release', async t => {
  const gate = deferred();
  let calls = 0;
  const revoked: string[] = [];
  const revoke = URL.revokeObjectURL.bind(URL);
  t.mock.method(URL, 'revokeObjectURL', (url: string) => { revoked.push(url); revoke(url); });
  const acquire = createArtworkLoader(() => { calls++; return gate.promise; });
  const first = acquire('same');
  const duplicate = acquire('same');
  assert.equal(first.loaded, duplicate.loaded);
  first.release(); first.release();
  duplicate.release();
  const renewed = acquire('same');
  assert.equal(renewed.loaded, first.loaded);
  gate.resolve();
  const url = await renewed.loaded;
  assert.equal(calls, 1);
  assert.deepEqual(revoked, []);
  const cached = acquire('same');
  assert.equal(await cached.loaded, url);
  renewed.release(); renewed.release(); duplicate.release();
  assert.deepEqual(revoked, []);
  cached.release(); cached.release();
  assert.deepEqual(revoked, [url]);
  const fresh = acquire('same');
  assert.notEqual(await fresh.loaded, url);
  assert.equal(calls, 2);
  fresh.release();
  assert.equal(revoked.length, 2);
});

test('abandoned in-flight loads create no blob and can be acquired afresh after settling', async t => {
  const gate = deferred();
  let calls = 0;
  const create = t.mock.method(URL, 'createObjectURL', () => 'blob:test');
  const revoke = t.mock.method(URL, 'revokeObjectURL', () => {});
  const acquire = createArtworkLoader(() => { calls++; return gate.promise; });
  const abandoned = acquire('same');
  abandoned.release();
  gate.resolve();
  assert.equal(await abandoned.loaded, '');
  assert.equal(create.mock.callCount(), 0);
  const fresh = acquire('same');
  assert.equal(await fresh.loaded, 'blob:test');
  abandoned.release();
  assert.equal(revoke.mock.callCount(), 0);
  fresh.release();
  assert.equal(calls, 2);
  assert.equal(revoke.mock.callCount(), 1);
});
