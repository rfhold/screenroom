import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { compile } from 'svelte/compiler';
import { chromium } from '@playwright/test';

const require = createRequire(import.meta.url);
const { build } = createRequire(require.resolve('vite'))('esbuild');
const root = fileURLToPath(new URL('..', import.meta.url));
const previews = process.env.SCREENROOM_PLAYER_PREVIEW_DIR;

test('isolated player presentation keeps geometry, dismisses menus and hides only idle pointer controls', { timeout: 60_000 }, async () => {
  const theme = await readFile(`${root}/src/lib/theme.css`, 'utf8');
  const result = await build({
    stdin: {
      contents: `import { mount } from 'svelte'; import Screen from './src/lib/components/PlaybackScreen.svelte';
        mount(Screen, { target: document.body, props: { itemId:'isolated', title:'A very long title '.repeat(30), context:{source_id:'source',library_id:'library'}, onClose:()=>window.closedPlayer=true } });`,
      resolveDir: root,
    },
    bundle: true, write: false, format: 'iife', platform: 'browser', conditions: ['browser'],
    define: { 'import.meta.env.VITE_SCREENROOM_API_ENDPOINT': '""' },
    plugins: [{ name: 'isolated-player', setup(build) {
      build.onResolve({ filter: /\/core\/client$/ }, () => ({ path: 'client', namespace: 'fixture' }));
      build.onResolve({ filter: /\/core\/playback$/ }, () => ({ path: 'playback', namespace: 'fixture' }));
      build.onLoad({ filter: /.*/, namespace: 'fixture' }, ({ path }) => ({ contents: path === 'client'
        ? `export const records={clear(){}}; export const client=async()=>({invalidate_reads(){}}); export const newClient=async()=>({free(){}});`
        : `export const seconds=value=>Number(value)/1000; export const mediaURL=()=>{throw Error('No media request allowed')};
          export class Playback {
            busy=true; closed=false; blocked=false; session=null; status='Preparing player'; started=false;
            constructor(client,media,view){this.view=view;window.owner=this;}
            create(){ window.readyPlayer=()=>{
              this.busy=false;
              this.session={transport:{timeline_offset_ms:'10000',duration_ms:'600000'},tracks:[
                {id:'audio',kind:'audio',display_name:'A very long audio track '.repeat(20),available:true,selected:true},
                {id:'sub',kind:'subtitle',display_name:'English',available:true,selected:true,subtitle_mode:'webvtt'}]};
              this.view();const video=document.querySelector('video');video.dispatchEvent(new Event('loadeddata'));video.play();
            }; window.failPlayer=()=>{this.blocked=true;this.status='unknown';this.view()}; }
            event(){} progress(){} update(value){window.trackUpdate=value} exit(){this.closed=true} settled(){return Promise.resolve()}
          }` }));
      build.onLoad({ filter: /\.svelte$/ }, async ({ path }) => ({
        contents: compile(await readFile(path, 'utf8'), { filename: path, generate: 'client', css: 'injected' }).js.code,
        resolveDir: path.slice(0, path.lastIndexOf('/')),
      }));
    } }],
  });
  const browser = await chromium.launch({ headless: true });
  try {
    const page = await browser.newPage({ viewport: { width: 1440, height: 900 }, hasTouch: true });
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    await page.clock.install();
    await page.addInitScript(() => {
      let paused = true;
      Object.defineProperties(HTMLMediaElement.prototype, {
        paused: { get: () => paused },
        duration: { get: () => 590 },
        readyState: { get: () => 4 },
      });
      const attribute = HTMLMediaElement.prototype.getAttribute;
      HTMLMediaElement.prototype.getAttribute = function(name) { return name === 'src' ? 'isolated-media' : attribute.call(this, name); };
      HTMLMediaElement.prototype.play = function() { paused = false; this.dispatchEvent(new Event('play')); this.dispatchEvent(new Event('playing')); return Promise.resolve(); };
      HTMLMediaElement.prototype.pause = function() { paused = true; this.dispatchEvent(new Event('pause')); };
      HTMLMediaElement.prototype.load = function() {};
    });
    await page.route('http://player.test/**', route => route.fulfill(route.request().url().endsWith('/app.js')
      ? { contentType: 'text/javascript', body: result.outputFiles[0].text }
      : { contentType: 'text/html', body: `<!doctype html><html><head><style>${theme}</style></head><body><script src="/app.js"></script></body></html>` }));
    await page.goto('http://player.test/');
    assert.deepEqual(errors, []);
    const video = page.locator('video');
    await page.getByText('Preparing playback', { exact: true }).waitFor();
    const geometry = await video.boundingBox();
    await video.evaluate(node => { window.originalVideo = node; });
    await page.clock.runFor(4000);
    assert.equal(await page.locator('header').evaluate(node => node.inert), false);
    await page.evaluate(() => window.readyPlayer());
    assert.deepEqual(await video.boundingBox(), geometry);
    if (previews) await page.screenshot({ path: `${previews}/player-polish-preview-desktop.png` });
    await page.mouse.move(700, 400);
    await page.clock.runFor(3200);
    assert.equal(await page.locator('.controls').evaluate(node => node.inert), true);
    assert.equal(await page.locator('header').evaluate(node => node.inert), true);
    assert.equal(await page.locator('section').evaluate(node => node === document.activeElement), true, 'entry autofocus transfers to the stable stage before hiding');
    for (const key of ['Shift', 'ArrowRight', 'x']) {
      await page.keyboard.press(key);
      assert.equal(await page.locator('.controls').evaluate(node => node.inert), false, `${key} reveals chrome`);
      assert.equal(await page.locator('section').evaluate(node => node === document.activeElement), true, `${key} leaves focus on the stage`);
      await page.clock.runFor(3200);
      assert.equal(await page.locator('.controls').evaluate(node => node.inert), true, `${key} on the stage must not latch chrome open`);
    }
    await page.keyboard.press('Tab');
    assert.equal(await page.locator('.controls').evaluate(node => node.inert), false);
    assert.equal(await page.getByRole('button', { name: 'Back to details' }).evaluate(node => node === document.activeElement), true);
    await page.clock.runFor(4000);
    assert.equal(await page.locator('.controls').evaluate(node => node.inert), false);
    await page.getByRole('button', { name: 'Mute', exact: true }).click();
    await page.clock.runFor(3200);
    assert.equal(await page.locator('.controls').evaluate(node => node.inert), true, 'pointer focus must not latch visible controls');
    assert.equal(await page.locator('section').evaluate(node => node === document.activeElement), true, 'hidden buttons cannot retain focus');
    await page.touchscreen.tap(700, 400);
    assert.equal(await page.locator('.controls').evaluate(node => node.inert), false);
    assert.equal(await video.evaluate(node => node.paused), false, 'a reveal tap must not accidentally toggle playback');
    await page.mouse.move(710, 400);
    await page.getByRole('button', { name: 'Playback settings', exact: true }).click();
    await page.clock.runFor(4000);
    assert.equal(await page.locator('.controls').evaluate(node => node.inert), false);
    assert.deepEqual(await video.boundingBox(), geometry);
    assert.equal(await video.evaluate(node => node === window.originalVideo), true);
    await page.keyboard.press('Escape');
    assert.equal(await page.locator('#player-settings').count(), 0);
    assert.equal(await page.getByRole('button', { name: 'Playback settings' }).evaluate(node => node === document.activeElement), true);
    assert.equal(await page.evaluate(() => !!window.closedPlayer), false);
    await page.getByRole('button', { name: 'Playback settings', exact: true }).click();
    await page.mouse.click(700, 400);
    assert.equal(await page.locator('#player-settings').count(), 0);
    await page.getByRole('button', { name: 'Back 30 seconds' }).click();
    assert.equal(await video.evaluate(node => node.currentTime), 0, 'seek clamps to transport offset');
    await page.getByRole('button', { name: 'Forward 30 seconds' }).click();
    assert.equal(await video.evaluate(node => node.currentTime), 30);
    await page.getByRole('button', { name: 'Pause', exact: true }).click();
    await page.clock.runFor(4000);
    assert.equal(await page.locator('.controls').evaluate(node => node.inert), false);
    await page.getByRole('button', { name: 'Play', exact: true }).click();
    const slider = await page.getByRole('slider', { name: 'Playback position' }).boundingBox();
    await page.mouse.move(slider.x + slider.width / 2, slider.y + slider.height / 2);
    await page.mouse.down();
    await page.clock.runFor(4000);
    assert.equal(await page.locator('.controls').evaluate(node => node.inert), false);
    await page.mouse.up();
    await page.clock.runFor(3200);
    assert.equal(await page.locator('.controls').evaluate(node => node.inert), true);
    for (const size of [{ width: 320, height: 568 }, { width: 568, height: 280 }, { width: 1920, height: 1080 }]) {
      await page.setViewportSize(size);
      await page.mouse.move(100, 100);
      await page.getByRole('button', { name: 'Playback settings', exact: true }).click();
      const bounds = await video.boundingBox();
      assert.deepEqual(bounds, { x: 0, y: 0, ...size });
      const menu = await page.locator('#player-settings').boundingBox();
      assert.ok(menu.x >= 0 && menu.y >= 0 && menu.x + menu.width <= size.width && menu.y + menu.height <= size.height);
      assert.equal(await page.locator('dialog').evaluate(node => node.scrollWidth <= node.clientWidth), true);
      for (const button of await page.locator('.toolbar button').all()) {
        const box = await button.boundingBox();
        assert.ok(box.width >= 44 && box.height >= 44 && box.x >= 0 && box.x + box.width <= size.width);
      }
      await page.getByRole('combobox', { name: 'Subtitles' }).selectOption('');
      assert.equal(await page.evaluate(() => window.trackUpdate.subtitles_off), true);
      if (size.width === 320) {
        await page.getByRole('slider', { name: 'Volume', exact: true }).fill('0.4');
        assert.equal(await video.evaluate(node => node.volume), 0.4);
        if (previews) await page.screenshot({ path: `${previews}/player-polish-preview-mobile-menu.png` });
      }
      await page.keyboard.press('Escape');
      assert.deepEqual(await video.boundingBox(), bounds);
      if (previews && size.width === 320) await page.screenshot({ path: `${previews}/player-polish-preview-mobile.png` });
    }
    const beforeError = await video.boundingBox();
    await page.evaluate(() => window.failPlayer());
    await page.getByText('Playback stopped', { exact: true }).waitFor();
    await page.clock.runFor(4000);
    assert.equal(await page.locator('.controls').evaluate(node => node.inert), false);
    assert.deepEqual(await video.boundingBox(), beforeError);
    assert.equal(await video.evaluate(node => node === window.originalVideo), true);
    await page.keyboard.press('Escape');
    assert.equal(await page.evaluate(() => window.closedPlayer), true);
    assert.deepEqual(errors, []);
  } finally {
    await browser.close();
  }
});
