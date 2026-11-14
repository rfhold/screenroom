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

test('mounted artwork keeps valid blob URLs when multiple pages replace item objects', async () => {
  const wrapper = `<script>
    import RemoteImage from './RemoteImage.svelte';
    let items = $state([]);
    window.appendPage = () => {
      items = Array.from({ length: items.length + 24 }, (_, id) => ({ id, art: { url: 'image-' + id, label: 'Image ' + id } }));
    };
    window.changeFirst = () => { items[0].art = { url: 'replacement', label: 'Replacement' }; };
  </script>
  {#each items as item (item.id)}<RemoteImage art={item.art} />{/each}`;
  const result = await build({
    stdin: { contents: `import { mount, unmount } from 'svelte'; import Wrapper from './Wrapper.svelte'; const app = mount(Wrapper, { target: document.body }); window.dispose = () => unmount(app);`, resolveDir: root },
    bundle: true, write: false, format: 'iife', platform: 'browser', conditions: ['browser'],
    plugins: [{ name: 'artwork-fixture', setup(build) {
      build.onResolve({ filter: /Wrapper\.svelte$/ }, () => ({ path: 'wrapper', namespace: 'fixture' }));
      build.onLoad({ filter: /.*/, namespace: 'fixture' }, () => ({ contents: compile(wrapper, { filename: `${root}/Wrapper.svelte`, generate: 'client' }).js.code, resolveDir: root }));
      build.onResolve({ filter: /RemoteImage\.svelte$/ }, () => ({ path: `${root}/src/lib/components/RemoteImage.svelte` }));
      build.onLoad({ filter: /RemoteImage\.svelte$/ }, async ({ path }) => ({ contents: compile(await readFile(path, 'utf8'), { filename: path, generate: 'client' }).js.code, resolveDir: `${root}/src/lib/components` }));
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
    await page.evaluate(() => window.appendPage());
    await page.waitForFunction(() => document.images.length === 24 && [...document.images].every(image => image.complete && image.naturalWidth));
    const first = await page.locator('img').evaluateAll(images => images.map(image => image.src));
    for (const count of [48, 72, 96]) {
      await page.evaluate(() => window.appendPage());
      await page.waitForFunction(count => document.images.length === count && [...document.images].every(image => image.complete && image.naturalWidth), count);
      assert.deepEqual(await page.locator('img').evaluateAll(images => images.slice(0, 24).map(image => image.src)), first);
      assert.equal(await page.evaluate(() => window.loads.length), count);
      assert.equal(await page.evaluate(async urls => (await Promise.all(urls.map(async url => (await fetch(url)).ok))).every(Boolean), first), true);
    }
    await page.evaluate(() => window.changeFirst());
    await page.waitForFunction(old => document.images[0].src !== old && document.images[0].complete && document.images[0].naturalWidth, first[0]);
    assert.equal(await page.locator('img').first().getAttribute('alt'), 'Replacement');
    assert.equal(await page.evaluate(async url => fetch(url).then(() => true, () => false), first[0]), false);
    const retained = await page.locator('img').evaluateAll(images => images.map(image => image.src));
    await page.evaluate(() => window.dispose());
    assert.equal(await page.evaluate(async urls => (await Promise.all(urls.map(url => fetch(url).then(() => false, () => true)))).every(Boolean), retained), true);
  } finally { await browser.close(); }
});
