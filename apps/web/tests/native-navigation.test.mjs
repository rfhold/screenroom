import { test } from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createServer } from 'vite';
import { chromium } from '@playwright/test';

const fixture = `
import { writable } from 'svelte/store';
export const records = new Map();
export let libraries = [];
export let capabilities;
export const settingsWarning = writable('');
export const configure = (l, c) => { libraries = l; capabilities = c; };
export const scope = (library = 'all', source_id = '') => ({ library_id: library === 'all' ? '' : library, source_id });
export const failure = e => String(e?.message || e);
export const credit = x => x;
export const allCredits = async () => [];
export const acquireArtwork = () => ({ promise: Promise.resolve(null), release() {} });
let settings = { scope: {}, source: 'all', types: { movie: true, series: true, episode: true } };
export const loadSettings = async () => settings;
export const saveSettings = async change => { settings = { ...settings, ...change }; };
const title = (id, kind = 'Movie', parent) => ({ id, title: id, kind, parent, year: '2026', meta: '', overview: '', cast: [], crew: [], providers: [{ provider: 'IMDb', status: 'source_supplied', url: 'https://www.imdb.com/title/tt1234567/' }], colors: ['#123', '#456'], rights: { label: 'Test' } });
const catalog = [...Array.from({ length: 24 }, (_, i) => title('Movie-' + i)), title('Show', 'Series'), title('Season', 'Season', 'Show'), title('Episode', 'Episode', 'Season')];
export const item = value => { records.set(value.id, value); return value; };
export const getItem = async (id, context) => { window.calls.push(['get', id, context]); if (window.slowRead) await new Promise(resolve => { window.releaseRead = resolve; }); return catalog.find(x => x.id === id); };
const api = {
 list_libraries: async () => ({ libraries: [{ id: 'opaque-library', title: 'My library' }], next_page_token: '' }),
 get_search_capabilities: async () => ({ sources: ['all'], types: ['Movie', 'Series', 'Episode'], methods: ['text', 'fuzzy', 'semantic'], maximum_query_length: 160 }),
 get_home: async () => [{ title: 'Recent', items: catalog.slice(0, 3) }],
 list_library_items: async (_context, page) => ({ items: catalog.slice(page.token ? 12 : 0, page.token ? 24 : 12), next_page_token: page.token ? '' : 'next' }),
 list_children: async id => ({ items: catalog.filter(x => x.parent === id), next_page_token: '' }),
 search: async request => { window.calls.push(['search', request]); return { items: catalog.slice(request.page.token ? 12 : 0, request.page.token ? 24 : 12), next_page_token: request.page.token ? '' : 'next', effective_methods: { text: true } }; },
 create_playback_session: async request => { window.calls.push(['create', request]); throw new Error('Owned fixture unavailable'); },
 cancel_all() {}, invalidate_reads() {}, free() {}
};
export const client = async () => api;
export const newClient = async () => api;
`;

for (const viewport of [{ width: 1440, height: 900 }, { width: 390, height: 844 }]) test(`native Kit routes, history snapshots and ephemeral playback ${viewport.width}`, { timeout: 90_000 }, async t => {
  const keys = ['SCREENROOM_WEB_OPERATOR_TARGET', 'SCREENROOM_WEB_OPERATOR_ORIGIN'];
  const environment = keys.map(key => [key, process.env[key]]);
  for (const key of keys) delete process.env[key];
  t.after(() => { for (const [key, value] of environment) { if (value === undefined) delete process.env[key]; else process.env[key] = value; } });
  const root = fileURLToPath(new URL('..', import.meta.url));
  const server = await createServer({
    root, server: { host: '127.0.0.1', port: 0, strictPort: false },
    plugins: [{ name: 'owned-navigation-fixture', enforce: 'pre', load(id) {
      if (id.endsWith('/src/lib/core/client.ts')) return fixture;
    } }],
  });
  await server.listen();
  t.after(() => server.close());
  const origin = `http://127.0.0.1:${server.httpServer.address().port}`;
  const browser = await chromium.launch({ headless: true });
  t.after(() => browser.close());
  const page = await browser.newPage({ viewport });
  await page.addInitScript(() => { window.calls = []; });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  for (const legacy of ['/?library=all', '/?screen=home&library=all']) {
    await page.goto(origin + legacy);
    await page.waitForURL(origin + '/');
    await page.getByRole('heading', { name: 'Home', exact: true }).waitFor();
    assert.equal(await page.getByText('Loading Screenroom…', { exact: true }).count(), 0);
  }
  await page.goto(`${origin}/?screen=browse&library=opaque-library&title=Movie-0&source_id=source`);
  await page.getByRole('heading', { name: 'Movie-0', exact: true }).waitFor();
  assert.match(page.url(), /\/titles\/Movie-0\?/);
  assert.equal(await page.getByText('source_supplied').count(), 0);
  await page.getByRole('button', { name: 'Play', exact: true }).click();
  await page.waitForURL('**/play/Movie-0?**');
  await page.waitForFunction(() => window.calls.some(x => x[0] === 'create'));
  assert.equal(await page.evaluate(() => window.calls.filter(x => x[0] === 'create').length), 1);
  assert.deepEqual(await page.evaluate(() => window.calls.find(x => x[0] === 'create')[1].scope), { source_id: 'source', library_id: 'opaque-library' });
  await page.locator('dialog').evaluate(dialog => { const a = document.createElement('a'); a.href = location.href; a.textContent = 'Current route'; dialog.append(a); });
  await page.getByRole('link', { name: 'Current route' }).click();
  assert.equal(await page.getByRole('dialog').count(), 1);
  assert.equal(await page.evaluate(() => window.calls.filter(x => x[0] === 'create').length), 1);
  await page.evaluate(() => { window.slowRead = true; });
  await page.goBack();
  await page.waitForFunction(() => !!window.releaseRead);
  assert.equal(await page.getByRole('dialog').count(), 0);
  await page.evaluate(() => { window.slowRead = false; window.releaseRead(); });
  await page.getByRole('heading', { name: 'Movie-0', exact: true }).waitFor();
  await page.goForward();
  await page.getByRole('button', { name: 'Play', exact: true }).waitFor();
  assert.equal(await page.getByRole('dialog').count(), 0);
  assert.equal(await page.evaluate(() => window.calls.filter(x => x[0] === 'create').length), 1);
  await page.reload();
  await page.getByRole('button', { name: 'Play', exact: true }).waitFor();
  assert.equal(await page.evaluate(() => window.calls.length && window.calls.filter(x => x[0] === 'create').length), 0);
  await page.goto(`${origin}/titles/Season?library=opaque-library&source_id=source`);
  await page.getByRole('link', { name: /Episode/ }).last().waitFor();
  await page.reload();
  await page.getByRole('link', { name: /Episode/ }).last().waitFor();
  await page.getByRole('link', { name: /Episode/ }).last().click();
  for (const reload of [false, true]) {
    if (reload) await page.reload();
    await page.getByRole('heading', { name: 'Episode', exact: true }).waitFor();
    const breadcrumb = page.getByRole('navigation', { name: 'Breadcrumb', exact: true });
    for (const name of ['Show', 'Season']) {
      const url = new URL(await breadcrumb.getByRole('link', { name, exact: true }).getAttribute('href'), origin);
      assert.equal(url.pathname, `/titles/${name}`);
      assert.equal(url.searchParams.get('library'), 'opaque-library');
      assert.equal(url.searchParams.get('source_id'), 'source');
    }
  }
  await page.goto(`${origin}/libraries/missing`);
  await page.getByRole('heading', { name: 'Not found', exact: true }).waitFor();
  await page.goto(`${origin}/search?q=Movie&library=opaque-library&source_id=source&types=Movie`);
  await page.getByText(/(?:12|24) loaded in My library/).waitFor();
  await page.getByRole('button', { name: 'Load more', exact: true }).evaluateAll(buttons => buttons[0]?.click());
  await page.getByText('24 loaded in My library').waitFor();
  const last = page.getByRole('link', { name: /Movie-23/ });
  await last.scrollIntoViewIfNeeded();
  await last.focus();
  const scroll = await page.evaluate(() => scrollY);
  await last.click();
  await page.getByRole('heading', { name: 'Movie-23', exact: true }).waitFor();
  await page.goBack();
  await page.getByText('24 loaded in My library').waitFor();
  t.diagnostic(`restored scroll ${await page.evaluate(() => scrollY)}; expected ${scroll}`);
  await page.waitForFunction(y => Math.abs(scrollY - y) < 2, scroll);
  assert.equal(await last.evaluate(link => link === document.activeElement), true);
  assert.match(page.url(), /source_id=source/);
  const searches = await page.evaluate(() => window.calls.filter(x => x[0] === 'search').length);
  await page.evaluate(async () => { const { revokePlayback } = await import('/src/lib/core/playback-lifecycle.ts'); revokePlayback(); });
  await last.click();
  await page.getByRole('heading', { name: 'Movie-23', exact: true }).waitFor();
  await page.goBack();
  await page.waitForFunction(count => window.calls.filter(x => x[0] === 'search').length > count, searches);
  await page.getByRole('link', { name: 'Maintenance', exact: true }).click();
  await page.getByText('Maintenance is unavailable on this connection.').waitFor();
  await page.reload();
  await page.getByText('Maintenance is unavailable on this connection.').waitFor();
  assert.deepEqual(errors, []);
});
