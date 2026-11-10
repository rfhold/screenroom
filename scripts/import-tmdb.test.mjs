import { readFile } from 'node:fs/promises';
import { test } from 'node:test';
import assert from 'node:assert/strict';

const scriptUrl = new URL('./import-tmdb.mjs', import.meta.url);
const source = (await readFile(scriptUrl, 'utf8')).replace(/^import[^\n]+\n/, '').replaceAll('import.meta.url', JSON.stringify(scriptUrl.href));
const execute = new (Object.getPrototypeOf(async function () {}).constructor)('writeFile', 'rename', 'process', 'console', 'fetch', source);
const films = [['Nosferatu', 1922], ['The General', 1926], ['His Girl Friday', 1940], ['Night of the Living Dead', 1968], ['Big Buck Bunny', 2008], ['Sintel', 2010], ['Tears of Steel', 2012]];
const sprite = () => ({ id: 891761, title: 'Sprite Fright', release_date: '2024-04-06', homepage: 'https://studio.blender.org/films/sprite-fright/', credits: { cast: [], crew: [{ id: 1993493, name: 'Matthew Luhn', job: 'Director' }] } });

async function run(change = d => d) {
  let serialized;
  let renamed = false;
  let writeTarget;
  let renameSource;
  let renameTarget;
  const errors = [];
  const process = { env: { TMDB_READ_ACCESS_TOKEN: 'test-only-token' }, exitCode: 0 };
  const fetch = async (url, options) => {
    assert.equal(new URL(url).origin, 'https://api.themoviedb.org');
    assert.equal(options.headers.Authorization, 'Bearer test-only-token');
    assert.equal(options.redirect, 'error');
    const u = new URL(url);
    let d;
    if (u.pathname.endsWith('/search/movie')) {
      const index = films.findIndex(([title]) => title === u.searchParams.get('query'));
      assert.notEqual(index, -1, 'Sprite Fright must not use a loose search fallback');
      d = { results: [{ id: index + 1, title: films[index][0], release_date: `${films[index][1]}-01-01` }] };
    } else if (/\/movie\/\d+$/.test(u.pathname)) {
      const id = Number(u.pathname.split('/').at(-1));
      d = id === 891761 ? change(sprite()) : { id, title: films[id - 1][0], release_date: `${films[id - 1][1]}-01-01`, credits: { cast: [], crew: [] } };
    } else if (u.pathname.endsWith('/search/tv')) {
      d = { results: [{ id: 90, name: 'The Beverly Hillbillies', first_air_date: '1962-09-26' }] };
    } else if (/\/episode\/\d+$/.test(u.pathname)) {
      const n = Number(u.pathname.split('/').at(-1));
      d = { id: 100 + n, name: n === 1 ? 'The Clampetts Strike Oil' : 'Getting Settled', air_date: n === 1 ? '1962-09-26' : '1962-10-03', season_number: 1, episode_number: n, credits: { cast: [], crew: [] } };
    } else if (u.pathname.endsWith('/season/1')) {
      d = { id: 91, name: 'Season 1', season_number: 1, air_date: '1962-09-26', vote_average: undefined, vote_count: undefined, credits: { cast: [], crew: [] } };
    } else {
      d = { id: 90, name: 'The Beverly Hillbillies', first_air_date: '1962-09-26', created_by: [], credits: { cast: [], crew: [] } };
    }
    return { ok: true, json: async () => ({ overview: 'Synthetic importer test data, never a production fixture.', vote_average: 1, vote_count: 1, ...d }) };
  };
  await execute(async (path, text) => { writeTarget = path.href; serialized = text; }, async (from, to) => { renameSource = from.href; renameTarget = to.href; renamed = true; }, process, { log() {}, error(message) { errors.push(message); } }, fetch);
  return { serialized, renamed, writeTarget, renameSource, renameTarget, errors, exitCode: process.exitCode };
}

test('explicit Sprite Fright identity preserves TMDB date and records original-year discrepancy', async () => {
  const result = await run();
  assert.equal(result.exitCode, 0);
  assert.equal(result.renamed, true);
  const data = JSON.parse(result.serialized);
  assert.equal(Object.keys(data.items).length, 12);
  assert.equal(data.items['sprite-fright'].date, '2024-04-06');
  assert.match(data.items['sprite-fright'].dateNote, /Original Blender release: 2021/);
  assert.equal(data.items['sprite-fright'].url, 'https://www.themoviedb.org/movie/891761');
  assert.equal(result.serialized.includes('test-only-token'), false);
});

for (const [name, change] of [
  ['ID', d => ({ ...d, id: 1 })],
  ['title', d => ({ ...d, title: 'Another Film' })],
  ['director identity', d => ({ ...d, credits: { crew: [{ id: 1, name: 'Matthew Luhn', job: 'Director' }] } })],
  ['director name', d => ({ ...d, credits: { crew: [{ id: 1993493, name: 'Another Person', job: 'Director' }] } })],
  ['director role', d => ({ ...d, credits: { crew: [{ id: 1993493, name: 'Matthew Luhn', job: 'Producer' }] } })],
  ['homepage protocol', d => ({ ...d, homepage: 'http://studio.blender.org/films/sprite-fright/' })],
  ['homepage host', d => ({ ...d, homepage: 'https://studio.blender.org.example.org/films/sprite-fright/' })],
  ['homepage path', d => ({ ...d, homepage: 'https://studio.blender.org/films/another-film/' })]
]) {
  test(`rejects incorrect Sprite Fright ${name} before any fixture write`, async () => {
    const result = await run(change);
    assert.equal(result.exitCode, 1);
    assert.equal(result.serialized, undefined);
    assert.equal(result.renamed, false);
  });
}

test('atomically replaces only the canonical server fixture', async () => {
  const result = await run();
  const target = new URL('../fixtures/catalog/tmdb.json', import.meta.url).href;
  assert.equal(result.exitCode, 0);
  assert.equal(result.writeTarget, `${target}.tmp`);
  assert.equal(result.renameSource, `${target}.tmp`);
  assert.equal(result.renameTarget, target);
});

test('preserves absent season ratings and votes as null and both episode identities', async () => {
  const result = await run();
  assert.equal(result.exitCode, 0);
  const { items } = JSON.parse(result.serialized);
  assert.equal(items['beverly-hillbillies-s1'].rating, null);
  assert.equal(items['beverly-hillbillies-s1'].votes, null);
  assert.equal(items['beverly-hillbillies-s1-e1'].title, 'The Clampetts Strike Oil');
  assert.equal(items['beverly-hillbillies-s1-e1'].date, '1962-09-26');
  assert.equal(items['beverly-hillbillies-s1-e2'].title, 'Getting Settled');
  assert.equal(items['beverly-hillbillies-s1-e2'].date, '1962-10-03');
});

test('rejects credentials in metadata without writing or revealing the secret', async () => {
  const result = await run(d => ({ ...d, overview: 'test-only-token' }));
  assert.equal(result.exitCode, 1);
  assert.equal(result.serialized, undefined);
  assert.equal(result.renamed, false);
  assert.deepEqual(result.errors, ['TMDB import failed; existing fixture preserved.']);
});
