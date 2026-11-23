import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { routeId } from '../e2e/route-id.mjs';

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
const { readLocation, itemHref, libraryHref, safeReturn, grantPlayback, consumePlayback } = await import(`data:text/javascript;base64,${Buffer.from(result.outputFiles[0].text).toString('base64')}`);

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
    assert.equal(new URL(libraryHref(location.library, 'source-uuid'), 'https://app.test').pathname, `/libraries/${id}`);
    assert.equal(readLocation(new URL(`${base}&series=series&season=season&title=movie`), [series, season, episode, movie]).title, undefined);
  }
});

test('native IDs are opaque, invalid scopes fail closed, and returns are internal-only', () => {
  assert.equal(libraryHref({ id: 'opaque/a b' }), '/libraries/opaque%2Fa%20b');
  assert.equal(readLocation(new URL('https://app.test/libraries/missing')).notFound, true);
  assert.equal(readLocation(new URL('https://app.test/titles/missing')).notFound, true);
  assert.equal(readLocation(new URL('https://app.test/titles/%zz')).notFound, true);
  assert.equal(readLocation(new URL('https://app.test/titles/season?series=wrong'), [
    { id: 'wrong', kind: 'Series' }, { id: 'parent', kind: 'Series' }, { id: 'season', kind: 'Season', parent: 'parent' },
  ]).notFound, true);
  for (const input of ['https://evil.test/', '//evil.test/', '/\\evil.test/', 'javascript:alert(1)', '/play/other', '/arbitrary']) assert.equal(safeReturn(input, '/'), '/');
  assert.equal(safeReturn('/titles/a?library=b&source_id=c', '/'), '/titles/a?library=b&source_id=c');
});

test('playback launch intent is memory-only and consumed exactly once', () => {
  grantPlayback('/play/a');
  assert.equal(consumePlayback('/play/a'), true);
  assert.equal(consumePlayback('/play/a'), false);
  grantPlayback('/play/a');
  assert.equal(consumePlayback('/play/b'), false);
  assert.equal(consumePlayback('/play/a'), false);
});

test('native episode detail derives validated series and season breadcrumbs', () => {
  const catalog = [
    { id: 'series', kind: 'Series' },
    { id: 'season', kind: 'Season', parent: 'series' },
    { id: 'episode', kind: 'Episode', parent: 'season' },
    { id: 'other', kind: 'Series' },
  ];
  const location = readLocation(new URL('https://app.test/titles/episode?library=opaque-library'), catalog);
  assert.deepEqual(location.path.map(item => item.id), ['series', 'season']);
  assert.equal(location.notFound, false);
  assert.equal(readLocation(new URL('https://app.test/titles/episode?series=other'), catalog).notFound, true);
});

test('showcase reads opaque native path IDs rather than retired query IDs', () => {
  const id = 'opaque /%?#雪';
  for (const resource of ['libraries', 'titles']) {
    assert.equal(routeId(`/${resource}/${encodeURIComponent(id)}?source_id=source`, resource, 'https://app.test'), id);
    assert.throws(() => routeId('/?library=old&title=old', resource, 'https://app.test'), /Expected/);
    assert.throws(() => routeId(`/${resource}/first/second`, resource, 'https://app.test'), /Expected/);
  }
});
