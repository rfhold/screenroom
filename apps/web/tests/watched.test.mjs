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

test('core adapter preserves explicit watched values and maps old-server omissions to unknown', async () => {
  const result = await build({
    stdin: { contents: `export { item, records } from './src/lib/core/client.ts';`, resolveDir: root },
    bundle: true, write: false, format: 'esm', platform: 'node',
    plugins: [{ name: 'wasm-fixture', setup(build) {
      build.onResolve({ filter: /screenroom_client_core/ }, args => ({ path: args.path, namespace: 'wasm-fixture' }));
      build.onLoad({ filter: /.*/, namespace: 'wasm-fixture' }, ({ path }) => ({ contents: path.endsWith('?url')
        ? 'export default "unused.wasm";'
        : 'export default async () => {}; export class Client {}; export const default_settings = () => ({}); export const normalize_saved_settings = value => value; export const validate_settings = value => value;' }));
    } }],
  });
  const { item, records } = await import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString('base64')}`);
  const record = { id: 'example', title: 'Example', kind: 'Movie', tags: [], colors: ['#000', '#fff'], provenance_notes: [], provider_references: [] };
  for (const watched of [true, false, null, undefined]) {
    const value = item(watched === undefined ? record : { ...record, watched });
    assert.equal(value.watched, watched ?? null);
    assert.equal(records.get(record.id), value);
  }
  assert.equal(item({ ...record, progress_percent: 100 }).watched, null);
});

test('mounted cards and details show only explicit watched state without reacquiring unchanged artwork', async () => {
  const wrapper = `<script>
    import Poster from './Poster.svelte';
    import PosterCard from './PosterCard.svelte';
    import Detail from './Detail.svelte';
    let item = $state({ id: 'example', title: 'Example', kind: 'Movie', year: 2030, meta: 'Metadata', overview: 'Overview', colors: ['#000', '#fff'], progress: 100, watched: undefined, providers: [], cast: [], crew: [], artwork: { url: 'poster', label: 'Example poster' } });
    window.setItem = (watched, url = 'poster', kind = 'Movie') => { item = { ...item, kind, watched, artwork: { ...item.artwork, url } }; };
  </script>
  <section data-testid="poster"><Poster {item} /></section>
  <section data-testid="card"><PosterCard {item} href="/?title=example" /></section>
  <section data-testid="detail"><Detail {item} /></section>`;
  const result = await build({
    stdin: { contents: `import { mount, unmount } from 'svelte'; import Wrapper from './Wrapper.svelte'; const app = mount(Wrapper, { target: document.body }); window.dispose = () => unmount(app);`, resolveDir: root },
    bundle: true, write: false, format: 'iife', platform: 'browser', conditions: ['browser'],
    plugins: [{ name: 'watched-fixture', setup(build) {
      build.onResolve({ filter: /Wrapper\.svelte$/ }, () => ({ path: 'wrapper', namespace: 'fixture' }));
      build.onLoad({ filter: /.*/, namespace: 'fixture' }, () => ({ contents: compile(wrapper, { filename: `${root}/Wrapper.svelte`, generate: 'client' }).js.code, resolveDir: `${root}/src/lib/components` }));
      build.onLoad({ filter: /\.svelte$/ }, async ({ path }) => ({ contents: compile(await readFile(path, 'utf8'), { filename: path, generate: 'client' }).js.code, resolveDir: `${root}/src/lib/components` }));
      build.onResolve({ filter: /^\.\.\/core\/client$/ }, () => ({ path: 'client', namespace: 'artwork-client' }));
      build.onLoad({ filter: /.*/, namespace: 'artwork-client' }, () => ({ contents: `
        import { createArtworkLoader } from './src/lib/core/artwork.ts';
        window.loads = [];
        export const acquireArtwork = createArtworkLoader(async url => {
          window.loads.push(url);
          return new TextEncoder().encode('<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><rect width="1" height="1" fill="red"/></svg>');
        });`, resolveDir: root }));
    } }],
  });
  const browser = await chromium.launch({ headless: true });
  try {
    const page = await browser.newPage();
    await page.setContent('<!doctype html><html><body></body></html>');
    await page.addScriptTag({ content: result.outputFiles[0].text });
    await page.waitForFunction(() => document.images.length === 3 && [...document.images].every(image => image.complete && image.naturalWidth));
    const sources = await page.locator('img').evaluateAll(images => images.map(image => image.src));
    for (const watched of [undefined, true, false, true, null, true, undefined]) {
      await page.evaluate(watched => window.setItem(watched), watched);
      await page.waitForFunction(watched => document.querySelectorAll('.watched').length === (watched === true ? 3 : 0), watched);
      assert.equal(await page.getByText('Watched', { exact: true }).count(), watched === true ? 3 : 0);
      const label = await page.getByRole('link').getAttribute('aria-label');
      assert.equal(label, `Example · Movie · Metadata${watched === true ? ' · Watched' : ''} · Playback progress 100%`);
      assert.equal(await page.getByRole('link', { name: label, exact: true }).count(), 1);
      const detailAccessibility = await page.getByTestId('detail').ariaSnapshot();
      if (watched === true) assert.match(detailAccessibility, /Watched/);
      else assert.doesNotMatch(detailAccessibility, /Watched|watched/);
      assert.equal(await page.getByRole('link').getAttribute('href'), '/?title=example');
      assert.equal(await page.locator('.progress span[style="width: 100%;"]').count(), 3);
      assert.deepEqual(await page.locator('img').evaluateAll(images => images.map(image => image.src)), sources);
      assert.deepEqual(await page.evaluate(() => window.loads), ['poster']);
      assert.equal(await page.locator('.watched svg:not([aria-hidden="true"])').count(), 0);
      assert.equal(await page.getByRole('button').count(), 0);
    }
    for (const kind of ['Movie', 'Episode', 'Season', 'Series']) {
      await page.evaluate(kind => window.setItem(true, 'poster', kind), kind);
      await page.waitForFunction(() => document.querySelectorAll('.watched').length === 3);
      assert.equal(await page.getByText('Watched', { exact: true }).count(), 3);
    }
    await page.evaluate(() => window.setItem(false, 'replacement'));
    await page.waitForFunction(old => document.images.length === 3 && [...document.images].every(image => image.src !== old && image.complete && image.naturalWidth), sources[0]);
    assert.equal(await page.locator('.watched').count(), 0);
    assert.deepEqual(await page.evaluate(() => window.loads), ['poster', 'replacement']);
    await page.evaluate(() => window.dispose());
  } finally { await browser.close(); }
});
