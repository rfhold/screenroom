import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
const require = createRequire(import.meta.url);
const { build } = createRequire(require.resolve('vite'))('esbuild');
const root = fileURLToPath(new URL('..', import.meta.url));
const bundle = await build({ entryPoints: [`${root}/src/lib/core/playback.ts`], bundle: true, write: false, format: 'esm', platform: 'node' });
const { Playback, seconds, position, mediaURL } = await import(`data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].text).toString('base64')}`);
const id = '11111111-1111-4111-8111-111111111111';
const caps = { mp4: true, h264: true, aac: true, native_hls: false, mse_hls: false, webvtt: true };
const context = { source_id: 'source', library_id: 'library' };
function setup(overrides = {}) {
  let session = { session_id: id, control_capability: 'a'.repeat(64), transport_revision: 1, last_accepted_sequence: 0, state: 'ready', report_outcome: 'not_sent', pending_operation: null, tracks: [], failure_code: null,
    transport: { media_url: `http://localhost:4000/playback/v1/media/${id}/1/${id}?cap=${'b'.repeat(64)}`, revision: 1, timeline_offset_ms: '1000', start_position_ms: '5000', duration_ms: '100000', mime_type: 'video/mp4' } };
  const calls = [], attachments = [];
  let detached = 0, invalidations = 0, personal = 0;
  const client = {
    async create_playback_session(body) { calls.push(['create', structuredClone(body)]); return { session: structuredClone(session) }; },
    async get_playback_session(body) { calls.push(['get', body]); return { session: structuredClone(session) }; },
    async report_playback_event(body) { calls.push(['report', body]); session.last_accepted_sequence = body.sequence; session.report_outcome = 'confirmed'; return { session: structuredClone(session), report_outcome: 'confirmed', accepted_sequence: body.sequence }; },
    async update_playback_session(body) { calls.push(['update', structuredClone(body)]); session.transport_revision++; session.transport.revision++; return { session: structuredClone(session) }; },
    async stop_playback_session(body) { calls.push(['stop', structuredClone(body)]); return { session: { ...session, state: 'stopped', transport: null, report_outcome: 'confirmed' } }; },
    invalidate_reads() { invalidations++; }, cancel_all() { calls.push(['cancel']); }, ...overrides,
  };
  const media = {
    currentTime: 4, paused: false, muted: false, volume: 1,
    detach() { detached++; this.currentTime = 0; this.paused = true; },
    async attach(transport, _session, restore) {
      attachments.push(transport);
      this.currentTime = seconds(restore?.position_ms ?? transport.start_position_ms) - seconds(transport.timeline_offset_ms);
      this.paused = restore?.paused ?? false;
    },
  };
  const player = new Playback(client, media, () => {}, () => { personal++; });
  return { player, client, media, calls, attachments, get detached() { return detached; }, get invalidations() { return invalidations; }, get personal() { return personal; }, session };
}
test('timeline precision, one offset conversion and safe media origin/control separation', () => {
  assert.equal(seconds('922337203685477'), 922337203685.477);
  assert.equal(position(4.125, '1000'), '5125');
  for (const value of ['9007199254740993', '-1', '1e3', '922337203685478']) assert.throws(() => seconds(value));
  assert.throws(() => position(Infinity, '0'));
  const { session } = setup();
  const url = session.transport.media_url;
  assert.equal(mediaURL(url, 'http://localhost:4000', session), url);
  for (const bad of [url.replace(':4000', ':4001'), url.replace('b'.repeat(64), 'a'.repeat(64)), `${url}&key=secret`, url.replace('/1/', '/2/')]) assert.throws(() => mediaURL(bad, 'http://localhost:4000', session));
});
test('exact explicit creation retry preserves UUID/body and never replays automatically', async () => {
  const bodies = [];
  const state = setup({ async create_playback_session(body) { bodies.push(structuredClone(body)); throw { code: 'playback_uncertain' }; } });
  await state.player.create('movie', context, caps);
  assert.equal(bodies.length, 1);
  await state.player.retryCreate();
  assert.deepEqual(bodies[0], bodies[1]);
  assert.deepEqual(bodies[0].scope, context);
  assert.equal(state.player.session, null);
  assert.match(state.player.status, /not confirmed/);
  state.player.exit();
});
test('start once, serialized monotonic sequences allow backwards seek and heartbeat', async t => {
  t.mock.timers.enable({ apis: ['setInterval'] });
  const state = setup();
  await state.player.create('episode', context, caps);
  await Promise.all([state.player.event('started'), state.player.event('started')]);
  state.media.currentTime = 20; await state.player.event('progress');
  state.media.currentTime = 1; await state.player.event('seeked');
  t.mock.timers.tick(15_000); await state.player.settled();
  const reports = state.calls.filter(([kind]) => kind === 'report').map(([, body]) => body);
  assert.deepEqual(reports.map(r => r.kind), ['started', 'progress', 'seeked', 'heartbeat']);
  assert.deepEqual(reports.map(r => r.sequence), [1, 2, 3, 4]);
  assert.equal(reports[2].position_ms, '2000');
  assert.equal(state.personal, 4);
  state.player.exit(); await state.player.settled();
  assert.equal(state.calls.filter(([kind]) => kind === 'stop').length, 1);
});
test('track/fallback transport rotation destroys media and never starts again', async () => {
  const state = setup();
  await state.player.create('movie', context, caps);
  await state.player.event('started');
  const detached = state.detached;
  await state.player.update({ subtitle_track_id: id, audio_track_id: null, subtitles_off: false });
  await state.player.update(null, true);
  await state.player.update(null, true);
  await state.player.event('started');
  assert.equal(state.calls.filter(([kind]) => kind === 'update').length, 2);
  assert.ok(state.detached > detached);
  assert.equal(state.attachments.length, 3);
  assert.equal(state.calls.filter(([kind, body]) => kind === 'report' && body.kind === 'started').length, 1);
  state.player.exit(); await state.player.settled();
});
test('uncertain start is read-reconciled once then terminated without later personal reports', async () => {
  let reports = 0;
  const state = setup({ async report_playback_event() { reports++; throw { code: 'playback_uncertain' }; } });
  await state.player.create('movie', context, caps);
  await state.player.event('started');
  await state.player.check();
  await state.player.event('progress');
  assert.equal(reports, 1);
  assert.equal(state.calls.filter(([kind]) => kind === 'get').length, 1);
  assert.equal(state.calls.filter(([kind]) => kind === 'stop').length, 1);
  assert.equal(state.player.blocked, true);
  assert.equal(state.player.closed, true);
  state.player.exit(); await state.player.settled();
  assert.equal(state.calls.filter(([kind]) => kind === 'stop').length, 1);
});
test('exit immediately removes media, ignores queued reports and stops late create authority', async () => {
  let resolve;
  const state = setup({ create_playback_session() { return new Promise(done => { resolve = done; }); } });
  const pending = state.player.create('movie', context, caps);
  await Promise.resolve();
  state.player.exit('profile_changed');
  assert.ok(state.detached > 0);
  assert.equal(state.player.closed, true);
  resolve({ session: state.session });
  await pending; await state.player.settled();
  assert.equal(state.attachments.length, 0);
  const stops = state.calls.filter(([kind]) => kind === 'stop');
  assert.equal(stops.length, 1);
  assert.ok(stops.every(([, body]) => JSON.stringify(body) === JSON.stringify(stops[0][1])));
});
test('transport uncertainty reads once and an explicit retry retains the exact UUID/body', async () => {
  const state = setup(), bodies = [];
  const update = state.client.update_playback_session;
  state.client.update_playback_session = async body => {
    bodies.push(structuredClone(body));
    if (bodies.length === 1) throw { code: 'playback_uncertain' };
    return update(body);
  };
  await state.player.create('movie', context, caps);
  await state.player.update({ audio_track_id: id, subtitle_track_id: null, subtitles_off: false });
  assert.equal(bodies.length, 1);
  assert.equal(state.calls.filter(([kind]) => kind === 'get').length, 1);
  await state.player.retryUpdate();
  assert.deepEqual(bodies[0], bodies[1]);
  state.player.exit(); await state.player.settled();
});
test('start during media attachment is serialized, stale transport receipt is rejected', async () => {
  const state = setup();
  state.media.attach = async () => { void state.player.event('started'); };
  await state.player.create('episode', context, caps); await state.player.settled();
  assert.equal(state.calls.filter(([kind, body]) => kind === 'report' && body.kind === 'started').length, 1);
  state.client.get_playback_session = async () => ({ session: { ...state.session, transport_revision: 0 } });
  await state.player.check();
  assert.equal(state.player.blocked, true);
  assert.match(state.player.status, /could not be confirmed/);
  state.player.exit(); await state.player.settled();
});
test('lost accepted Start response retains the final logical position after detach', async () => {
  const state = setup();
  state.session.transport.timeline_offset_ms = '0';
  state.session.transport.start_position_ms = '10000';
  state.client.report_playback_event = async body => {
    state.calls.push(['report', body]);
    state.session.last_accepted_sequence = body.sequence;
    state.session.report_outcome = 'confirmed';
    throw { code: 'playback_uncertain' };
  };
  await state.player.create('movie', context, caps);
  state.media.currentTime = 120;
  await state.player.event('started');
  assert.equal(state.media.currentTime, 0);
  assert.equal(state.calls.filter(([kind]) => kind === 'get').length, 1);
  const stops = state.calls.filter(([kind]) => kind === 'stop');
  assert.equal(stops.length, 1);
  assert.equal(stops[0][1].position_ms, '120000');
  assert.equal(state.player.closed, true);
});
test('exit capture wins over a late Update with a different timeline offset', async () => {
  const state = setup();
  state.session.transport.timeline_offset_ms = '5000';
  state.session.transport.start_position_ms = '10000';
  let resolve;
  state.client.update_playback_session = () => new Promise(done => { resolve = done; });
  await state.player.create('movie', context, caps);
  state.media.currentTime = 115;
  const pending = state.player.update(null);
  await Promise.resolve();
  assert.equal(state.media.currentTime, 0);
  state.player.exit();
  resolve({ session: { ...state.session, transport_revision: 2,
    transport: { ...state.session.transport, revision: 2, start_position_ms: '120000', timeline_offset_ms: '115000' } } });
  await pending; await state.player.settled();
  const stops = state.calls.filter(([kind]) => kind === 'stop');
  assert.equal(stops.length, 1);
  assert.equal(stops[0][1].position_ms, '120000');
  assert.equal(stops[0][1].transport_revision, 2);
  assert.equal(state.attachments.length, 1);
});
for (const paused of [true, false]) test(`same-revision Check preserves 120s and paused=${paused}`, async () => {
  const state = setup();
  state.session.transport.timeline_offset_ms = '0';
  state.session.transport.start_position_ms = '10000';
  await state.player.create('movie', context, caps);
  state.media.currentTime = 120;
  await state.player.event('started');
  state.media.paused = paused;
  const detached = state.detached;
  await state.player.check();
  assert.equal(state.media.currentTime, 120);
  assert.equal(state.media.paused, paused);
  assert.equal(state.detached, detached);
  assert.equal(state.attachments.length, 1);
  assert.equal(state.calls.filter(([kind, body]) => kind === 'report' && body.kind === 'started').length, 1);
  state.player.exit(); await state.player.settled();
  assert.equal(state.calls.find(([kind]) => kind === 'stop')[1].position_ms, '120000');
});
test('uncertain Update recovery restores the old transport at retained position and pause', async () => {
  const state = setup();
  state.session.transport.timeline_offset_ms = '5000';
  state.session.transport.start_position_ms = '10000';
  state.client.update_playback_session = async () => { throw { code: 'playback_uncertain' }; };
  await state.player.create('movie', context, caps);
  state.media.currentTime = 115; state.media.paused = true;
  await state.player.update(null);
  assert.equal(state.attachments.length, 2);
  assert.equal(state.media.currentTime, 115);
  assert.equal(state.media.paused, true);
  state.player.exit(); await state.player.settled();
  assert.equal(state.calls.find(([kind]) => kind === 'stop')[1].position_ms, '120000');
});
test('a genuinely new revision attaches its descriptor timeline and retains pause', async () => {
  const state = setup();
  state.session.transport.timeline_offset_ms = '5000';
  state.session.transport.start_position_ms = '10000';
  await state.player.create('movie', context, caps);
  state.media.currentTime = 115; state.media.paused = true;
  state.client.update_playback_session = async () => ({ session: { ...state.session, transport_revision: 2,
    transport: { ...state.session.transport, revision: 2, start_position_ms: '120000', timeline_offset_ms: '115000' } } });
  await state.player.update(null);
  assert.equal(state.attachments.length, 2);
  assert.equal(state.media.currentTime, 5);
  assert.equal(state.media.paused, true);
  state.player.exit(); await state.player.settled();
  assert.equal(state.calls.find(([kind]) => kind === 'stop')[1].position_ms, '120000');
});
