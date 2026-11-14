import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { compile } from 'svelte/compiler';

const require = createRequire(import.meta.url);
const { build } = createRequire(require.resolve('vite'))('esbuild');
const components = fileURLToPath(new URL('../src/lib/components', import.meta.url));

async function renderer() {
  const result = await build({
    stdin: { contents: `import { render } from 'svelte/server'; import Card from './PosterCard.svelte'; export default props => render(Card, { props }).body;`, resolveDir: components },
    bundle: true, write: false, format: 'esm', platform: 'node', conditions: ['node'],
    plugins: [{ name: 'card-fixture', setup(build) {
      build.onLoad({ filter: /\.svelte$/ }, async ({ path }) => {
        const source = path.endsWith('/RemoteImage.svelte')
          ? '<script>let { art } = $props();</script><img src={art.url} alt={art.label} />'
          : await readFile(path, 'utf8');
        return { contents: compile(source, { filename: path, generate: 'server' }).js.code, resolveDir: components };
      });
    } }],
  });
  const render = (await import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString('base64')}`)).default;
  return props => render(props).replace(/<!--[\s\S]*?-->/g, '');
}

test('cards render contextual headings, episode subtitles, accessible links, and matching poster placeholders', async () => {
  const render = await renderer();
  const item = { id: 'episode', kind: 'Episode', title: 'Actual episode title', meta: 'Series: Subtitle · S05E09', colors: ['#000', '#fff'], progress: 42 };
  for (const seasonContext of [false, true]) {
    const html = render({ item, href: '/?title=episode', seasonContext });
    const heading = seasonContext ? 'E09' : 'Series: Subtitle · S05:E09';
    assert.match(html, /href="\/\?title=episode"/);
    assert.ok(html.includes(`aria-label="${heading} · Episode · Actual episode title · Playback progress 42%"`));
    assert.match(html, new RegExp(`class="episode-number[^\"]*">${seasonContext ? 'E09' : 'S05:E09'}</span>`));
    if (seasonContext) assert.doesNotMatch(html, /class="series-name/);
    else assert.match(html, /class="series-name[^\"]*">Series: Subtitle<\/span>/);
    assert.match(html, /class="metadata[^\"]*">Actual episode title<\/span>/);
    assert.match(html, new RegExp(`class="art-title[^\"]*">${heading}</span>`));
    assert.match(html, /width: 42%/);
    assert.doesNotMatch(html, /S05E09/);
  }
  const missing = render({ item: { ...item, meta: '' }, href: '/episode' });
  assert.match(missing, /class="title[^\"]*">Episode<\/span>/);
  assert.match(missing, /class="metadata[^\"]*">Actual episode title<\/span>/);
  assert.doesNotMatch(missing, /class="episode-number|class="series-name/);
  const image = render({ item: { ...item, artwork: { url: '/episode-art', label: 'Own episode image' } }, href: '/episode', seasonContext: true });
  assert.match(image, /src="\/episode-art" alt="Own episode image"/);
  assert.doesNotMatch(image, /art-title|POSTER NOT INCLUDED/);
  for (const kind of ['Movie', 'Series', 'Season']) {
    const html = render({ item: { ...item, kind, title: 'Unchanged title', year: 2001, meta: 'Original metadata' }, href: '/original', seasonContext: true });
    assert.match(html, /class="title[^\"]*">Unchanged title<\/span>/);
    assert.match(html, new RegExp(`class="metadata[^\"]*">${kind === 'Movie' ? '2001' : 'Original metadata'}</span>`));
    assert.ok(html.includes(`aria-label="Unchanged title · ${kind} · Original metadata · Playback progress 42%"`));
    assert.doesNotMatch(html, /class="episode-number|class="series-name/);
  }
});

test('number styling is independently smaller and muted without muting the series heading', async () => {
  const source = await readFile(`${components}/PosterCard.svelte`, 'utf8');
  assert.match(source, /\.episode-number\s*\{[^}]*color: var\(--muted\);[^}]*font-size: 12px;[^}]*font-weight: 400;/);
  assert.match(source, /\.title\s*\{[^}]*font-weight: 600;[^}]*font-size: 14px;/);
});

test('only the season browse grid supplies episode context; home rows and search remain general', async () => {
  const home = await readFile(`${components}/Home.svelte`, 'utf8');
  const search = await readFile(`${components}/Search.svelte`, 'utf8');
  assert.equal((home.match(/seasonContext=/g) || []).length, 1);
  assert.match(home, /seasonContext=\{location\.path\.at\(-1\)\?\.kind === 'Season' && item\.parent === location\.path\.at\(-1\)\?\.id\}/);
  assert.doesNotMatch(search, /seasonContext/);
});
