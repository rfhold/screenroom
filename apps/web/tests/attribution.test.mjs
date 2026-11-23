import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { compile } from 'svelte/compiler';

const require = createRequire(import.meta.url);
const { build } = createRequire(require.resolve('vite'))('esbuild');
const components = fileURLToPath(new URL('../src/lib/components', import.meta.url));

async function renderer(component) {
  const result = await build({
    stdin: { contents: `import { render } from 'svelte/server'; import Component from './${component}.svelte'; export default props => render(Component, { props }).body;`, resolveDir: components },
    bundle: true, write: false, format: 'esm', platform: 'node', conditions: ['node'],
    plugins: [{ name: 'attribution-fixture', setup(build) {
      build.onResolve({ filter: /^\.\.\/core\/client$/ }, () => ({ path: 'client', namespace: 'fixture' }));
      build.onLoad({ filter: /.*/, namespace: 'fixture' }, () => ({ contents: 'export const artwork = art => art;' }));
      build.onLoad({ filter: /\.svelte$/ }, async ({ path }) => {
        const source = path.endsWith('/RemoteImage.svelte')
          ? '<script>let { art } = $props();</script><img src={art.url} alt={art.label} />'
          : await readFile(path, 'utf8');
        return { contents: compile(source, { filename: path, generate: 'server' }).js.code, resolveDir: components };
      });
    } }],
  });
  return (await import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString('base64')}`)).default;
}

test('home has no global attribution surface or footer-only metadata request', async () => {
  const source = await readFile(`${components}/Home.svelte`, 'utf8');
  assert.doesNotMatch(source, /ProviderAttribution|<footer|get_catalog_info|catalogInfo/);
});

test('item details omit rights and provenance panels while retaining item content', async () => {
  const render = await renderer('Detail');
  const art = { id: 'portrait', label: 'Portrait', creator: 'Portrait author', license: 'CC BY 3.0', licenseUrl: 'https://example.org/license', source: 'https://example.org/source' };
  const rights = { label: 'Title rights', note: 'Title terms', attribution: 'Title author' };
  const html = render({ item: {
    id: 'item', title: 'Example title', kind: 'Movie', year: 2030, meta: 'Example metadata',
    overview: 'Example overview', colors: ['#000000', '#ffffff'], providers: [
      { provider: 'Imdb', status: 'source_supplied', url: 'https://www.imdb.com/title/tt123/' },
      { provider: 'Tmdb', status: 'source_supplied', url: 'https://www.themoviedb.org/movie/123' },
    ],
    rights, metadataRights: { label: 'Metadata terms', attribution: 'Metadata author' },
    provenanceNotes: ['Server correction'], metadataRetrievedAt: '2030-01-01',
    artwork: { ...art, id: 'poster', label: 'Poster', creator: 'Poster author' },
    cast: [{ id: 'cast', name: 'Cast member', role: 'Actor', artwork: art }],
    crew: [{ id: 'crew', name: 'Crew member', role: 'Director', artwork: art }],
  } });
  for (const text of ['Example title', 'Example overview', 'Cast member', 'Crew member']) assert.ok(html.includes(text), text);
  for (const text of ['Imdb', 'Tmdb', 'Identifier supplied by Jellyfin', 'https://www.imdb.com/title/tt123/', 'https://www.themoviedb.org/movie/123']) assert.ok(html.includes(text), text);
  assert.doesNotMatch(html, /source_supplied/);
  assert.doesNotMatch(html, /<footer|provider-attribution|Data &amp; artwork credits/);
  for (const text of ['Title rights', 'Metadata rights', 'Artwork rights', 'Rights not supplied', 'Title terms', 'Metadata terms', 'Title author', 'Metadata author', 'Server correction', 'Metadata retrieved', '2030-01-01', 'Poster author', 'Portrait author', 'CC BY 3.0', 'https://example.org/license', 'https://example.org/source', 'Cropped for display', 'Source &amp; provenance']) assert.ok(!html.includes(text), text);
});
