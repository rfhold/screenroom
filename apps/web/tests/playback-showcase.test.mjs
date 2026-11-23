import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, readFile, rm, cp, readdir, stat, utimes } from 'node:fs/promises';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { allowedPlaybackRequest, measuredViewport, mergeViewport, playbackSelection } from '../e2e/playback-contract.mjs';
import { trackReadiness } from '../e2e/playback-tracks.mjs';
import { muteSettled, volumeSettled } from '../e2e/playback-controls.mjs';

const endpoint = 'http://127.0.0.1:50052';
const selection = { instanceID: '11111111-2222-4333-8444-555555555555', item: { id: 'explicit-item', title: 'Nosferatu', year: 1922 } };
const state = profile => ({ id: `${profile}-playing`, label: 'Playing', status: 'captured', viewport: measuredViewport(profile === 'desktop' ? { width: 1440, height: 900 } : { width: 390, height: 844 }), image: `${profile}-playing.png`, observation: 'Actual media and UI before and after.' });
const settledTrack = { error: false, uncertain: false, mediaReady: true, controlsEnabled: true, selectorEnabled: true, requestedSelected: true, transportChanged: true, updateResponded: true };
test('mute capture waits for actual media and the rendered label in the same observation', () => {
  for (const muted of [true, false]) {
    const label = muted ? 'Unmute' : 'Mute';
    assert.equal(muteSettled({ media: { muted }, muteControl: label }, muted), true);
    assert.equal(muteSettled({ media: { muted }, muteControl: muted ? 'Mute' : 'Unmute' }, muted), false);
    assert.equal(muteSettled({ media: { muted: !muted }, muteControl: label }, muted), false);
    assert.equal(muteSettled({ media: null, muteControl: label }, muted), false);
  }
});
test('volume capture waits for the visible slider and unmuted media to settle together', () => {
  const state = { media: { volume: 0.5, muted: false }, volumeValues: [0.5] };
  assert.equal(volumeSettled(state, 0.5), true);
  assert.equal(volumeSettled({ ...state, volumeValues: [0] }, 0.5), false);
  assert.equal(volumeSettled({ ...state, media: { volume: 0, muted: false } }, 0.5), false);
  assert.equal(volumeSettled({ ...state, media: { volume: 0.5, muted: true } }, 0.5), false);
  assert.equal(volumeSettled({ ...state, volumeValues: [] }, 0.5), false);
  assert.equal(volumeSettled({ ...state, media: null }, 0.5), false);
});
test('changed track requires settled new transport, not an immediate requested select value', () => {
  assert.equal(trackReadiness({ ...settledTrack, transportChanged: false }, false), 'waiting');
  assert.equal(trackReadiness({ ...settledTrack, updateResponded: false }, false), 'waiting');
  assert.equal(trackReadiness(settledTrack, false), 'ready');
});
test('current selected track can be captured without any redundant update', () => {
  assert.equal(trackReadiness({ ...settledTrack, transportChanged: false, updateResponded: false }, true), 'ready');
  assert.equal(trackReadiness({ ...settledTrack, requestedSelected: false }, true), 'waiting');
});
test('track readiness rejects busy, detached media, disabled selectors and natural uncertainty', () => {
  for (const field of ['mediaReady', 'controlsEnabled', 'selectorEnabled', 'requestedSelected']) {
    assert.equal(trackReadiness({ ...settledTrack, [field]: false }, false), 'waiting');
    assert.equal(trackReadiness({ ...settledTrack, [field]: false }, true), 'waiting');
  }
  assert.equal(trackReadiness({ ...settledTrack, error: true }, false), 'blocked');
  assert.equal(trackReadiness({ ...settledTrack, uncertain: true }, true), 'blocked');
});
test('every enumerated option gets a distinct selected configuration or explicit gap', async () => {
  const source = await readFile(new URL('../e2e/playback.spec.ts', import.meta.url), 'utf8');
  assert.match(source, /for \(const option of options\)/);
  assert.match(source, /if \(!option\.available\)/);
  assert.match(source, /if \(!currentSelection\)/);
  assert.match(source, /select\.selectOption\(\{ value: option\.value \}\)/);
  assert.match(source, /JSON\.stringify\(publicOptions\)/);
  assert.doesNotMatch(source, /JSON\.stringify\(options\)/);
  assert.match(source, /Already selected option must not issue a redundant transport update/);
  assert.match(source, /await transport\.dispose\(\)/);
  assert.doesNotMatch(source, /selectOption\(''\)/);
});
test('combined fresh manifest has unique viewport IDs and no diagnostic inputs', () => {
  const manifest = mergeViewport(mergeViewport(null, selection, 'desktop', [state('desktop')], false), selection, 'mobile', [state('mobile')], true);
  assert.equal(manifest.platforms.length, 1);
  assert.equal(manifest.platforms[0].outcome, 'blocked');
  assert.deepEqual(manifest.platforms[0].states.map(row => row.id), ['desktop-playing', 'mobile-playing']);
  assert.throws(() => mergeViewport(manifest, selection, 'mobile', [state('mobile')], false), /Duplicate/);
  assert.throws(() => mergeViewport(manifest, { ...selection, instanceID: 'foreign' }, 'mobile', [], false), /Foreign/);
  assert.throws(() => mergeViewport(null, selection, 'desktop', [{ ...state('desktop'), image: '../../audit.png' }], false), /Invalid/);
  assert.throws(() => mergeViewport(null, selection, 'desktop', [{ ...state('desktop'), observation: '' }], false), /Invalid/);
  assert.throws(() => mergeViewport(null, selection, 'desktop', [{ ...state('desktop'), viewport: { width: 1440, height: 900 } }], false), /Invalid/);
  assert.equal(manifest.platforms[0].states[0].viewport, '1440x900 CSS pixels');
  assert.equal(manifest.platforms[0].states[1].viewport, '390x844 CSS pixels');
  assert.ok(!JSON.stringify(manifest).includes('player-audit'));
});
test('request guard permits only actual public playback gateway and known reads', () => {
  const origin = 'http://127.0.0.1:4176';
  const allowed = path => allowedPlaybackRequest(`${endpoint}${path}`, origin, endpoint);
  assert.ok(allowed('/screenroom.playback_gateway.v1.PlaybackService/ReportPlaybackEvent'));
  assert.ok(allowed('/screenroom.catalog.v1.CatalogService/GetItem'));
  assert.ok(allowed(`/playback/v1/media/${selection.instanceID}/1/${selection.instanceID}?cap=${'a'.repeat(64)}`));
  for (const path of ['/Items/private', '/screenroom.user.v1.UserService/SetJellyfinLink', '/screenroom.playback_gateway.v1.PlaybackService/Anything', `/playback/v1/media/${selection.instanceID}/1/${selection.instanceID}?cap=bad`]) assert.equal(allowed(path), false);
  assert.equal(allowedPlaybackRequest('http://jellyfin:8096/video', origin, endpoint), false);
  assert.equal(allowedPlaybackRequest(`${origin}/__operator/secrets`, origin, endpoint), false);
});
test('media request guard accepts only root capabilities or complete signed child queries', () => {
  const origin = 'http://127.0.0.1:4176';
  const media = `${endpoint}/playback/v1/media/${selection.instanceID}/1/${selection.instanceID}`;
  const cap = 'a'.repeat(64);
  const sig = 'b'.repeat(64);
  const anchor = selection.instanceID;
  const allowed = query => allowedPlaybackRequest(`${media}?${query}`, origin, endpoint);
  assert.equal(allowed(`cap=${cap}`), true);
  for (const query of [
    `cap=${cap}&anchor=${anchor}&sig=${sig}`,
    `sig=${sig}&cap=${cap}&anchor=${anchor}`,
    `anchor=${anchor}&sig=${sig}&cap=${cap}`,
  ]) assert.equal(allowed(query), true);
  for (const query of [
    '',
    `cap=${cap}&anchor=${anchor}`,
    `cap=${cap}&sig=${sig}`,
    `anchor=${anchor}&sig=${sig}`,
    `cap=${cap}&cap=${cap}`,
    `cap=${cap}&anchor=${anchor}&sig=${sig}&cap=${cap}`,
    `cap=${cap}&anchor=${anchor}&sig=${sig}&anchor=${anchor}`,
    `cap=${cap}&anchor=${anchor}&sig=${sig}&sig=${sig}`,
    `cap=${cap}&anchor=${anchor}&sig=${sig}&url=private`,
    `cap=${cap}&anchor=${anchor}&sig=${sig}&`,
    `cap=${cap}&anchor=${anchor}&sig=`,
    `cap=${cap}&anchor=&sig=${sig}`,
    `cap=${cap}&anchor=${anchor}&sig=${'B'.repeat(64)}`,
    `cap=${cap}&anchor=${anchor}&sig=${'b'.repeat(63)}`,
    `cap=${'g'.repeat(64)}&anchor=${anchor}&sig=${sig}`,
    `cap=${cap}&anchor=${'-'.repeat(36)}&sig=${sig}`,
    `cap=${cap}&anchor=00000000-0000-0000-0000-000000000000&sig=${sig}`,
    `cap=${cap}&anchor=${anchor.slice(1)}&sig=${sig}`,
    `%63ap=${cap}&anchor=${anchor}&sig=${sig}`,
    `cap=${cap}&%61nchor=${anchor}&sig=${sig}`,
    `cap=${cap}&anchor=${anchor}&%73ig=${sig}`,
    `cap=%61${cap.slice(1)}&anchor=${anchor}&sig=${sig}`,
    `cap=${cap}&anchor=%31${anchor.slice(1)}&sig=${sig}`,
    `cap=${cap}&anchor=${anchor}&sig=%62${sig.slice(1)}`,
    `cap=${cap}&anchor=${anchor}&sig=${sig}=extra`,
  ]) assert.equal(allowed(query), false);
});
test('signed child query shape cannot broaden approved media origins or paths', () => {
  const origin = 'http://127.0.0.1:4176';
  const path = `/playback/v1/media/${selection.instanceID}/1/${selection.instanceID}`;
  const query = `?cap=${'a'.repeat(64)}&anchor=${selection.instanceID}&sig=${'b'.repeat(64)}`;
  for (const value of [
    `http://127.0.0.1:50051${path}${query}`,
    `http://jellyfin:8096${path}${query}`,
    `https://127.0.0.1:50052${path}${query}`,
    `http://foreign.example${path}${query}`,
    `http://user:password@127.0.0.1:50052${path}${query}`,
    `${endpoint}${path}${query}#fragment`,
    `${endpoint}${path}/extra${query}`,
    `${endpoint}/Items/private${query}`,
    `${endpoint}/__operator/secrets${query}`,
  ]) assert.equal(allowedPlaybackRequest(value, origin, endpoint), false);
});
test('playback validation rejects missing explicit attachment and selectors', () => {
  assert.throws(() => playbackSelection({}), /explicit attachment/);
  const env = { SCREENROOM_BACKEND_ATTACH: '1', SCREENROOM_BACKEND_PROJECT: 'screenroom-jellyfin-local' };
  assert.throws(() => playbackSelection(env), /50052/);
  assert.throws(() => playbackSelection({ ...env, SCREENROOM_E2E_API_ENDPOINT: endpoint }), /explicit SCREENROOM_SHOWCASE/);
});
test('manifest identity comes from explicit root instance selector, verified against descriptor', async () => {
  const base = fileURLToPath(new URL('../e2e/build/', import.meta.url));
  await mkdir(base, { recursive: true });
  const directory = await mkdtemp(join(base, 'playback-selection-'));
  try {
    const descriptor = join(directory, 'descriptor.json');
    await writeFile(descriptor, JSON.stringify({ ready: true, instanceID: selection.instanceID, userID: selection.instanceID, publicOrigin: endpoint }));
    const env = { SCREENROOM_BACKEND_ATTACH: '1', SCREENROOM_BACKEND_PROJECT: 'screenroom-jellyfin-local', SCREENROOM_E2E_API_ENDPOINT: endpoint, SCREENROOM_BACKEND_DESCRIPTOR: descriptor, SCREENROOM_SHOWCASE_ITEM_ID: selection.item.id, SCREENROOM_SHOWCASE_TITLE: selection.item.title, SCREENROOM_SHOWCASE_YEAR: '1922' };
    assert.throws(() => playbackSelection(env), /SCREENROOM_BACKEND_INSTANCE_ID/);
    assert.throws(() => playbackSelection({ ...env, SCREENROOM_BACKEND_INSTANCE_ID: 'wrong' }), /SCREENROOM_BACKEND_INSTANCE_ID/);
    assert.equal(playbackSelection({ ...env, SCREENROOM_BACKEND_INSTANCE_ID: selection.instanceID }).instanceID, selection.instanceID);
  } finally { await rm(directory, { recursive: true, force: true }); }
});
test('capture uses current UI observations, not audit outputs or render overrides', async () => {
  const source = await readFile(new URL('../e2e/playback.spec.ts', import.meta.url), 'utf8');
  assert.ok(source.includes('const before = await observe(page)'));
  assert.ok(source.includes('const after = await observe(page)'));
  assert.ok(source.includes("path: `${directory}/${image}`"));
  assert.doesNotMatch(source, /player-audit|5173|page\.route|addStyleTag|animations:|\.currentTime\s*=/);
});
test('playback freshness clears only playback and invalid journey/category clears nothing', async () => {
  const base = fileURLToPath(new URL('../e2e/build/', import.meta.url));
  await mkdir(base, { recursive: true });
  const directory = await mkdtemp(join(base, 'playback-clean-'));
  try {
    for (const name of ['clean-screenshots.mjs', 'categories.mjs']) await cp(new URL(`../e2e/${name}`, import.meta.url), join(directory, name));
    const output = join(directory, 'build/screenshots');
    await mkdir(join(output, 'playback'), { recursive: true });
    await mkdir(join(output, 'playback/previous-run'));
    await writeFile(join(output, 'home-desktop.png'), 'historical');
    await writeFile(join(output, 'playback/manifest.json'), 'stale');
    await utimes(join(output, 'playback'), new Date(0), new Date(0));
    for (const env of [{ SCREENROOM_E2E_JOURNEY: 'invalid' }, { SCREENROOM_E2E_JOURNEY: 'playback', E2E_CATEGORY: 'all' }]) {
      const result = spawnSync(process.execPath, [join(directory, 'clean-screenshots.mjs')], { env: { ...process.env, ...env }, encoding: 'utf8' });
      assert.notEqual(result.status, 0);
      assert.equal(await readFile(join(output, 'playback/manifest.json'), 'utf8'), 'stale');
    }
    const result = spawnSync(process.execPath, [join(directory, 'clean-screenshots.mjs')], { env: { ...process.env, SCREENROOM_E2E_JOURNEY: 'playback', E2E_CATEGORY: 'happy-path' }, encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr);
    await assert.rejects(readFile(join(output, 'playback/manifest.json')), { code: 'ENOENT' });
    assert.deepEqual(await readdir(join(output, 'playback')), []);
    assert.ok((await stat(join(output, 'playback'))).mtimeMs > Date.now() - 60_000);
    assert.equal(await readFile(join(output, 'home-desktop.png'), 'utf8'), 'historical');
  } finally { await rm(directory, { recursive: true, force: true }); }
});
