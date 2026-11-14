import test from 'node:test';
import assert from 'node:assert/strict';
import { changeJellyfinLink, progressError } from '../src/lib/core/jellyfin.mjs';
import { MetadataOperation } from '../src/lib/core/operation.mjs';

function mock({ failure = false, readFailure = false, actual = 'profile', returned = 'profile' } = {}) {
  const calls = [];
  const value = id => id === null ? null : { source_id: 'source', jellyfin_user_id: id };
  return { calls, client: {
    async set_jellyfin_link(source, profile) { calls.push(['set', source, profile]); if (failure) throw new Error('transport'); return value(returned); },
    async clear_jellyfin_link(source) { calls.push(['clear', source]); if (failure) throw new Error('transport'); return source; },
    invalidate_reads() { calls.push(['invalidate']); },
    async get_jellyfin_link(source) { calls.push(['get', source]); if (readFailure) throw new Error('transport'); return value(actual); },
  } };
}

test('set verifies returned mapping and a fresh server read', async () => {
  const { client, calls } = mock();
  assert.deepEqual(await changeJellyfinLink(client, 'source', 'profile', new MetadataOperation()), {
    link: { source_id: 'source', jellyfin_user_id: 'profile' }, confirmed: true, ambiguous: false,
  });
  assert.deepEqual(calls, [['set', 'source', 'profile'], ['invalidate'], ['get', 'source']]);
});
test('ambiguous mutation reads truth without repeating the write', async () => {
  const { client, calls } = mock({ failure: true });
  const result = await changeJellyfinLink(client, 'source', 'profile', new MetadataOperation());
  assert.equal(result.confirmed, true);
  assert.equal(result.ambiguous, true);
  assert.equal(calls.filter(call => call[0] === 'set').length, 1);
  assert.equal(calls.at(-1)[0], 'get');
});
test('unconfirmed or inconsistent mappings are not success', async () => {
  for (const options of [{ actual: 'other' }, { returned: 'other' }, { failure: true, actual: 'other' }]) {
    const { client } = mock(options);
    assert.equal((await changeJellyfinLink(client, 'source', 'profile', new MetadataOperation())).confirmed, false);
  }
});
test('unlink confirms absent mapping, including an ambiguous response', async () => {
  for (const failure of [false, true]) {
    const { client, calls } = mock({ actual: null, failure });
    const result = await changeJellyfinLink(client, 'source', null, new MetadataOperation());
    assert.equal(result.confirmed, true);
    assert.equal(result.link, null);
    assert.equal(calls.filter(call => call[0] === 'clear').length, 1);
  }
});
test('failed reconciliation cannot claim success or retry mutation', async () => {
  const { client, calls } = mock({ failure: true, readFailure: true });
  await assert.rejects(changeJellyfinLink(client, 'source', 'profile', new MetadataOperation()));
  assert.equal(calls.filter(call => call[0] === 'set').length, 1);
});
test('cancelled operations do not start writes', async () => {
  const { client, calls } = mock();
  const op = new MetadataOperation(); op.cancel();
  await assert.rejects(changeJellyfinLink(client, 'source', 'profile', op));
  assert.deepEqual(calls, []);
});
test('row errors use sanitized text, never backend messages', () => {
  const text = progressError({ code: 'progress_unavailable', message: 'secret http://private/token' });
  assert.match(text, /Continue Watching is unavailable/);
  assert.doesNotMatch(text, /secret|private|token/);
});
test('mutation and reconciliation share one 120-second operation budget', async () => {
  let now = 0;
  const { client } = mock();
  client.set_jellyfin_link = async () => { now += 70_000; return { source_id: 'source', jellyfin_user_id: 'profile' }; };
  client.get_jellyfin_link = async () => { now += 70_000; return { source_id: 'source', jellyfin_user_id: 'profile' }; };
  await assert.rejects(changeJellyfinLink(client, 'source', 'profile', new MetadataOperation(() => true, { now: () => now })), /deadline exceeded/);
});
test('cancelling an in-flight mutation prevents follow-up reads and publication', async () => {
  const { client, calls } = mock();
  let complete;
  client.set_jellyfin_link = () => new Promise(resolve => { complete = resolve; });
  const op = new MetadataOperation();
  const pending = changeJellyfinLink(client, 'source', 'profile', op);
  await Promise.resolve();
  op.cancel();
  await assert.rejects(pending, /cancelled/);
  complete({ source_id: 'source', jellyfin_user_id: 'profile' });
  await Promise.resolve();
  assert.deepEqual(calls, []);
});
