import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { readFile, mkdir, mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { compile } from 'svelte/compiler';
import { chromium } from '@playwright/test';
const require = createRequire(import.meta.url);
const { build } = createRequire(require.resolve('vite'))('esbuild');
const root = fileURLToPath(new URL('..', import.meta.url));
const id = '11111111-1111-4111-8111-111111111111';
const sub = '22222222-2222-4222-8222-222222222222';
const audio = '44444444-4444-4444-8444-444444444444';
const alternateAudio = '55555555-5555-4555-8555-555555555555';
const cap = 'b'.repeat(64);
function varint(value) {
  const bytes = []; let n = BigInt(value);
  while (n >= 128n) { bytes.push(Number(n & 127n) | 128); n >>= 7n; }
  bytes.push(Number(n)); return Buffer.from(bytes);
}
function field(number, value) {
  if (typeof value === 'number') return Buffer.concat([varint(number * 8), varint(value)]);
  const bytes = Buffer.isBuffer(value) ? value : Buffer.from(value);
  return Buffer.concat([varint(number * 8 + 2), varint(bytes.length), bytes]);
}
const message = (...fields) => Buffer.concat(fields.map(([number, value]) => field(number, value)));
function decode(bytes) {
  let offset = 0; const fields = {};
  function integer() { let n = 0, shift = 0, byte; do { byte = bytes[offset++]; n += (byte & 127) * 2 ** shift; shift += 7; } while (byte & 128); return n; }
  while (offset < bytes.length) {
    const key = integer(), number = key >> 3, wire = key & 7;
    if (wire === 0) fields[number] = integer();
    else if (wire === 2) { const length = integer(); fields[number] = bytes.subarray(offset, offset + length); offset += length; }
    else if (wire === 1) { fields[number] = bytes.readDoubleLE(offset); offset += 8; }
    else throw new Error(`Unexpected wire ${wire}`);
  }
  return fields;
}
function frame(bytes, trailer = false) {
  const head = Buffer.alloc(5); head[0] = trailer ? 128 : 0; head.writeUInt32BE(bytes.length, 1); return Buffer.concat([head, bytes]);
}
async function listen(server) {
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  return `http://127.0.0.1:${server.address().port}`;
}
for (const mode of ['mp4', 'hls']) test(`mounted real WASM ${mode} decodes owned clip across origins with anonymous WebVTT and revocation`, { timeout: 60_000 }, async t => {
  const tempRoot = join(tmpdir(), 'opencode');
  await mkdir(tempRoot, { recursive: true });
  const temp = await mkdtemp(join(tempRoot, 'playback-'));
  const requests = [], events = [], rpc = [];
  let origin, state = 2, sequence = 0, revision = 1;
  let releaseCreate;
  const createGate = new Promise(resolve => { releaseCreate = resolve; });
  let startPosition = '1000', selectedAudio = audio, subtitles = true;
  const mediaPath = resource => `/playback/v1/media/${id}/${revision}/${resource}?cap=${cap}`;
  const url = resource => `${origin}${mediaPath(resource)}`;
  const session = () => message([1, id], [2, 'a'.repeat(64)], [3, state],
    [5, message([1, sub], [2, 2], [3, 'English'], [4, 'en'], [5, Number(subtitles)], [6, 1], [7, 1], [8, url(sub)])],
    ...[[audio, 'English'], [alternateAudio, 'Commentary']].map(([id, name]) => [5, message([1, id], [2, 1], [3, name], [5, Number(selectedAudio === id)], [6, 1])]),
    ...(state === 6 ? [] : [[6, message([1, mode === 'mp4' ? 1 : 3], [2, mode === 'mp4' ? 'video/mp4' : 'application/vnd.apple.mpegurl'], [3, url(id)], [4, revision], [5, '9000'], [6, startPosition], [7, '1000'])]]),
    [7, revision], [8, sequence], [9, sequence ? 3 : 1], [10, '2030-01-01T00:00:00Z'], [11, '2030-01-02T00:00:00Z']);
  const server = createServer(async (req, res) => {
    try {
      res.setHeader('Access-Control-Allow-Origin', '*');
      res.setHeader('Access-Control-Allow-Headers', 'content-type,x-grpc-web,x-screenroom-user-id,grpc-timeout,x-user-agent');
      res.setHeader('Access-Control-Expose-Headers', 'grpc-status,grpc-message');
      res.setHeader('Cache-Control', 'no-store');
      res.setHeader('Referrer-Policy', 'no-referrer');
      if (req.method === 'OPTIONS') { res.end(); return; }
      if (req.method === 'POST') {
        const chunks = []; for await (const chunk of req) chunks.push(chunk);
        const bytes = Buffer.concat(chunks), body = decode(bytes.subarray(5, 5 + bytes.readUInt32BE(1)));
        rpc.push({ method: req.url, body, selector: req.headers['x-screenroom-user-id'] });
        let response;
        if (req.url.endsWith('/ReportPlaybackEvent')) {
          sequence = body[4]; events.push({ kind: body[5], sequence, position: body[6]?.toString() });
          response = message([1, sequence], [2, 3], [3, session()]);
        } else {
          if (req.url.endsWith('/CreatePlaybackSession')) await createGate;
          if (req.url.endsWith('/StopPlaybackSession')) state = 6;
          if (req.url.endsWith('/UpdatePlaybackSession')) {
            revision++; startPosition = body[5].toString();
            const choices = body[6] ? decode(body[6]) : {};
            if (choices[1]) selectedAudio = choices[1].toString();
            if (choices[2]) subtitles = true;
            if (choices[3]) subtitles = false;
          }
          response = message([1, session()]);
        }
        res.setHeader('Content-Type', 'application/grpc-web+proto');
        res.end(Buffer.concat([frame(response), frame(Buffer.from('grpc-status: 0\r\n'), true)])); return;
      }
      requests.push({ url: req.url, cookie: req.headers.cookie, referer: req.headers.referer, origin: req.headers.origin });
      const resource = new URL(req.url, origin).pathname.split('/').at(-1);
      if (state === 6) { res.writeHead(410); res.end(); return; }
      let bytes;
      if (resource === sub) { res.setHeader('Content-Type', 'text/vtt'); bytes = Buffer.from('WEBVTT\n\n00:00:00.000 --> 00:00:06.000\nOwned subtitle\n'); }
      else if (mode === 'hls' && resource === id) {
        res.setHeader('Content-Type', 'application/vnd.apple.mpegurl');
        bytes = Buffer.from(`#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:8\n#EXT-X-MEDIA-SEQUENCE:0\n#EXTINF:8,\n${mediaPath('33333333-3333-4333-8333-333333333333')}\n#EXT-X-ENDLIST\n`);
      } else { res.setHeader('Content-Type', mode === 'mp4' ? 'video/mp4' : 'video/mp2t'); bytes = await readFile(`${temp}/${mode === 'mp4' ? 'clip.mp4' : 'clip.ts'}`); }
      const match = /bytes=(\d+)-(\d*)/.exec(req.headers.range || '');
      if (match) {
        const start = Number(match[1]), end = Math.min(Number(match[2] || bytes.length - 1), bytes.length - 1);
        res.statusCode = 206; res.setHeader('Content-Range', `bytes ${start}-${end}/${bytes.length}`); bytes = bytes.subarray(start, end + 1);
      }
      res.setHeader('Content-Length', bytes.length); res.end(bytes);
    } catch (error) { res.statusCode = 500; res.end(String(error)); }
  });
  let spa, browser;
  try {
    await promisify(execFile)('ffmpeg', ['-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'testsrc=size=160x90:rate=24', '-f', 'lavfi', '-i', 'sine=frequency=440:sample_rate=48000', '-t', '8', '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-profile:v', 'baseline', '-c:a', 'aac', '-movflags', '+faststart', `${temp}/clip.mp4`]);
    if (mode === 'hls') await promisify(execFile)('ffmpeg', ['-hide_banner', '-loglevel', 'error', '-i', `${temp}/clip.mp4`, '-c', 'copy', '-bsf:v', 'h264_mp4toannexb', '-f', 'mpegts', `${temp}/clip.ts`]);
    origin = await listen(server);
    const wrapper = `<script>
      import Detail from './src/lib/components/Detail.svelte';
      import PlaybackScreen from './src/lib/components/PlaybackScreen.svelte';
      import { revokePlayback } from './src/lib/core/playback-lifecycle';
      import { Client } from '../../target/client-core/wasm/screenroom_client_core';
      const get = Client.prototype.get_playback_session;
      Client.prototype.get_playback_session = async function(request) {
        const response = await get.call(this, request);
        window.checkedReads = (window.checkedReads || 0) + 1;
        return response;
      };
      window.revoke = revokePlayback;
      let playing = $state(false);
      let title = $state({ id: 'owned-movie', title: 'Owned clip', kind: 'Movie', year: 2030, meta: 'Owned media', overview: 'Details remain behind the player.', colors: ['#000', '#fff'], providers: [], cast: [], crew: [] });
      window.rerender = () => { title = { ...title, overview: 'Updated metadata' }; };
    </script>
    <nav aria-label="Catalog navigation"><a href="#catalog">Catalog</a></nav>
    <div style="height: 180px"></div>
    <Detail item={title} onPlay={() => playing = true} />
    {#if playing}<PlaybackScreen itemId={title.id} title={title.title} context={{source_id:'source',library_id:'library'}} onClose={() => playing = false} />{/if}
    <div style="height: 1200px"></div>`;
    const result = await build({
      stdin: { contents: `import { mount, unmount } from 'svelte'; import App from './PlaybackTest.svelte'; const app = mount(App, {target:document.body}); window.dispose = () => unmount(app);`, resolveDir: root },
      bundle: true, write: false, format: 'iife', platform: 'browser', conditions: ['browser'],
      define: {
        'import.meta.env.VITE_SCREENROOM_SAME_ORIGIN': '""',
        'import.meta.env.VITE_SCREENROOM_API_ENDPOINT': JSON.stringify(origin),
        'import.meta.env.VITE_SCREENROOM_USER_ID': JSON.stringify(id)
      },
      plugins: [{ name: 'playback-owned', setup(build) {
        build.onResolve({ filter: /PlaybackTest\.svelte$/ }, () => ({ path: 'wrapper', namespace: 'owned' }));
        build.onLoad({ filter: /.*/, namespace: 'owned' }, () => ({ contents: compile(wrapper, { filename: `${root}/PlaybackTest.svelte`, generate: 'client', css: 'injected' }).js.code, resolveDir: root }));
        build.onLoad({ filter: /\.svelte$/ }, async ({ path }) => ({ contents: compile(await readFile(path, 'utf8'), { filename: path, generate: 'client', css: 'injected' }).js.code, resolveDir: path.slice(0, path.lastIndexOf('/')) }));
        build.onResolve({ filter: /\.wasm\?url$/ }, () => ({ path: 'wasm-url', namespace: 'wasm-url' }));
        build.onLoad({ filter: /.*/, namespace: 'wasm-url' }, () => ({ contents: 'export default "/core.wasm";' }));
      } }],
    });
    spa = createServer(async (req, res) => {
      if (req.url === '/core.wasm') { res.setHeader('Content-Type', 'application/wasm'); res.end(await readFile(`${root}/../../target/client-core/wasm/screenroom_client_core_bg.wasm`)); }
      else if (req.url === '/app.js') { res.setHeader('Content-Type', 'text/javascript'); res.end(result.outputFiles[0].text); }
      else { res.setHeader('Content-Type', 'text/html'); res.end(`<!doctype html><html><head><meta name="referrer" content="no-referrer"><style>${await readFile(`${root}/src/lib/theme.css`, 'utf8')}</style></head><body><script src="/app.js"></script></body></html>`); }
    });
    const spaOrigin = await listen(spa);
    assert.notEqual(spaOrigin, origin);
    browser = await chromium.launch({ headless: true });
    const page = await browser.newPage();
    await page.setViewportSize({ width: 1440, height: 900 });
    page.setDefaultTimeout(8000);
    await page.context().addCookies([{ name: 'must_not_send', value: 'owned-test-cookie', url: origin }]);
    const errors = []; page.on('pageerror', error => errors.push(error.message));
    await page.goto(spaOrigin);
    await page.evaluate(() => { document.body.style.overflow = 'auto'; window.scrollTo(0, 100); });
    await page.getByRole('button', { name: 'Play', exact: true }).click();
    await page.getByRole('dialog').waitFor();
    const screen = page.getByRole('dialog');
    const screenshots = `${root}/../../build/player-polish`;
    await mkdir(screenshots, { recursive: true });
    await page.getByText('Preparing playback', { exact: true }).waitFor();
    await page.screenshot({ path: `${screenshots}/${mode}-1440-preparing.png` });
    releaseCreate();
    assert.equal(await page.locator('dialog').evaluate(dialog => dialog.matches(':modal')), true);
    assert.equal(await page.locator('nav a').evaluate(link => { link.focus(); return link === document.activeElement; }), false);
    await assert.rejects(page.locator('article button').click({ trial: true, timeout: 300 }));
    assert.equal(await page.evaluate(() => document.body.style.overflow), 'hidden');
    assert.equal(await page.getByRole('button', { name: 'Back to details' }).evaluate(button => button === document.activeElement), true);
    await page.waitForFunction(() => {
      const video = document.querySelector('video'); return video?.readyState >= 2 && video.videoWidth > 0;
    });
    if (await screen.getByRole('button', { name: 'Pause', exact: true }).count()) await screen.getByRole('button', { name: 'Pause', exact: true }).click();
    await screen.getByRole('button', { name: 'Play', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('video').currentTime > 0.25);
    await page.getByRole('button', { name: 'Pause', exact: true }).click();
    const decoded = await page.locator('video').evaluate(video => ({ width: video.videoWidth, height: video.videoHeight, frames: video.getVideoPlaybackQuality().totalVideoFrames }));
    assert.deepEqual([decoded.width, decoded.height], [160, 90]);
    assert.ok(decoded.frames > 0);
    t.diagnostic(`${mode} actual decoded media: ${JSON.stringify(decoded)}`);
    for (const viewport of [{ width: 1440, height: 900 }, { width: 390, height: 844 }]) {
      await page.setViewportSize(viewport);
      await page.evaluate(() => window.rerender());
      const geometry = await page.getByRole('dialog').evaluate(dialog => {
        const screen = dialog.getBoundingClientRect(), video = dialog.querySelector('video').getBoundingClientRect();
        return { x: screen.x, y: screen.y, width: screen.width, height: screen.height, videoWidth: video.width, videoHeight: video.height, overflow: dialog.scrollWidth > dialog.clientWidth };
      });
      assert.deepEqual([geometry.x, geometry.y, geometry.width, geometry.height], [0, 0, viewport.width, viewport.height]);
      assert.equal(geometry.overflow, false);
      assert.ok(geometry.videoWidth >= viewport.width - 2, JSON.stringify(geometry));
      assert.ok(geometry.videoHeight > viewport.height * 0.5, JSON.stringify(geometry));
      t.diagnostic(`${mode} viewport ${viewport.width}x${viewport.height}: ${JSON.stringify(geometry)}`);
      await page.screenshot({ path: `${screenshots}/${mode}-${viewport.width}-decoded.png` });
    }
    await page.waitForFunction(() => {
      const video = document.querySelector('video'); return video?.readyState >= 2 && video.videoWidth > 0;
    });
    const fullscreenButton = page.getByRole('button', { name: 'Fullscreen', exact: true });
    if (await fullscreenButton.count()) {
      await fullscreenButton.click();
      await page.waitForFunction(() => document.fullscreenElement === document.querySelector('.viewport') || document.body.textContent.includes('Fullscreen is unavailable.'));
      if (await page.evaluate(() => !!document.fullscreenElement)) {
        t.diagnostic(`${mode}: browser fullscreen entered and exited through user actions`);
        await page.getByRole('button', { name: 'Exit fullscreen', exact: true }).click();
        await page.waitForFunction(() => !document.fullscreenElement);
      } else {
        assert.equal(await page.getByText('Fullscreen is unavailable. You can keep watching here.', { exact: true }).count(), 1);
        t.diagnostic(`${mode}: browser rejected fullscreen; truthful error verified`);
      }
    } else assert.equal(await page.getByRole('button', { name: 'Fullscreen unavailable', exact: true }).isDisabled(), true);
    await screen.getByRole('button', { name: 'Play', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('video').currentTime > 0.25);
    await page.waitForFunction(() => { const video = document.querySelector('video'); return video.textTracks.length === 1 && video.textTracks[0].cues?.length > 0; });
    assert.ok(events.some(event => event.kind === 1), JSON.stringify({ events, errors }));
    await page.getByRole('button', { name: 'Pause', exact: true }).click();
    await page.getByRole('button', { name: 'Mute', exact: true }).click();
    assert.equal(await page.locator('video').evaluate(video => video.muted), true);
    await page.getByRole('button', { name: 'Unmute', exact: true }).click();
    await page.getByRole('slider', { name: 'Volume', exact: true }).fill('0.4');
    assert.ok(Math.abs(await page.locator('video').evaluate(video => video.volume) - 0.4) < 0.01);
    const volumeSlider = screen.getByRole('slider', { name: 'Volume', exact: true });
    await volumeSlider.focus();
    await page.keyboard.press('ArrowLeft');
    await page.waitForFunction(() => Math.abs(document.querySelector('video').volume - 0.35) < 0.01);
    await page.keyboard.press('Home');
    await page.waitForFunction(() => document.querySelector('video').volume === 0);
    await screen.getByRole('button', { name: 'Unmute', exact: true }).focus();
    await page.keyboard.press('Space');
    await page.waitForFunction(() => {
      const video = document.querySelector('video');
      return !video.muted && Math.abs(video.volume - 0.35) < 0.01;
    });
    await screen.getByRole('button', { name: 'Mute', exact: true }).focus();
    await page.keyboard.press('Enter');
    await page.waitForFunction(() => document.querySelector('video').muted);
    await screen.getByRole('button', { name: 'Unmute', exact: true }).focus();
    await page.keyboard.press('Enter');
    await page.waitForFunction(() => {
      const video = document.querySelector('video');
      return !video.muted && Math.abs(video.volume - 0.35) < 0.01;
    });
    await screen.getByRole('button', { name: 'Mute', exact: true }).waitFor();
    await screen.getByRole('button', { name: 'Play', exact: true }).focus();
    await page.keyboard.press('Space');
    await page.waitForFunction(() => !document.querySelector('video').paused);
    await screen.getByRole('button', { name: 'Pause', exact: true }).focus();
    await page.keyboard.press('Enter');
    await page.waitForFunction(() => document.querySelector('video').paused);
    const positionSlider = screen.getByRole('slider', { name: 'Playback position' });
    await positionSlider.focus();
    await page.keyboard.press('Home');
    await page.waitForFunction(() => document.querySelector('video').currentTime < 0.05);
    await page.keyboard.press('ArrowRight');
    await page.waitForFunction(() => document.querySelector('video').currentTime > 0.05);
    assert.ok(Number(await positionSlider.inputValue()) > 1);
    await positionSlider.fill('2.5');
    await page.waitForFunction(() => Math.abs(document.querySelector('video').currentTime - 1.5) < 0.05);
    await page.waitForFunction(() => document.querySelector('[aria-label="Playback time"]').textContent.includes('0:02'));
    assert.match(await page.getByLabel('Playback time', { exact: true }).textContent(), /0:02.*0:09/);
    await page.locator('video').evaluate(video => {
      window.checkMediaChanges = 0;
      video.addEventListener('loadedmetadata', () => window.checkMediaChanges++);
      video.addEventListener('play', () => window.checkMediaChanges++);
    });
    await page.waitForFunction(() => {
      const video = document.querySelector('video');
      return video.paused && !video.seeking && Math.abs(video.currentTime - 1.5) < 0.05;
    });
    await page.getByRole('button', { name: 'Playback settings', exact: true }).click();
    await page.getByText('Session information', { exact: true }).click();
    await page.getByRole('button', { name: 'Check session state', exact: true }).click();
    await page.waitForFunction(() => window.checkedReads === 1);
    const afterCheck = await page.locator('video').evaluate(video => ({
      paused: video.paused, time: video.currentTime, changes: window.checkMediaChanges,
    }));
    assert.equal(afterCheck.paused, true);
    assert.ok(Math.abs(afterCheck.time - 1.5) < 0.05, JSON.stringify(afterCheck));
    assert.equal(afterCheck.changes, 0);
    await page.getByLabel('Audio', { exact: true }).selectOption(alternateAudio);
    await page.waitForFunction(() => {
      const video = document.querySelector('video'); return video.readyState >= 2 && video.paused && Math.abs(video.currentTime - 1.5) < 0.05;
    });
    assert.equal(selectedAudio, alternateAudio);
    await page.getByLabel('Subtitles', { exact: true }).selectOption('');
    await page.waitForFunction(() => document.querySelector('video').readyState >= 2 && document.querySelectorAll('track').length === 0);
    assert.equal(subtitles, false);
    await page.getByLabel('Subtitles', { exact: true }).selectOption(sub);
    await page.waitForFunction(() => document.querySelector('video').textTracks.length === 1 && document.querySelector('video').textTracks[0].cues?.length > 0);
    await screen.getByRole('button', { name: 'Play', exact: true }).waitFor({ state: 'visible' });
    await page.waitForFunction(() => !document.querySelector('.toolbar button').disabled);
    assert.ok(rpc.filter(call => call.method.endsWith('/UpdatePlaybackSession')).every(call => call.body[5].toString() === '2500'));
    await page.screenshot({ path: `${screenshots}/${mode}-390-settings.png` });
    await page.getByRole('button', { name: 'Playback settings', exact: true }).click();
    assert.equal(rpc.filter(call => call.method.endsWith('/CreatePlaybackSession')).length, 1);
    await page.evaluate(() => window.revoke());
    assert.equal(await page.locator('video').getAttribute('src'), null);
    assert.equal(await page.locator('track').count(), 0);
    await page.getByText('Playback stopped', { exact: true }).waitFor();
    await page.screenshot({ path: `${screenshots}/${mode}-390-stopped.png` });
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.screenshot({ path: `${screenshots}/${mode}-1440-stopped.png` });
    if (mode === 'mp4') await page.getByRole('button', { name: 'Back to details' }).click();
    else await page.keyboard.press('Escape');
    await page.waitForFunction(() => !document.querySelector('dialog'));
    assert.equal(await page.getByRole('button', { name: 'Play', exact: true }).evaluate(button => button === document.activeElement), true);
    assert.equal(await page.evaluate(() => document.body.style.overflow), 'auto');
    assert.equal(await page.getByRole('navigation', { name: 'Catalog navigation' }).count(), 1);
    await page.getByRole('button', { name: 'Play', exact: true }).click();
    await page.getByRole('dialog').waitFor();
    if (mode === 'hls') {
      await page.getByRole('button', { name: 'Close player', exact: true }).click();
      await page.waitForFunction(() => !document.querySelector('dialog'));
      assert.equal(await page.getByRole('button', { name: 'Play', exact: true }).evaluate(button => button === document.activeElement), true);
      assert.equal(await page.evaluate(() => document.body.style.overflow), 'auto');
    }
    await page.evaluate(() => window.dispose());
    assert.equal(await page.locator('dialog').count(), 0);
    assert.equal(await page.evaluate(() => document.body.style.overflow), 'auto');
    assert.ok(rpc.every(call => call.selector === id));
    const create = rpc.find(call => call.method.endsWith('/CreatePlaybackSession'));
    assert.equal(create.body[2].toString(), 'owned-movie');
    const scope = decode(create.body[3]);
    assert.equal(scope[1].toString(), 'source');
    assert.equal(scope[2].toString(), 'library');
    assert.equal(events.filter(event => event.kind === 1).length, 1);
    assert.ok(requests.some(request => request.url.includes(sub)));
    assert.ok(requests.every(request => !request.cookie && !request.referer));
    assert.ok(requests.every(request => request.origin === spaOrigin));
    assert.deepEqual(errors, []);
    assert.ok(rpc.some(call => call.method.endsWith('/StopPlaybackSession')));
  } finally {
    await browser?.close();
    if (spa) { spa.closeAllConnections(); await new Promise(resolve => spa.close(resolve)); }
    server.closeAllConnections(); await new Promise(resolve => server.close(resolve));
    await rm(temp, { recursive: true, force: true });
  }
});
