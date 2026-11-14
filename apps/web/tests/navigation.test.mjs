import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const require = createRequire(import.meta.url);
const { build } = createRequire(require.resolve('vite'))('esbuild');
const result = await build({
  entryPoints: [fileURLToPath(new URL('../src/lib/core/navigation.ts', import.meta.url))],
  bundle: true, write: false, format: 'esm', platform: 'node',
  plugins: [{ name: 'navigation-client', setup(build) {
    build.onResolve({ filter: /^\.\/client$/ }, () => ({ path: 'client', namespace: 'fixture' }));
    build.onLoad({ filter: /.*/, namespace: 'fixture' }, () => ({ contents: `
      export const records = new Map();
      export const libraries = ['all', 'movies', 'tv', 'opaque-library', '75dc7eb4-9022-4528-9bae-7831b849314b'].map(id => ({ id, title: 'Duplicate title' }));
      export const capabilities = { sources: ['all', 'jellyfin'], maximum_query_length: 160 };
      export const scope = (library_id, source_id) => ({ library_id, source_id });
      export const loadSettings = async () => ({ scope: {}, source: 'all' });
      export const getItem = async () => undefined;
      export const client = async () => undefined;
      export const item = value => value;
      export const saveSettings = async () => {};
    ` }));
  } }],
});
const { readLocation, itemHref, libraryHref } = await import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString('base64')}`);

test('library identifiers never restrict server item kinds or hierarchy', () => {
  const series = { id: 'series', title: 'Show', kind: 'Series' };
  const season = { id: 'season', title: 'Season', kind: 'Season', parent: series.id };
  const episode = { id: 'episode', title: 'Episode', kind: 'Episode', parent: season.id };
  const movie = { id: 'movie', title: 'Movie', kind: 'Movie' };
  for (const id of ['movies', 'tv', 'opaque-library', '75dc7eb4-9022-4528-9bae-7831b849314b']) {
    const base = `https://app.test/?screen=browse&library=${id}&source_id=source-uuid`;
    const location = readLocation(new URL(`${base}&series=series&season=season&title=episode`), [series, season, episode, movie]);
    assert.equal(location.library.id, id);
    assert.deepEqual(location.path, [series, season]);
    assert.equal(location.title, episode);
    assert.equal(location.sourceId, 'source-uuid');
    assert.equal(readLocation(new URL(`${base}&title=movie`), [movie]).title, movie);
    const href = new URL(itemHref(location, episode), 'https://app.test');
    assert.equal(href.searchParams.get('library'), id);
    assert.equal(href.searchParams.get('source_id'), 'source-uuid');
    assert.equal(new URL(libraryHref(location.library, 'source-uuid'), 'https://app.test').searchParams.get('library'), id);
    assert.equal(readLocation(new URL(`${base}&series=series&season=season&title=movie`), [series, season, episode, movie]).title, undefined);
  }
});
