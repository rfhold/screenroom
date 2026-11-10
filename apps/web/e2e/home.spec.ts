import { expect, test, type Page, type Response } from '@playwright/test';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { createHash } from 'node:crypto';
import { paginate } from './pagination';
import { serverVariant, duplicateLibraryLabels, requestLibrary, refreshedDetail, brandingUrl, requestScope, degradedSearch } from './rpc-variant';
import { isRpcPath, rpcRoute } from './rpc-paths';
const snapshot = JSON.parse(await readFile(new URL('../../../fixtures/catalog/tmdb.json', import.meta.url), 'utf8'));
type RequestRecord = { url: string; contentType: string; bytes?: number; sha256?: string };
const network = new WeakMap<Page, RequestRecord[]>();

const rows = ['Continue watching', 'Catalog'];
const menu = (page: Page) => page.getByRole('navigation', { name: 'Main menu', exact: true });
const crumbs = (page: Page) => page.getByRole('navigation', { name: 'Breadcrumb', exact: true });
function expectRpc(response: Response) {
  expect(response.request().method()).toBe('POST');
  expect(response.status()).toBe(200);
  expect(response.headers()['content-type']).toContain('application/grpc-web+proto');
}
async function openSearch(page: Page) { await menu(page).getByRole('link', { name: 'Search titles', exact: true }).click(); }
async function lexicalOnly(page: Page) {
  await searchSettings(page);
  await page.getByRole('checkbox', { name: /^Semantic/ }).uncheck();
  await searchSettings(page, false);
}
async function searchSettings(page: Page, open = true) {
  const button = page.getByRole('button', { name: 'Search settings', exact: true });
  if (await button.getAttribute('aria-expanded') !== String(open)) await button.click();
  await expect(button).toHaveAttribute('aria-expanded', String(open));
}
async function expand(page: Page) { await page.getByRole('button', { name: 'Expand sidebar', exact: true }).click(); }
async function metadataClock(page: Page) {
  await page.addInitScript(() => {
    const clock = { offset: 0, writes: 0 };
    Object.defineProperty(window, '__screenroomClock', { value: clock });
    const now = performance.now.bind(performance);
    Object.defineProperty(performance, 'now', { value: () => now() + clock.offset });
    const setItem = Storage.prototype.setItem;
    Storage.prototype.setItem = function (key, value) {
      if (key.startsWith('screenroom.settings.v1:')) clock.writes++;
      return setItem.call(this, key, value);
    };
  });
}
const advanceMetadataClock = (page: Page) => page.evaluate(() => { (window as typeof window & { __screenroomClock: { offset: number } }).__screenroomClock.offset += 70_000; });
async function capture(page: Page, name: string) {
  await expect(page.locator('.image-state[aria-label^="Loading "]')).toHaveCount(0);
  await page.evaluate(async () => {
    await document.fonts.ready;
    await Promise.all([...document.images].filter(image => image.getBoundingClientRect().top < innerHeight && image.getBoundingClientRect().bottom > 0).map(image => image.decode()));
  });
  if (!test.info().tags.includes('@happy-path')) return;
  const file = path.resolve('e2e/build/screenshots', `${name}.png`);
  await mkdir(path.dirname(file), { recursive: true });
  await page.screenshot({ path: file, fullPage: name.startsWith('search-settings-'), animations: 'disabled' });
  return file;
}

test.beforeEach(async ({ page }) => {
  await page.addInitScript(namespace => { if (!localStorage.getItem('screenroom.settingsNamespace')) localStorage.setItem('screenroom.settingsNamespace', namespace); }, `${process.env.SCREENROOM_E2E_SETTINGS_NAMESPACE || 'web-e2e'}-${crypto.randomUUID()}`);
  const errors: string[] = [];
  const forbidden: string[] = [];
  const requests: RequestRecord[] = [];
  network.set(page, requests);
  page.on('response', response => {
    const contentType = response.headers()['content-type'] ?? '';
    if (/grpc-web|wasm|image\//.test(contentType)) requests.push({ url: response.url(), contentType });
  });
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
  page.on('request', request => {
    const url = new URL(request.url());
    const endpoint = new URL(process.env.SCREENROOM_E2E_API_ENDPOINT || 'http://127.0.0.1:50051');
    const mock = url.origin === endpoint.origin && (isRpcPath(url.pathname) || /^\/artwork\/[0-9a-f-]{36}\/revisions\/[1-9][0-9]*\/(native|original)$/.test(url.pathname));
    const local = url.origin === 'http://127.0.0.1:4173' && (url.pathname === '/' || /^\/(src|node_modules|@fs|@id|@vite|_app)\//.test(url.pathname) || /^\/\.svelte-kit\/generated\/dev\/client\/(matchers|nodes\/[0-9]+)\.js$/.test(url.pathname));
    if (!mock && !local && url.protocol !== 'blob:') forbidden.push(request.url());
  });
  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Home', exact: true })).toBeVisible();
  await expect(page.getByRole('heading', { level: 2 })).toHaveText(rows);
  (page as Page & { errors: string[]; forbidden: string[] }).errors = errors;
  (page as Page & { forbidden: string[] }).forbidden = forbidden;
});

test('Rust WASM makes real gRPC-web calls and core-loaded artwork becomes blob images', { tag: '@regression' }, async ({ page }, info) => {
  const requests: RequestRecord[] = [];
  page.on('response', response => {
    const contentType = response.headers()['content-type'] ?? '';
    if (/grpc-web|wasm|image\//.test(contentType)) requests.push({ url: response.url(), contentType });
  });
  await page.reload();
  await expect(page.getByRole('heading', { level: 2 })).toHaveText(rows);
  await expect(page.locator('.poster img').first()).toHaveAttribute('src', /^blob:/);
  await expand(page);
  await page.getByRole('radio', { name: 'Transcription', exact: true }).check();
  await expect.poll(() => page.evaluate(() => Object.entries(localStorage).some(([key, value]) => key.startsWith('screenroom.settings.v1:') && value.includes('transcription')))).toBe(true);
  await page.reload();
  await expand(page);
  await expect(page.getByRole('radio', { name: 'Transcription', exact: true })).toBeChecked();
  await openSearch(page);
  await page.getByRole('searchbox').fill('Bunny');
  await expect(page.getByRole('link', { name: /^Big Buck Bunny/ })).toBeVisible();
  const refreshedSearch = page.waitForResponse(response => response.url().endsWith('.SearchService/Search'));
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await refreshedSearch;
  await expect(page.getByRole('link', { name: /^Big Buck Bunny/ })).toBeVisible();
  await expect(page.getByRole('searchbox')).toHaveValue('Bunny');
  expect(requests.some(record => record.contentType.includes('application/wasm'))).toBe(true);
  expect(requests.some(record => record.url.endsWith('.BrowseService/GetHome') && record.contentType.includes('grpc-web'))).toBe(true);
  expect(requests.some(record => record.url.endsWith('.SearchService/Search') && record.contentType.includes('grpc-web'))).toBe(true);
  expect(requests.some(record => /\/artwork\//.test(record.url) && record.contentType.startsWith('image/'))).toBe(true);
  await info.attach('wasm-grpc-artwork-requests', { body: JSON.stringify(requests, null, 2), contentType: 'application/json' });
  await mkdir('e2e/build/requests', { recursive: true });
  await writeFile(`e2e/build/requests/wasm-grpc-artwork-${info.project.name}.json`, JSON.stringify(requests, null, 2));
});

test('one navigation budget spans metadata and final Home response without a late settings commit', { tag: '@regression' }, async ({ page }) => {
  await metadataClock(page);
  let libraries = 0, homes = 0;
  await page.route(rpcRoute('BrowseService/ListLibraries'), async route => {
    const response = await route.fetch(); libraries++;
    await advanceMetadataClock(page); await route.fulfill({ response });
  });
  await page.route(rpcRoute('BrowseService/GetHome'), async route => {
    const response = await route.fetch(); homes++;
    await advanceMetadataClock(page); await route.fulfill({ response });
  });
  await page.reload();
  await expect(page.getByRole('alert')).toContainText('Metadata operation deadline exceeded');
  expect({ libraries, homes }).toEqual({ libraries: 1, homes: 1 });
  expect(await page.evaluate(() => (window as typeof window & { __screenroomClock: { writes: number } }).__screenroomClock.writes)).toBe(0);
  await page.unroute(rpcRoute('BrowseService/ListLibraries'));
  await page.unroute(rpcRoute('BrowseService/GetHome'));
  await page.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(page.locator('.rows h2')).toHaveText(rows);
  expect(await page.evaluate(() => (window as typeof window & { __screenroomClock: { writes: number } }).__screenroomClock.writes)).toBe(1);
});

test('Search shares the navigation metadata deadline and Retry starts a fresh complete operation', { tag: '@regression' }, async ({ page }) => {
  await metadataClock(page);
  let libraries = 0, searches = 0;
  await page.route(rpcRoute('BrowseService/ListLibraries'), async route => {
    const response = await route.fetch(); libraries++;
    await advanceMetadataClock(page); await route.fulfill({ response });
  });
  await page.route(rpcRoute('SearchService/Search'), async route => {
    const response = await route.fetch(); searches++;
    await advanceMetadataClock(page); await route.fulfill({ response });
  });
  await page.goto('/?screen=search&library=all&q=Bunny&types=Movie&methods=text');
  await expect(page.getByRole('alert')).toContainText('Metadata operation deadline exceeded');
  expect({ libraries, searches }).toEqual({ libraries: 1, searches: 1 });
  await expect(page.getByRole('link', { name: /^Big Buck Bunny/ })).toHaveCount(0);
  await page.unroute(rpcRoute('BrowseService/ListLibraries'));
  await page.unroute(rpcRoute('SearchService/Search'));
  await page.getByRole('button', { name: 'Retry search', exact: true }).click();
  await expect(page.getByRole('link', { name: /^Big Buck Bunny/ })).toBeVisible();
});

test('initial unavailable RPC shows an honest error and Retry rereads the server', { tag: '@retry' }, async ({ page }) => {
  const unavailable = rpcRoute('BrowseService/GetHome');
  await page.route(unavailable, async route => {
    const trailer = Buffer.from('grpc-status: 14\r\ngrpc-message: mock unavailable\r\n');
    const header = Buffer.alloc(5); header[0] = 0x80; header.writeUInt32BE(trailer.length, 1);
    await route.fulfill({ status: 200, contentType: 'application/grpc-web+proto', headers: { 'access-control-allow-origin': 'http://127.0.0.1:4173', 'access-control-expose-headers': 'grpc-status,grpc-message' }, body: Buffer.concat([header, trailer]) });
  });
  await page.reload();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Unable to load Screenroom');
  await expect(page.getByRole('alert')).toContainText('unavailable');
  await expect(page.locator('.poster-row')).toHaveCount(0);
  await page.unroute(unavailable);
  await page.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(page.getByRole('heading', { level: 2 })).toHaveText(rows);
  const catalog = rpcRoute('CatalogService/GetCatalogInfo');
  await page.route(catalog, async route => {
    const trailer = Buffer.from('grpc-status: 14\r\ngrpc-message: catalog unavailable\r\n');
    const header = Buffer.alloc(5); header[0] = 0x80; header.writeUInt32BE(trailer.length, 1);
    await route.fulfill({ status: 200, contentType: 'application/grpc-web+proto', headers: { 'access-control-allow-origin': 'http://127.0.0.1:4173' }, body: Buffer.concat([header, trailer]) });
  });
  await expand(page);
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('catalog unavailable');
  await expect(page.getByRole('status').filter({ hasText: 'Refreshed from server.' })).toHaveCount(0);
  await page.unroute(catalog);
  const catalogBody = page.waitForResponse(response => response.url().endsWith('/GetCatalogInfo')).then(response => response.body());
  await page.getByRole('button', { name: 'Retry', exact: true }).click();
  const logoUrl = brandingUrl(await catalogBody);
  await expect(page.locator('.rows h2')).toHaveText(rows);
  await page.route(logoUrl, route => route.fulfill({ status: 200, contentType: 'image/png', body: Buffer.from('invalid image bytes') }));
  await page.reload();
  await page.locator('.credits > summary').click();
  await expect(page.locator('.credits .brand [role="img"]')).toHaveAttribute('aria-label', /artwork unavailable$/);
  await expect(page.locator('.credits .brand img')).toHaveCount(0);
  await page.unroute(logoUrl);
  await page.reload();
  await page.locator('.credits > summary').click();
  await expect(page.locator('.credits .brand img')).toHaveAttribute('src', /^blob:/);
  await page.locator('.credits .brand img').evaluate(image => (image as HTMLImageElement).decode());
});

test('server response variants drive libraries, capabilities, providers, ratings and refreshed notices', { tag: '@regression' }, async ({ page }) => {
  const variant = await serverVariant(page);
  await page.reload();
  await expect(menu(page).getByRole('link', { name: 'Cinema from server', exact: true })).toBeVisible();
  await menu(page).getByRole('link', { name: 'Cinema from server', exact: true }).click();
  await expect(page).toHaveURL(/library=movies/);
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Cinema from server');
  await page.getByRole('link', { name: /^Big Buck Bunny · Movie/ }).click();
  await expect(page.getByRole('link', { name: 'Archive: Verified; user rating 3.5/5', exact: true })).toHaveAttribute('href', 'https://example.invalid/catalog');
  await page.locator('.metadata-info summary').click();
  await expect(page.locator('.info-content')).toContainText('Vote count unavailable');
  await expect(page.locator('.info-content')).not.toContainText('TMDB');
  await expect(page.locator('.info-content')).not.toContainText('Jellyfin');
  await expect(page.getByRole('article')).toContainText('Server-specific provenance');
  await expect(page.getByRole('article')).toContainText('2030-01-03');
  await page.locator('.credits > summary').click();
  await expect(page.locator('.credits')).toContainText('Alternate server notice');
  variant.updateNotice('Refreshed server notice');
  await expand(page);
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(page.getByRole('status').filter({ hasText: 'Refreshed from server.' })).toBeVisible();
  await expect(page.locator('.credits')).toContainText('Refreshed server notice');
  await expect(page.locator('.credits')).not.toContainText('Alternate server notice');
  await openSearch(page);
  await searchSettings(page);
  await expect(page.getByRole('searchbox')).toHaveAttribute('maxlength', '37');
  await expect(page.locator('#catalog-source option')).toHaveCount(1);
  await expect(page.getByRole('checkbox')).toHaveCount(2);
  await expect(page.getByRole('checkbox', { name: 'Movies', exact: true })).toBeVisible();
  await expect(page.getByRole('checkbox', { name: 'Text (titles and metadata)', exact: true })).toBeVisible();
  await expect(page.locator('#search-settings')).toContainText('Server capability description');
});

test('search-result detail Refresh completes after real reads and reports failed reads', { tag: '@regression' }, async ({ page }, info) => {
  await openSearch(page);
  await page.getByRole('searchbox').fill('Bunny');
  await page.getByRole('link', { name: /^Big Buck Bunny · Movie/ }).click();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Big Buck Bunny');
  const context = page.url();
  const itemId = new URL(context).searchParams.get('title');
  await refreshedDetail(page);
  await expand(page);
  const reads = ['GetItem', 'ListCredits', 'GetCatalogInfo'].map(method => page.waitForResponse(response => response.url().endsWith(`/${method}`)));
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  const responses = await Promise.all(reads);
  for (const response of responses) {
    expectRpc(response);
    if (!response.url().endsWith('/GetCatalogInfo')) expect(requestLibrary(response.request().postDataBuffer()!, 1)).toBe(itemId);
  }
  await info.attach('detail-refresh-rpc-headers', { body: JSON.stringify(responses.map(response => ({ url: response.url(), method: response.request().method(), status: response.status(), contentType: response.headers()['content-type'], itemId: response.url().endsWith('/GetCatalogInfo') ? undefined : requestLibrary(response.request().postDataBuffer()!, 1) })), null, 2), contentType: 'application/json' });
  await expect(page.locator('p.status')).toHaveText('Refreshed from server.');
  await expect(page.locator('p.status')).not.toContainText('Refreshing');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Big Buck Bunny');
  await expect(page).toHaveURL(context);
  await expect(crumbs(page)).toContainText('Search');
  await expect(page.getByRole('article')).toContainText('Fresh detail metadata from RPC');
  await expect(page.locator('section[aria-label="Crew"] li > span')).toHaveText(Array(6).fill(/^Fresh crew role from RPC: /));
  await page.locator('.credits > summary').click();
  await expect(page.locator('.credits')).toContainText('Fresh catalog notice from RPC');
  const credits = rpcRoute('CatalogService/ListCredits');
  await page.route(credits, async route => {
    const trailer = Buffer.from('grpc-status: 14\r\ngrpc-message: detail credits unavailable\r\n');
    const header = Buffer.alloc(5); header[0] = 0x80; header.writeUInt32BE(trailer.length, 1);
    await route.fulfill({ status: 200, contentType: 'application/grpc-web+proto', body: Buffer.concat([header, trailer]) });
  });
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('detail credits unavailable');
  await expect(page.getByRole('status')).toHaveCount(0);
  await expect(page).toHaveURL(context);
  await page.unroute(credits);
  await page.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Big Buck Bunny');
  await expect(page.locator('section[aria-label="Crew"] li > span')).toHaveText(Array(6).fill(/^Fresh crew role from RPC: /));
  await expect(page.getByRole('alert')).toHaveCount(0);
  await expect(page.locator('p.status')).toHaveCount(0);
  await expect(page).toHaveURL(context);
});

test('duplicate server library labels preserve stable IDs through browse, search, Back and reload', { tag: '@regression' }, async ({ page }) => {
  await duplicateLibraryLabels(page);
  await page.reload();
  const choices = menu(page).getByRole('link', { name: 'Shared library', exact: true });
  await expect(choices).toHaveCount(2);
  await expect(choices.nth(0)).toHaveAttribute('href', /library=movies$/);
  await expect(choices.nth(1)).toHaveAttribute('href', /library=tv$/);
  const browse = () => page.waitForResponse(response => response.url().endsWith('/ListLibraryItems') && requestLibrary(response.request().postDataBuffer()!, 1) === 'tv');
  const firstBrowse = browse();
  await choices.nth(1).click();
  expectRpc(await firstBrowse);
  await expect(page).toHaveURL(/library=tv$/);
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Shared library');
  await expect(page.locator('.catalog .card')).toHaveCount(1);
  await expect(choices.nth(1)).toHaveAttribute('aria-current', 'page');
  await expect(choices.nth(0)).not.toHaveAttribute('aria-current', 'page');
  const reloadBrowse = browse();
  await page.reload();
  await reloadBrowse;
  await expect(page).toHaveURL(/library=tv$/);
  await openSearch(page);
  await expect(page).toHaveURL(/library=tv/);
  const search = page.waitForResponse(response => response.url().endsWith('/Search') && requestLibrary(response.request().postDataBuffer()!, 2) === 'tv');
  await page.getByRole('searchbox').fill('Beverly');
  expectRpc(await search);
  await expect(page.locator('[aria-label="Search results"] .card')).toHaveCount(1);
  await expect(page).toHaveURL(/library=tv.*q=Beverly/);
  const searchContext = page.url();
  const reloadSearch = page.waitForResponse(response => response.url().endsWith('/Search') && requestLibrary(response.request().postDataBuffer()!, 2) === 'tv');
  await page.reload();
  await reloadSearch;
  await expect(page).toHaveURL(searchContext);
  await expect(page.getByRole('searchbox')).toHaveValue('Beverly');
  await page.goBack();
  await expect(page).toHaveURL(/screen=browse&library=tv$/);
  await expect(page.locator('.catalog .card')).toHaveCount(1);
  await expect(crumbs(page).getByRole('link', { name: 'Shared library', exact: true })).toHaveAttribute('href', /library=tv$/);
});

test('local settings persist across reload and do not bleed into another namespace', { tag: '@regression' }, async ({ page, browser }) => {
  const original = await page.evaluate(() => localStorage.getItem('screenroom.settingsNamespace'));
  await expand(page);
  await page.getByRole('radio', { name: 'Library tracks', exact: true }).check();
  await expect.poll(() => page.evaluate(() => Object.entries(localStorage).some(([key, value]) => key.startsWith('screenroom.settings.v1:') && value.includes('library_tracks')))).toBe(true);
  await page.evaluate(namespace => localStorage.setItem('screenroom.settingsNamespace', `${namespace}-isolated`), original);
  await page.reload();
  await expand(page);
  await expect(page.getByRole('radio', { name: 'Off', exact: true })).toBeChecked();
  await page.evaluate(namespace => localStorage.setItem('screenroom.settingsNamespace', namespace!), original);
  await page.reload();
  await expand(page);
  await expect(page.getByRole('radio', { name: 'Library tracks', exact: true })).toBeChecked();
  const state = await page.context().storageState();
  const relaunched = await browser.newContext({ storageState: state });
  try {
    const reopened = await relaunched.newPage();
    await reopened.goto('http://127.0.0.1:4173');
    await expand(reopened);
    await expect(reopened.getByRole('radio', { name: 'Library tracks', exact: true })).toBeChecked();
  } finally { await relaunched.close(); }
  expect(network.get(page)!.some(record => record.url.includes('PreferencesService'))).toBe(false);
});

test('superseded routes cannot overwrite newer scope, including queued settings commits', { tag: '@regression' }, async ({ page }) => {
  await page.goto('/?screen=browse&library=movies');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Movies');
  let release!: () => void;
  let held!: () => void;
  const gate = new Promise<void>(resolve => release = resolve);
  const pending = new Promise<void>(resolve => held = resolve);
  await page.route(rpcRoute('CatalogService/GetItem'), async route => {
    const response = await route.fetch(); held(); await gate; await route.fulfill({ response });
  });
  try {
    await page.getByRole('link', { name: /^Big Buck Bunny · Movie/ }).click();
    await pending;
    await page.evaluate(() => { const link = document.createElement('a'); link.href = '/?screen=browse&library=tv&source_id=mock-jellyfin'; link.textContent = 'Newer scoped route'; document.body.append(link); });
    await page.getByRole('link', { name: 'Newer scoped route' }).click();
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('TV Shows');
    const completed = page.waitForResponse(response => response.url().endsWith('/GetItem'));
    release(); await (await completed).finished();
    await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
    await page.unroute(rpcRoute('CatalogService/GetItem'));
    const guarded = await page.evaluate(async () => {
      const modulePath = '/src/lib/core/client.ts';
      const settings = await import(modulePath);
      let checks = 0;
      await settings.saveSettings({ scope: { source_id: '', library_id: 'movies' } }, () => ++checks === 1);
      return { checks, scope: (await settings.loadSettings()).scope };
    });
    expect(guarded).toEqual({ checks: 2, scope: { source_id: 'mock-jellyfin', library_id: 'tv' } });
    await expect(page.getByRole('heading', { level: 1 })).toHaveText('TV Shows');
    await page.goto('/?screen=search');
    await expect(page.getByRole('searchbox')).toHaveAttribute('aria-label', 'Search TV Shows');
    await expect(crumbs(page).getByRole('link', { name: 'TV Shows', exact: true })).toHaveAttribute('href', /source_id=mock-jellyfin/);
  } finally { release(); await page.unroute(rpcRoute('CatalogService/GetItem')); }
});

test('unavailable local persistence leaves healthy catalog reads and current settings usable', { tag: '@regression' }, async ({ page }) => {
  await page.addInitScript(() => {
    const original = Storage.prototype.setItem;
    Storage.prototype.setItem = function (key, value) {
      if (key.startsWith('screenroom.settings.v1:')) throw new DOMException('Storage quota exceeded', 'QuotaExceededError');
      return original.call(this, key, value);
    };
  });
  await page.reload();
  const warning = page.getByText('Settings cannot be saved on this device. Current choices remain available until this page is closed.', { exact: true });
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Home');
  await expect(warning).toBeVisible();
  await expand(page);
  await page.getByRole('radio', { name: 'Library tracks', exact: true }).check();
  await menu(page).getByRole('link', { name: 'Movies', exact: true }).click();
  await expect(page.locator('.catalog > a')).toHaveCount(8);
  await expect(page.getByRole('radio', { name: 'Library tracks', exact: true })).toBeChecked();
  await openSearch(page); await searchSettings(page);
  await page.getByRole('checkbox', { name: 'Episodes', exact: true }).check();
  await page.getByRole('checkbox', { name: /^Semantic/ }).uncheck();
  await page.getByRole('searchbox').fill('Bunny');
  await expect(page.getByRole('link', { name: /^Big Buck Bunny/ })).toBeVisible();
  await page.getByRole('link', { name: /^Big Buck Bunny/ }).click();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Big Buck Bunny');
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(page.getByText('Refreshed from server.', { exact: true })).toBeVisible();
  await menu(page).getByRole('link', { name: 'Home', exact: true }).click();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Home');
  await openSearch(page); await searchSettings(page);
  await expect(page.getByRole('checkbox', { name: 'Episodes', exact: true })).toBeChecked();
  await expect(page.getByRole('checkbox', { name: /^Semantic/ })).not.toBeChecked();
  await expect(warning).toBeVisible();
  const invalid = await page.evaluate(async () => {
    const modulePath = '/src/lib/core/client.ts';
    const settings = await import(modulePath);
    await settings.saveSettings({});
    const before = JSON.stringify(await settings.loadSettings());
    let rejected = false;
    try { await settings.saveSettings({ types: { movie: false, series: false, episode: false } }); }
    catch { rejected = true; }
    return { rejected, unchanged: before === JSON.stringify(await settings.loadSettings()) };
  });
  expect(invalid).toEqual({ rejected: true, unchanged: true });
  await expect(page.getByRole('alert')).toHaveCount(0);
  await expect(page.getByRole('status').filter({ hasText: /^Saved/ })).toHaveCount(0);
});

test('settings restore scope, provider, types and methods in a relaunched context and reject corrupt data', { tag: '@regression' }, async ({ page, browser }) => {
  await menu(page).getByRole('link', { name: 'Movies', exact: true }).click();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Movies');
  await openSearch(page);
  await searchSettings(page);
  await page.getByRole('combobox', { name: 'Catalog', exact: true }).selectOption('tmdb');
  await page.getByRole('checkbox', { name: 'Episodes', exact: true }).check();
  await page.getByRole('checkbox', { name: /^Title only/ }).check();
  await expect.poll(() => page.evaluate(() => Object.entries(localStorage).some(([key, value]) => key.startsWith('screenroom.settings.v1:') && value.includes('"title_only":true') && value.includes('"episode":true') && value.includes('"library_id":"movies"') && value.includes('"source":"tmdb"')))).toBe(true);
  const relaunched = await browser.newContext({ storageState: await page.context().storageState() });
  try {
    const reopened = await relaunched.newPage();
    await reopened.goto('http://127.0.0.1:4173/?screen=search');
    await searchSettings(reopened);
    await expect(reopened.getByRole('searchbox')).toHaveAttribute('aria-label', 'Search Movies');
    await expect(reopened.getByRole('combobox', { name: 'Catalog', exact: true })).toHaveValue('tmdb');
    await expect(reopened.getByRole('checkbox', { name: 'Episodes', exact: true })).toBeChecked();
    await expect(reopened.getByRole('checkbox', { name: /^Title only/ })).toBeChecked();
    await expect(reopened.getByRole('checkbox', { name: /^Semantic/ })).not.toBeChecked();
    await reopened.evaluate(() => { for (const key of Object.keys(localStorage)) if (key.startsWith('screenroom.settings.v1:')) localStorage.setItem(key, '{invalid json'); });
    await reopened.goto('http://127.0.0.1:4173/?screen=search');
    await searchSettings(reopened);
    await expect(reopened.getByRole('combobox', { name: 'Catalog', exact: true })).toHaveValue('all');
    await expect(reopened.getByRole('checkbox', { name: /^Title only/ })).not.toBeChecked();
  } finally { await relaunched.close(); }
});

test('full artwork attribution is fetched only when opened, includes all source records, and retries atomically', { tag: '@retry' }, async ({ page }) => {
  expect(network.get(page)!.some(record => record.url.endsWith('/ListArtwork'))).toBe(false);
  const routePath = rpcRoute('CatalogService/ListArtwork');
  await page.route(routePath, route => {
    const trailer = Buffer.from('grpc-status: 14\r\ngrpc-message: artwork credits unavailable\r\n');
    const header = Buffer.alloc(5); header[0] = 0x80; header.writeUInt32BE(trailer.length, 1);
    return route.fulfill({ status: 200, contentType: 'application/grpc-web+proto', body: Buffer.concat([header, trailer]) });
  });
  await page.locator('.credits > summary').click();
  await expect(page.locator('.credits [role="alert"]')).toContainText('artwork credits unavailable');
  await expect(page.locator('.credits .content > ul').first().locator(':scope > li')).toHaveCount(0);
  await page.unroute(routePath);
  await page.getByRole('button', { name: 'Retry credits', exact: true }).click();
  await expect(page.locator('.credits .content > ul').first().locator(':scope > li')).toHaveCount(21);
  await expect(page.locator('.credits section[data-section]')).toHaveCount(7);
  await expect(page.locator('.credits [role="alert"]')).toHaveCount(0);
});

test('search displays actual effective methods and explicit semantic degradation', { tag: '@retry' }, async ({ page }) => {
  await degradedSearch(page);
  await openSearch(page);
  await page.getByRole('searchbox').fill('Bunny');
  await expect(page.getByRole('link', { name: /^Big Buck Bunny/ })).toBeVisible();
  await expect(page.getByRole('status')).toHaveText('Effective methods: text, fuzzy. embedding_unavailable');
  await searchSettings(page);
  await expect(page.getByRole('checkbox', { name: /^Semantic/ })).toBeChecked();
  await page.unroute(rpcRoute('SearchService/Search'));
  await page.getByRole('searchbox').fill('Sintel');
  await expect(page.getByRole('status')).toHaveText('Effective methods: text, fuzzy, semantic.');
});

test('installation scope survives hierarchy, detail credits, paging and navigation', { tag: '@regression' }, async ({ page }) => {
  const contexts: { method: string; scope: { source_id: string; library_id: string } }[] = [];
  page.on('request', request => {
    const method = request.url().split('/').at(-1)!;
    if (['GetItem', 'ListChildren', 'ListCredits'].includes(method)) contexts.push({ method, scope: requestScope(request.postDataBuffer()!, method === 'GetItem' ? 2 : 3) });
  });
  const pages = await paginate(page, 'CatalogService/ListChildren');
  await page.goto('/?screen=browse&library=tv&series=beverly-hillbillies&season=beverly-hillbillies-s1&source_id=mock-jellyfin');
  await expect(page.locator('.catalog > a')).toHaveCount(2);
  expect(pages.calls).toEqual(['', pages.token]);
  await page.getByRole('link', { name: /^Getting Settled · Episode/ }).click();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Getting Settled');
  await expect(page).toHaveURL(/source_id=mock-jellyfin/);
  await expect(crumbs(page).getByRole('link', { name: 'TV Shows', exact: true })).toHaveAttribute('href', /source_id=mock-jellyfin/);
  expect(contexts.some(entry => entry.method === 'ListCredits')).toBe(true);
  expect(contexts.every(entry => entry.scope.source_id === 'mock-jellyfin' && entry.scope.library_id === 'tv')).toBe(true);
});

test('an older delayed search cannot replace a newer query result', { tag: '@regression' }, async ({ page }) => {
  await openSearch(page);
  await lexicalOnly(page);
  let release!: () => void;
  let arrived!: () => void;
  const held = new Promise<void>(resolve => release = resolve);
  const started = new Promise<void>(resolve => arrived = resolve);
  let first = true;
  await page.route(rpcRoute('SearchService/Search'), async route => {
    if (!first) { await route.continue(); return; }
    first = false;
    const response = await route.fetch();
    arrived();
    await held;
    await route.fulfill({ response }).catch(() => {});
  });
  await page.getByRole('searchbox').fill('Bunny');
  await started;
  await page.getByRole('searchbox').fill('Sintel');
  await expect(page.getByRole('link', { name: /^Sintel · Movie/ })).toBeVisible();
  release();
  await expect(page.locator('.search-toolbar')).toHaveAttribute('aria-busy', 'false');
  await expect(page.getByRole('link', { name: /^Big Buck Bunny/ })).toHaveCount(0);
  await expect(page.getByRole('searchbox')).toHaveValue('Sintel');
});
test('pagination collects library pages in order without duplicate IDs', { tag: '@regression' }, async ({ page }) => {
  const pages = await paginate(page, 'BrowseService/ListLibraryItems');
  await menu(page).getByRole('link', { name: 'Movies', exact: true }).click();
  await expect(page.locator('.catalog > a')).toHaveCount(8);
  expect(pages.count()).toBe(8);
  expect(pages.calls).toEqual(['', pages.token]);
  await expect(page.locator('.catalog > a').first()).toHaveAccessibleName(/^Big Buck Bunny/);
});

test('pagination collects child pages and shows the second episode', { tag: '@regression' }, async ({ page }) => {
  const pages = await paginate(page, 'CatalogService/ListChildren');
  await page.goto('/?screen=browse&library=tv&series=beverly-hillbillies&season=beverly-hillbillies-s1');
  await expect(page.locator('.catalog > a')).toHaveCount(2);
  await expect(page.getByRole('link', { name: /^Getting Settled · Episode/ })).toBeVisible();
  expect(pages.calls).toEqual(['', pages.token]);
});

test('pagination collects search pages before reporting the unique result count', { tag: '@regression' }, async ({ page }) => {
  const pages = await paginate(page, 'SearchService/Search');
  await openSearch(page);
  await page.getByRole('searchbox').fill('e');
  await expect(page.locator('.search-toolbar')).toHaveAttribute('aria-busy', 'false');
  await expect.poll(() => page.locator('[aria-label="Search results"] > a').count()).toBeGreaterThan(1);
  await expect(page.locator('[aria-label="Search results"] > a')).toHaveCount(pages.count());
  await expect(page.locator('.result-status')).toHaveText(`${pages.count()} results in All libraries`);
  expect(pages.calls).toEqual(['', pages.token]);
});

test('pagination includes second-page titles in the credits disclosure', { tag: '@regression' }, async ({ page }) => {
  const pages = await paginate(page, 'BrowseService/ListLibraryItems');
  await page.locator('.credits > summary').click();
  await expect.poll(() => page.locator('.credits .content > ul').last().locator(':scope > li').count()).toBeGreaterThan(1);
  await expect(page.locator('.credits .content > ul').last().locator(':scope > li')).toHaveCount(pages.count());
  expect(pages.calls).toEqual(['', pages.token]);
});

test('pagination rejects a repeated continuation instead of looping', { tag: '@regression' }, async ({ page }) => {
  const pages = await paginate(page, 'BrowseService/ListLibraryItems', true);
  await menu(page).getByRole('link', { name: 'Movies', exact: true }).click();
  await expect(page.getByRole('alert')).toContainText('Server repeated a pagination token.');
  expect(pages.calls).toEqual(['', pages.token]);
});

test('pagination collects all detail credits through the shared collector', { tag: '@regression' }, async ({ page }) => {
  const pages = await paginate(page, 'CatalogService/ListCredits');
  await page.goto('/?screen=browse&library=movies&title=big-buck-bunny');
  const people = page.locator('section[aria-label="Cast"] li, section[aria-label="Crew"] li');
  await expect.poll(() => people.count()).toBeGreaterThan(1);
  await expect(people).toHaveCount(pages.count());
  expect(pages.calls).toEqual(['', pages.token]);
});

test.afterEach(async ({ page }) => {
  expect((page as Page & { errors: string[] }).errors).toEqual([]);
  expect((page as Page & { forbidden: string[] }).forbidden).toEqual([]);
  await expect(page.locator('audio, video, dialog')).toHaveCount(0);
  await expect(page.getByRole('button', { name: /^(Play|Resume|Back|Return)( |$)/ })).toHaveCount(0);
  await expect(page.getByRole('banner')).toHaveCount(0);
  expect(await page.locator('.shell').innerText()).not.toMatch(/The Quiet Tide|Harbor House|IMDb|Rotten Tomatoes/);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
});

test('showcase captures share canonical navigation, state and viewport framing', { tag: '@happy-path' }, async ({ page }, info) => {
  async function topCapture(flow: string) {
    await page.evaluate(() => { scrollTo(0, 0); (document.activeElement as HTMLElement)?.blur(); });
    await expect.poll(() => page.evaluate(() => scrollY)).toBe(0);
    await capture(page, `${flow}-${info.project.name}`);
  }
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Home');
  await expect(page.getByRole('button', { name: 'Expand sidebar' })).toHaveAttribute('aria-expanded', 'false');
  await topCapture('home');
  await expect(page.locator('.poster img').first()).toHaveAttribute('src', /^blob:/);
  await expand(page);
  await expect(page.getByRole('radio', { name: 'Off', exact: true })).toBeChecked();
  const expanded = (await menu(page).boundingBox())!;
  expect(expanded.width).toBeGreaterThan(200);
  await topCapture('sidebar');
  await page.getByRole('button', { name: 'Collapse sidebar' }).click();
  await menu(page).getByRole('link', { name: 'Movies', exact: true }).click();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Movies');
  await expect(page.locator('.catalog > a')).toHaveCount(8);
  await topCapture('library');
  await page.getByRole('link', { name: /^Big Buck Bunny · Movie/ }).click();
  await expect(crumbs(page).getByRole('link')).toHaveText(['Home', 'Movies', 'Big Buck Bunny']);
  await expect(page).toHaveURL(/screen=browse&library=movies&title=big-buck-bunny/);
  await expect(page.locator('.metadata-info')).not.toHaveAttribute('open');
  const detail = (await page.locator('.detail').boundingBox())!;
  const poster = (await page.locator('.detail .art').boundingBox())!;
  expect(detail.width).toBeLessThanOrEqual(1120);
  expect(poster.width).toBe(detail.width >= 640 ? 200 : 160);
  expect(poster.height).toBe(detail.width >= 640 ? 300 : 240);
  await expect(page.getByRole('heading', { name: 'Big Buck Bunny', exact: true })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Crew', exact: true })).toBeAttached();
  await topCapture('detail');
  await menu(page).getByRole('link', { name: 'Home', exact: true }).click();
  await openSearch(page);
  await page.getByRole('searchbox').fill('Bunny');
  await expect(page.locator('.search-toolbar')).toHaveAttribute('aria-busy', 'false');
  await expect(page.getByRole('link', { name: /^Big Buck Bunny/ })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Search settings', exact: true })).toHaveAttribute('aria-expanded', 'false');
  await expect(crumbs(page).getByRole('link')).toHaveText(['Home', 'All libraries', 'Search']);
  expect(new URL(page.url()).searchParams.has('source')).toBe(false);
  await searchSettings(page);
  await expect(page.getByRole('combobox', { name: 'Catalog', exact: true })).toHaveValue('all');
  await expect(page.getByRole('checkbox', { name: 'Movies', exact: true })).toBeChecked();
  await expect(page.getByRole('checkbox', { name: 'Shows', exact: true })).toBeChecked();
  await expect(page.getByRole('checkbox', { name: 'Episodes', exact: true })).not.toBeChecked();
  await searchSettings(page, false);
  await expect(page.locator('.result-status')).toHaveText(/\d+ results? in All libraries/);
  await topCapture('search');
  await page.locator('.credits > summary').click();
  const logo = page.locator('.credits .brand img');
  await expect(logo).toHaveAttribute('src', /^blob:/);
  await logo.evaluate(image => (image as HTMLImageElement).decode());
  expect(await logo.evaluate(image => (image as HTMLImageElement).naturalWidth)).toBeGreaterThan(0);
  const requests = network.get(page)!;
  const logoBytes = Buffer.from(await logo.evaluate(async image => [...new Uint8Array(await (await fetch((image as HTMLImageElement).src)).arrayBuffer())]));
  const expectedLogo = await readFile(new URL('../../../fixtures/catalog/artwork/tmdb.png', import.meta.url));
  expect(logoBytes.equals(expectedLogo)).toBe(true);
  requests.push({ url: (await logo.getAttribute('src'))!, contentType: 'image/png', bytes: logoBytes.length, sha256: createHash('sha256').update(logoBytes).digest('hex') });
  expect(requests.some(record => record.contentType.includes('application/wasm'))).toBe(true);
  for (const method of ['BrowseService/GetHome', 'BrowseService/ListLibraries', 'BrowseService/ListLibraryItems', 'CatalogService/GetItem', 'CatalogService/ListCredits', 'CatalogService/GetCatalogInfo', 'CatalogService/ListArtwork', 'SearchService/Search']) {
    expect(requests.some(record => record.url.endsWith(`.${method}`) && record.contentType.includes('grpc-web')), method).toBe(true);
  }
  expect(requests.some(record => /\/artwork\//.test(record.url) && record.contentType.startsWith('image/'))).toBe(true);
  expect(requests.some(record => record.url.includes('/artwork/') && record.contentType === 'image/png')).toBe(true);
  await mkdir('e2e/build/requests', { recursive: true });
  const body = JSON.stringify(requests, null, 2);
  await writeFile(`e2e/build/requests/happy-path-${info.project.name}.json`, body);
  await info.attach(`happy-path-requests-${info.project.name}`, { body, contentType: 'application/json' });
});

test('details use available-width header breakpoints and full-width wrapping people sections', { tag: '@regression' }, async ({ page }) => {
  await menu(page).getByRole('link', { name: 'Movies', exact: true }).click();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Movies');
  await page.getByRole('link', { name: /^Big Buck Bunny · Movie/ }).click();
  const original = page.viewportSize()!;
  for (const width of [320, 390, 767, 768, 1440]) {
    await page.setViewportSize({ width, height: original.height });
    const article = (await page.locator('.detail').boundingBox())!;
    const header = (await page.locator('.detail-header').boundingBox())!;
    const art = (await page.locator('.detail .art').boundingBox())!;
    const information = (await page.locator('.information').boundingBox())!;
    expect(article.width).toBeLessThanOrEqual(1120);
    const credits = (await page.locator('.credits').boundingBox())!;
    expect(credits.x).toBe(article.x);
    expect(credits.width).toBe(article.width);
    expect(art.width).toBe(article.width >= 640 ? 200 : 160);
    expect(art.height).toBe(article.width >= 640 ? 300 : 240);
    if (article.width >= 640) expect(Math.abs(art.y - information.y)).toBeLessThan(1);
    else expect(information.y).toBeGreaterThanOrEqual(art.y + art.height);
    for (const section of await page.locator('.detail > section').all()) {
      const bounds = (await section.boundingBox())!;
      expect(bounds.y).toBeGreaterThanOrEqual(header.y + header.height);
      expect(bounds.x).toBe(article.x);
      expect(bounds.width).toBe(article.width);
    }
    const crew = page.getByRole('region', { name: 'Crew', exact: true });
    const cards = await crew.getByRole('listitem').all();
    const first = (await cards[0].boundingBox())!;
    const second = (await cards[1].boundingBox())!;
    expect(second.y).toBe(first.y);
    expect(first.width).toBeGreaterThanOrEqual(128);
    for (const card of cards) {
      const portrait = (await card.locator('.portrait, .initials').boundingBox())!;
      expect(Math.abs(portrait.width - portrait.height)).toBeLessThan(1);
      const name = card.locator('strong');
      expect(await name.evaluate(n => n.scrollHeight <= n.clientHeight)).toBe(true);
    }
    const badges = page.locator('.metadata .badge').last();
    const jellyfin = (await badges.boundingBox())!;
    const info = (await page.locator('.metadata-info summary').boundingBox())!;
    if (info.y === jellyfin.y) expect(info.x - jellyfin.x - jellyfin.width).toBeLessThanOrEqual(9);
    expect(info.width).toBe(44);
    expect(info.height).toBe(44);
    const metadata = (await page.locator('.metadata').boundingBox())!;
    const synopsis = (await page.locator('.information > p').last().boundingBox())!;
    expect(synopsis.y).toBeGreaterThanOrEqual(metadata.y + metadata.height);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  }
  await page.setViewportSize(original);
});

test('home rows, linked title pages and keyboard access', { tag: '@regression' }, async ({ page }) => {
  await expect(page).toHaveTitle('Home · Screenroom');
  await expect(page.getByRole('heading', { level: 2 })).toHaveText(rows);
  await page.keyboard.press('Tab');
  await expect(page.getByRole('link', { name: 'Skip to content' })).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(page.getByRole('main')).toBeFocused();
  const card = page.getByRole('region', { name: rows[0], exact: true }).getByRole('link', { name: /^Big Buck Bunny/ });
  await expect(card).toHaveAttribute('href', '/?title=big-buck-bunny');
  await card.click();
  await expect(page).toHaveURL(/title=big-buck-bunny/);
  await expect(page.getByRole('article', { name: 'Big Buck Bunny details' })).toContainText('Movie · 2008');
  await expect(crumbs(page).getByRole('link')).toHaveText(['Home', 'Big Buck Bunny']);
  await page.reload();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Big Buck Bunny');
  await expect(page.getByRole('article', { name: 'Big Buck Bunny details' })).toContainText('Movie · 2008');
  await page.goBack();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Home');
  await page.goForward();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Big Buck Bunny');
  await crumbs(page).getByRole('link', { name: 'Home', exact: true }).click();
  await page.getByRole('region', { name: rows[1], exact: true }).getByRole('link', { name: /^Big Buck Bunny/ }).click();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Big Buck Bunny');
  await page.goBack();
  await page.getByRole('region', { name: rows[0], exact: true }).getByRole('link', { name: / · Episode · / }).first().click();
  await expect(page.getByRole('article')).toContainText('Episode · 1962');
  await menu(page).getByRole('link', { name: 'Home', exact: true }).click();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Home');
  await expect(page.getByRole('heading', { level: 2 })).toHaveText(rows);
  await page.evaluate(() => { scrollTo(0, 0); (document.activeElement as HTMLElement)?.blur(); });
});

test('sidebar expands in document flow, stays consistent across screens and offers preferences/actions', { tag: '@regression' }, async ({ page }, info) => {
  const main = page.getByRole('main');
  const before = (await main.boundingBox())!;
  const toggleBefore = (await page.getByRole('button', { name: 'Expand sidebar' }).boundingBox())!;
  await expand(page);
  const toggle = page.getByRole('button', { name: 'Collapse sidebar' });
  await expect(toggle).toBeFocused();
  await expect(toggle).toHaveAttribute('aria-expanded', 'true');
  const after = (await main.boundingBox())!;
  if (info.project.name === 'mobile') expect(after.y).toBeGreaterThan(before.y + 200);
  else { expect(after.x).toBeGreaterThan(before.x + 150); expect(after.width).toBeLessThan(before.width); }
  const toggleAfter = (await toggle.boundingBox())!;
  expect(toggleAfter.x).toBe(toggleBefore.x);
  expect(toggleAfter.y).toBe(toggleBefore.y);
  if (info.project.name === 'desktop') {
    await page.evaluate(() => scrollTo(0, document.documentElement.scrollHeight));
    expect(Math.abs((await toggle.boundingBox())!.y - toggleAfter.y)).toBeLessThan(1);
    await page.evaluate(() => scrollTo(0, 0));
  }
  await page.getByRole('radio', { name: 'Library tracks', exact: true }).check();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Home');
  for (const destination of ['Movies', 'Search titles', 'TV Shows', 'Home']) {
    await menu(page).getByRole('link', { name: destination, exact: true }).click();
    const position = (await toggle.boundingBox())!;
    expect(position.x).toBe(toggleAfter.x);
    expect(position.y).toBe(toggleAfter.y);
    await expect(toggle).toHaveAttribute('aria-expanded', 'true');
    await expect(page.getByRole('radio', { name: 'Library tracks', exact: true })).toBeChecked();
  }
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(page.getByRole('status')).toHaveText('Refreshed from server.');
  await expect(page.getByRole('link', { name: 'Log out', exact: true })).toHaveCount(0);
  await page.goto('/?screen=welcome');
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Home');
  await expect(page.getByRole('button', { name: 'Expand sidebar', exact: true })).toBeVisible();
  await page.reload();
  await expect(page.getByRole('link', { name: 'Continue', exact: true })).toHaveCount(0);
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Home');
  await expand(page);
  await page.getByRole('button', { name: 'Collapse sidebar' }).click();
  await expect(page.getByRole('button', { name: 'Expand sidebar' })).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(menu(page).getByRole('link', { name: 'Home', exact: true })).toBeFocused();
});

test('library hierarchy uses real breadcrumbs, scope and browser Back/Forward', { tag: '@regression' }, async ({ page }, info) => {
  await page.getByRole('link', { name: 'Browse libraries' }).click();
  await expect(page.getByRole('link', { name: / · Movie · / })).toHaveCount(8);
  await expect(page.getByRole('link', { name: / · Series · / })).toHaveCount(1);
  await menu(page).getByRole('link', { name: 'Movies', exact: true }).click();
  await expect(page.getByRole('link', { name: / · Series · / })).toHaveCount(0);
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Movies');
  await expect(page.getByRole('link', { name: / · Movie · / })).toHaveCount(8);
  await openSearch(page);
  await lexicalOnly(page);
  await page.getByRole('searchbox').fill('Beverly');
  await expect(page.getByText('No matches in Movies. Try another search.')).toBeVisible();
  await page.goBack();
  await page.goBack();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Movies');
  await menu(page).getByRole('link', { name: 'TV Shows', exact: true }).click();
  await expect(page.getByRole('link', { name: / · Movie · / })).toHaveCount(0);
  await page.getByRole('link', { name: /^The Beverly Hillbillies · Series/ }).click();
  await expect(page.getByRole('link', { name: / · Season · / })).toHaveCount(1);
  await expect(page.getByRole('link', { name: / · Season · / })).toContainText('2 included episodes');
  await page.getByRole('link', { name: /^Season 1 ·/ }).click();
  await expect(page.getByRole('link', { name: / · Episode · / })).toHaveCount(2);
  const seasonUrl = page.url();
  await expect(crumbs(page).getByRole('link')).toHaveText(['Home', 'TV Shows', 'The Beverly Hillbillies', 'Season 1']);
  await expect(crumbs(page).getByRole('link', { name: 'The Beverly Hillbillies', exact: true })).toHaveAttribute('href', '/?screen=browse&library=tv&series=beverly-hillbillies');
  await openSearch(page);
  await expect(page.getByText('Enter a title to search TV Shows.')).toBeVisible();
  await page.getByRole('searchbox').fill('Beverly');
  await expect(page.getByRole('link', { name: /^The Beverly Hillbillies · Series/ })).toBeVisible();
  await page.reload();
  await expect(page.getByRole('searchbox')).toHaveValue('Beverly');
  await expect(crumbs(page).getByRole('link')).toHaveText(['Home', 'TV Shows', 'The Beverly Hillbillies', 'Season 1', 'Search']);
  await page.keyboard.press('Escape');
  await expect(page.getByRole('searchbox')).toBeVisible();
  await expect(page.getByRole('searchbox')).toHaveValue('Beverly');
  await page.goBack();
  await expect(page).toHaveURL(seasonUrl);
  await page.goForward();
  await expect(page.getByRole('searchbox')).toHaveValue('Beverly');
  await crumbs(page).getByRole('link', { name: 'Season 1', exact: true }).click();
  await page.getByRole('link', { name: /^The Clampetts Strike Oil ·/ }).click();
  await expect(page.getByRole('article')).toContainText('The Beverly Hillbillies · S1 E1 · ~25m');
  await expect(page.getByRole('article')).toContainText('1962-09-26');
  await page.reload();
  await expect(crumbs(page).getByRole('link')).toHaveText(['Home', 'TV Shows', 'The Beverly Hillbillies', 'Season 1', 'The Clampetts Strike Oil']);
  await crumbs(page).getByRole('link', { name: 'The Beverly Hillbillies', exact: true }).click();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('The Beverly Hillbillies');
  await crumbs(page).getByRole('link', { name: 'TV Shows', exact: true }).click();
  await expect(page.getByRole('link', { name: / · Series · / })).toHaveCount(1);
});

test('search query, inline method guards, approximation, persistence and URL filter history', { tag: '@regression' }, async ({ page }, info) => {
  await page.evaluate(() => localStorage.setItem('screenroom.demo.searchTypes', '{bad json'));
  await openSearch(page);
  await expect(page.getByRole('searchbox')).toBeFocused();
  await expect(page.getByRole('button', { name: 'Search settings', exact: true })).toHaveAttribute('aria-expanded', 'false');
  await expect(page.getByRole('region', { name: 'Search settings', exact: true })).toHaveCount(0);
  await expect(page.getByRole('checkbox')).toHaveCount(0);
  await expect(page.getByRole('combobox', { name: 'Catalog', exact: true })).toHaveCount(0);
  await searchSettings(page);
  const settings = page.getByRole('region', { name: 'Search settings', exact: true });
  await expect(settings.getByRole('heading')).toHaveText(['Catalog', 'Media types', 'Search methods']);
  await expect(settings.getByRole('checkbox')).toHaveCount(7);
  await expect(settings.getByRole('combobox', { name: 'Catalog', exact: true })).toHaveValue('all');
  const viewport = page.viewportSize()!;
  await page.setViewportSize({ width: 320, height: viewport.height });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  for (const control of await settings.locator('input, select').all()) {
    const bounds = await control.boundingBox();
    expect(bounds!.x).toBeGreaterThanOrEqual(0);
    expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(320);
  }
  await page.setViewportSize(viewport);
  await expect(page.getByRole('checkbox', { name: 'Movies', exact: true })).toBeChecked();
  await expect(page.getByRole('checkbox', { name: 'Shows', exact: true })).toBeChecked();
  await expect(page.getByRole('checkbox', { name: 'Episodes', exact: true })).not.toBeChecked();
  await expect(page.getByText('Enter a title to search All libraries.')).toBeVisible();
  await page.getByRole('searchbox').fill('Bunny');
  await expect(page.getByRole('link', { name: /^Big Buck Bunny/ })).toBeVisible();
  await capture(page, `method-search-${info.project.name}`);
  await page.getByRole('searchbox').fill('zzzzzzzz');
  await expect(page.locator('.result-status')).toHaveText(/\d+ results? in All libraries/);
  await page.getByRole('button', { name: 'Clear query' }).click();
  await expect(page.getByRole('searchbox')).toBeFocused();
  await expect(page.getByText('Enter a title to search All libraries.')).toBeVisible();
  await page.getByRole('checkbox', { name: 'Shows', exact: true }).uncheck();
  await expect(page.getByRole('checkbox', { name: 'Movies', exact: true })).toBeDisabled();
  await expect(page.locator('.search-toolbar')).toHaveAttribute('aria-busy', 'false');
  await page.goBack();
  await expect(page.getByRole('checkbox', { name: 'Shows', exact: true })).toBeChecked();
  await page.goForward();
  await expect(page.getByRole('checkbox', { name: 'Movies', exact: true })).toBeDisabled();
  await page.getByRole('checkbox', { name: 'Episodes', exact: true }).check();
  await page.getByRole('checkbox', { name: 'Movies', exact: true }).uncheck();
  await expect(page.getByRole('checkbox', { name: 'Episodes', exact: true })).toBeDisabled();
  await page.getByRole('searchbox').fill('Clampetts');
  await expect(page.getByRole('link', { name: /^The Clampetts Strike Oil/ })).toHaveCount(1);
  await page.reload();
  await searchSettings(page);
  await expect(page.getByRole('checkbox', { name: 'Episodes', exact: true })).toBeDisabled();
  await page.getByRole('checkbox', { name: 'Movies', exact: true }).check();
  await searchSettings(page);
  const methods = page.getByRole('region', { name: 'Search methods', exact: true });
  await expect(methods).toBeVisible();
  await expect(page.getByRole('link', { name: /^The Clampetts Strike Oil/ })).toBeVisible();
  await capture(page, `search-settings-${info.project.name}`);
  await methods.getByRole('checkbox', { name: /^Title only/ }).check();
  await expect(methods.getByRole('checkbox', { name: /^Semantic/ })).toBeDisabled();
  await expect(methods.getByRole('checkbox', { name: /^Semantic/ })).not.toBeChecked();
  await methods.getByRole('checkbox', { name: /^Fuzzy titles/ }).uncheck();
  await expect(methods.getByRole('checkbox', { name: /^Text/ })).toBeDisabled();
  await methods.getByRole('checkbox', { name: /^Title only/ }).uncheck();
  await expect(methods.getByRole('checkbox', { name: /^Semantic/ })).not.toBeChecked();
  await methods.getByRole('checkbox', { name: /^Fuzzy titles/ }).check();
  await methods.getByRole('checkbox', { name: /^Text/ }).uncheck();
  await expect(page.locator('.search-toolbar')).toHaveAttribute('aria-busy', 'false');
  const fuzzyUrl = page.url();
  await page.keyboard.press('Escape');
  await expect(page.getByRole('button', { name: 'Search settings' })).toBeFocused();
  await expect(methods).not.toBeVisible();
  await page.getByRole('searchbox').fill('Big Buck Bunnu');
  await expect(page.getByRole('link', { name: /^Big Buck Bunny/ })).toBeVisible();
  await searchSettings(page);
  await methods.getByRole('checkbox', { name: /^Semantic/ }).check();
  await methods.getByRole('checkbox', { name: /^Fuzzy titles/ }).uncheck();
  await expect(methods.getByRole('checkbox', { name: /^Semantic/ })).toBeDisabled();
  await page.getByRole('searchbox').fill('rabbit');
  await expect(page.getByRole('link', { name: /^Big Buck Bunny/ })).toBeVisible();
  await page.reload();
  await expect(page.getByRole('searchbox')).toHaveValue('rabbit');
  await searchSettings(page);
  await expect(methods.getByRole('checkbox', { name: /^Semantic/ })).toBeDisabled();
  await page.goBack();
  await expect(methods.getByRole('checkbox', { name: /^Fuzzy titles/ })).toBeChecked();
  await page.goBack();
  expect(new URL(page.url()).searchParams.get('methods')).toBe(new URL(fuzzyUrl).searchParams.get('methods'));
  await openSearch(page);
  await expect(page.getByRole('searchbox')).toHaveValue('');
  await searchSettings(page);
  await expect(page.getByRole('checkbox', { name: 'Episodes', exact: true })).toBeChecked();
  await expect(methods.getByRole('checkbox', { name: /^Fuzzy titles/ })).toBeChecked();
  await expect(methods.getByRole('checkbox', { name: /^Semantic/ })).not.toBeChecked();
  await page.getByRole('searchbox').fill('Bunny');
  await page.getByRole('searchbox').fill('zzzzzzzz');
  await expect(page.getByText('No matches in All libraries. Try another search.')).toBeVisible();
  await expect(page.getByRole('link', { name: /^Big Buck Bunny/ })).toHaveCount(0);
});

test('direct URLs reconstruct every screen and safely fall back on invalid state', { tag: '@regression' }, async ({ page }) => {
  for (const [url, title] of [
    ['/?screen=browse&library=movies', 'Movies'],
    ['/?screen=browse&library=tv&series=beverly-hillbillies', 'The Beverly Hillbillies'],
    ['/?screen=browse&library=tv&series=beverly-hillbillies&season=beverly-hillbillies-s1', 'Season 1'],
    ['/?screen=browse&library=tv&series=beverly-hillbillies&season=beverly-hillbillies-s1&title=beverly-hillbillies-s1-e1', 'The Clampetts Strike Oil'],
    ['/?title=big-buck-bunny', 'Big Buck Bunny'],
    ['/?screen=welcome', 'Home'],
    ['/?screen=broken&library=invalid&title=missing', 'Home'],
    ['/?screen=browse&library=movies&series=beverly-hillbillies&season=beverly-hillbillies-s1&title=beverly-hillbillies-s1-e1', 'Movies'],
    ['/?screen=browse&library=tv&series=beverly-hillbillies&season=missing', 'The Beverly Hillbillies'],
    ['/?screen=browse&library=invalid&series=missing', 'All libraries']
  ]) {
    await page.goto(url);
    await expect(page.getByRole('heading', { level: 1 })).toHaveText(title);
    await expect(page.getByRole('button', { name: 'Expand sidebar' })).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  }
  await page.goto('/?screen=search&library=movies&q=Bunny&types=Movie&methods=titleOnly,semantic');
  await expect(page.getByRole('searchbox')).toHaveValue('Bunny');
  await searchSettings(page);
  await expect(page.getByRole('checkbox', { name: 'Movies', exact: true })).toBeDisabled();
  await expect(page.getByRole('link', { name: /^Big Buck Bunny/ })).toBeVisible();
  await expect(page.getByRole('checkbox', { name: /^Text/ })).toBeDisabled();
  await expect(page.getByRole('checkbox', { name: /^Semantic/ })).not.toBeChecked();
  await page.goto('/?screen=search&library=tv&q=Beverly&types=invalid&methods=invalid&series=missing&season=missing&title=missing');
  await expect(page.getByRole('searchbox')).toHaveValue('Beverly');
  await expect(page.getByRole('link', { name: /^The Beverly Hillbillies · Series/ })).toBeVisible();
  await searchSettings(page);
  for (const name of [/^Text/, /^Fuzzy titles/, /^Semantic/]) await expect(page.getByRole('checkbox', { name })).toBeChecked();
});

test('browse details validate ancestry while search details retain their originating context', { tag: '@regression' }, async ({ page }) => {
  for (const [query, heading, breadcrumb] of [
    ['screen=browse&library=tv&series=beverly-hillbillies&season=beverly-hillbillies-s1&title=missing', 'Season 1', ['Home', 'TV Shows', 'The Beverly Hillbillies', 'Season 1']],
    ['screen=browse&library=all&series=beverly-hillbillies&title=big-buck-bunny', 'The Beverly Hillbillies', ['Home', 'All libraries', 'The Beverly Hillbillies']],
    ['screen=browse&library=tv&series=beverly-hillbillies&title=beverly-hillbillies-s1-e2', 'Getting Settled', ['Home', 'TV Shows', 'The Beverly Hillbillies', 'Getting Settled']],
    ['screen=browse&library=tv&series=beverly-hillbillies&season=beverly-hillbillies-s1&title=beverly-hillbillies-s1-e1', 'The Clampetts Strike Oil', ['Home', 'TV Shows', 'The Beverly Hillbillies', 'Season 1', 'The Clampetts Strike Oil']],
    ['screen=search&library=all&series=beverly-hillbillies&season=beverly-hillbillies-s1&q=Bunny&title=big-buck-bunny', 'Big Buck Bunny', ['Home', 'All libraries', 'The Beverly Hillbillies', 'Season 1', 'Search', 'Big Buck Bunny']],
    ['screen=search&library=tv&series=beverly-hillbillies&q=Settled&title=beverly-hillbillies-s1-e2', 'Getting Settled', ['Home', 'TV Shows', 'The Beverly Hillbillies', 'Search', 'Getting Settled']]
  ] as const) {
    await page.goto(`/?${query}`);
    await expect(page.getByRole('heading', { level: 1 })).toHaveText(heading);
    await expect(crumbs(page).getByRole('link')).toHaveText([...breadcrumb]);
    if (await page.getByRole('article').count()) await expect(page.getByRole('article')).toContainText('Jellyfin');
  }
});

test('search result details preserve query and link targets support new tabs', { tag: '@regression' }, async ({ page, context }) => {
  await page.goto('/?screen=search&library=movies&q=Bunny&types=Movie&methods=text');
  const card = page.getByRole('link', { name: /^Big Buck Bunny/ });
  await expect(card).toBeVisible();
  const destination = await card.getAttribute('href');
  expect(destination).toContain('q=Bunny');
  expect(destination).toContain('title=big-buck-bunny');
  const popupPromise = context.waitForEvent('page');
  await card.click({ modifiers: ['Meta'] });
  const popup = await popupPromise;
  await popup.waitForLoadState();
  await expect(popup.getByRole('heading', { level: 1 })).toHaveText('Big Buck Bunny');
  await popup.close();
  await card.click();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Big Buck Bunny');
  await page.goBack();
  await expect(page.getByRole('searchbox')).toHaveValue('Bunny');
  await expect(card).toBeVisible();
  await page.goForward();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Big Buck Bunny');
  await crumbs(page).getByRole('link', { name: 'Search', exact: true }).click();
  await expect(page.getByRole('searchbox')).toHaveValue('Bunny');
});

test('rapid typing replaces history and a navigation cancels outstanding query updates', { tag: '@regression' }, async ({ page }) => {
  await openSearch(page);
  await page.getByRole('searchbox').pressSequentially('Bunny');
  await expect(page.locator('.search-toolbar')).toHaveAttribute('aria-busy', 'false');
  await expect(page).toHaveURL(/q=Bunny/);
  await expect(page.getByRole('link', { name: /^Big Buck Bunny/ })).toBeVisible();
  await page.goBack();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Home');
  await page.goForward();
  await expect(page.getByRole('searchbox')).toHaveValue('Bunny');
  await page.getByRole('searchbox').fill('New query');
  await menu(page).getByRole('link', { name: 'Movies', exact: true }).click();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Movies');
  await expect(page.getByRole('link', { name: / · Movie · / })).toHaveCount(8);
  await page.reload();
  await expect(page.getByRole('heading', { level: 1 })).toHaveText('Movies');
});

test('catalog scope intersects filters, restores URL history and never invents a Jellyfin connection', { tag: '@regression' }, async ({ page }, info) => {
  await openSearch(page);
  await searchSettings(page);
  const source = page.getByRole('combobox', { name: 'Catalog', exact: true });
  await expect(source).toHaveValue('all');
  await page.getByRole('searchbox').fill('Bunny');
  const bunny = page.getByRole('link', { name: /^Big Buck Bunny/ });
  await expect(bunny).toBeVisible();
  await source.selectOption('tmdb');
  await expect(page).toHaveURL(/source=tmdb/);
  await expect(bunny).toBeVisible();
  await searchSettings(page, false);
  await expect(page.getByRole('checkbox')).toHaveCount(0);
  await expect(source).toHaveCount(0);
  await expect(page).toHaveURL(/source=tmdb/);
  await expect(bunny).toBeVisible();
  await searchSettings(page);
  await expect(source).toHaveValue('tmdb');
  await searchSettings(page, false);
  await page.reload();
  await expect(page.getByRole('button', { name: 'Search settings', exact: true })).toHaveAttribute('aria-expanded', 'false');
  await searchSettings(page);
  await expect(source).toHaveValue('tmdb');
  await expect(bunny).toBeVisible();
  await bunny.click();
  await expect(page).toHaveURL(/source=tmdb/);
  await crumbs(page).getByRole('link', { name: 'Search', exact: true }).click();
  await searchSettings(page);
  await expect(source).toHaveValue('tmdb');
  await expect(page.getByRole('searchbox')).toHaveValue('Bunny');
  await searchSettings(page);
  await source.selectOption('jellyfin');
  await expect(bunny).toBeVisible();
  await expect(page).toHaveURL(/source=jellyfin/);
  await page.reload();
  await searchSettings(page);
  await expect(source).toHaveValue('jellyfin');
  await expect(bunny).toBeVisible();
  await searchSettings(page);
  await source.selectOption('tmdb');
  await expect(bunny).toBeVisible();
  await page.goBack();
  await expect(source).toHaveValue('jellyfin');
  await page.goForward();
  await searchSettings(page);
  await expect(source).toHaveValue('tmdb');
  await page.getByRole('checkbox', { name: 'Episodes', exact: true }).check();
  await page.getByRole('checkbox', { name: 'Movies', exact: true }).uncheck();
  await expect(bunny).toHaveCount(0);
  await expect(source).toHaveValue('tmdb');
  await openSearch(page);
  await searchSettings(page);
  await expect(source).toHaveValue('tmdb');
  expect(new URL(page.url()).searchParams.get('source')).toBe('tmdb');
  await page.goto('/?screen=search&library=tv&q=Bunny&types=Movie&methods=text&source=tmdb');
  await expect(page.locator('.result-status')).toContainText('No matches in TV Shows');
  await expect(bunny).toHaveCount(0);
  await page.goto('/?screen=search&library=movies&q=Bunny&types=Movie&methods=text&source=invalid');
  await searchSettings(page);
  await expect(source).toHaveValue('all');
  await expect(bunny).toBeVisible();
  await page.getByRole('searchbox').fill('Sintel');
  await expect(page).not.toHaveURL(/source=/);
  await page.goto('/?screen=search&library=tv&source=jellyfin');
  await searchSettings(page);
  await expect(source).toHaveValue('jellyfin');
  await expect(page.locator('.result-status')).toContainText('Enter a title to search TV Shows.');
});

test('real artwork, cast roles, crew, ratings and provider links', { tag: '@regression' }, async ({ page }, info) => {
  const images = page.locator('img');
  await expect(images.first()).toBeVisible();
  await expect.poll(() => images.evaluateAll(nodes => nodes.every(n => (n as HTMLImageElement).complete && (n as HTMLImageElement).naturalWidth > 0))).toBe(true);
  await expect(page.getByText('POSTER NOT INCLUDED').first()).toBeVisible();
  await page.goto('/?title=big-buck-bunny');
  const article = page.getByRole('article');
  await expect(article.getByRole('region', { name: 'Crew', exact: true })).toContainText('Sacha Goedegebure');
  await expect(article.getByRole('region', { name: 'Crew', exact: true })).toContainText('Director');
  const director = article.getByRole('region', { name: 'Crew', exact: true }).getByRole('listitem').filter({ has: page.getByText('Sacha Goedegebure', { exact: true }) }).filter({ has: page.getByText('Director', { exact: true }) });
  await expect(director.getByRole('img', { name: 'Sacha Goedegebure portrait', exact: true })).toBeVisible();
  const artDirector = article.getByRole('region', { name: 'Crew', exact: true }).getByRole('listitem').filter({ has: page.getByText('Andreas Goralczyk', { exact: true }) }).filter({ has: page.getByText('Art Direction', { exact: true }) });
  const andyPortrait = artDirector.getByRole('img', { name: 'Andy Goralczyk portrait', exact: true });
  await expect(andyPortrait).toBeVisible();
  await expect(andyPortrait).toHaveAttribute('src', /^blob:/);
  await expect.poll(() => andyPortrait.evaluate(n => (n as HTMLImageElement).complete && (n as HTMLImageElement).naturalWidth > 0)).toBe(true);
  await expect(article.getByRole('region', { name: 'Cast', exact: true })).toContainText('No cast credits listed');
  const records = snapshot.items as Record<string, { url: string; rating: number; votes: number }>;
  expect(Object.keys(records)).toHaveLength(12);
  const metadata = article.getByRole('group', { name: 'Rating and catalogs', exact: true });
  const tmdb = metadata.getByRole('link', { name: `TMDB: Listed; user rating ${records['big-buck-bunny'].rating.toFixed(1)}/10`, exact: true });
  await expect(tmdb).toBeVisible();
  await expect(tmdb).toContainText(`${records['big-buck-bunny'].rating.toFixed(1)}/10`);
  await expect(tmdb).toHaveAttribute('href', records['big-buck-bunny'].url);
  await expect(tmdb).toHaveAttribute('rel', 'noopener noreferrer');
  await expect(metadata.getByRole('status', { name: 'Jellyfin: Not connected', exact: true })).toBeVisible();
  const infoToggle = metadata.locator('summary[aria-label="Rating and catalog details"]');
  const ratingInfo = metadata.locator('.info-content');
  await expect(metadata.locator('details')).not.toHaveAttribute('open');
  await expect(ratingInfo).not.toBeVisible();
  await expect(article.getByRole('heading', { name: 'TMDB user rating', exact: true })).toHaveCount(0);
  await expect(article.getByRole('heading', { name: 'Catalog & connections', exact: true })).toHaveCount(0);
  await infoToggle.focus();
  await infoToggle.press('Enter');
  await expect(ratingInfo).toBeVisible();
  await expect(ratingInfo).toContainText(`TMDB user rating: ${records['big-buck-bunny'].rating.toFixed(1)} / 10`);
  await expect(ratingInfo).toContainText(`${records['big-buck-bunny'].votes.toLocaleString('en-US')} votes`);
  await expect(ratingInfo).toContainText(String(snapshot.retrievedAt).slice(0, 10));
  await expect(ratingInfo).toContainText('Not live');
  await expect(ratingInfo).toContainText('A catalog listing does not mean this title is available to play.');
  await expect(ratingInfo).toContainText('Jellyfin · Not connected');
  await metadata.scrollIntoViewIfNeeded();
  await capture(page, `metadata-info-${info.project.name}`);
  await infoToggle.click();
  await expect(ratingInfo).not.toBeVisible();
  await infoToggle.click();
  await expect(ratingInfo).toBeVisible();
  const viewport = page.viewportSize()!;
  await page.setViewportSize({ width: 320, height: viewport.height });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  for (const badge of await metadata.locator('.badge, summary').all()) {
    const bounds = await badge.boundingBox();
    expect(bounds!.x).toBeGreaterThanOrEqual(0);
    expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(320);
  }
  await page.setViewportSize(viewport);
  await infoToggle.click();
  await article.getByRole('region', { name: 'Crew', exact: true }).scrollIntoViewIfNeeded();
  await capture(page, `people-${info.project.name}`);
  await page.goto('/?title=night-of-the-living-dead');
  await expect(page.getByRole('region', { name: 'Cast', exact: true })).toContainText('Duane Jones');
  await expect(page.getByRole('region', { name: 'Cast', exact: true })).toContainText('Ben');
  await expect(page.getByRole('img', { name: /Duane Jones as Ben/ })).toBeVisible();
  await expect(page.getByRole('img', { name: /No portrait included for/ }).first()).toBeVisible();
  await page.goto('/?title=the-general');
  await expect(page.getByRole('img', { name: /Buster Keaton as Johnnie Gray/ }).first()).toBeVisible();
  await page.goto('/?title=beverly-hillbillies-s1-e2');
  await expect(page.getByRole('article')).toContainText('1962-10-03');
  await expect(page.getByRole('region', { name: 'Crew', exact: true })).toContainText('Richard Whorf');
  await expect(page.getByRole('region', { name: 'Crew', exact: true })).toContainText('Paul Henning');
});

test('readable credits attribute server images and distinguish film rights', { tag: '@regression' }, async ({ page }, info) => {
  await page.locator('summary').filter({ hasText: 'Data & artwork credits' }).click();
  const credits = page.locator('.credits');
  await expect(credits).toContainText('This product uses the TMDB API but is not endorsed or certified by TMDB.');
  await expect(credits).toContainText('William Maanders');
  await expect(credits).toContainText('CC BY 3.0 Netherlands');
  await expect(credits).toContainText('CC BY 1.0');
  await expect(credits).toContainText('not a complete season');
  await expect(credits.getByRole('img', { name: 'TMDB approved primary short blue logo', exact: true })).toBeVisible();
  await credits.scrollIntoViewIfNeeded();
  await capture(page, `credits-${info.project.name}`);
  for (const image of await page.locator('img').all()) {
    await image.scrollIntoViewIfNeeded();
    await expect.poll(() => image.evaluate(n => (n as HTMLImageElement).complete && (n as HTMLImageElement).naturalWidth > 0)).toBe(true);
  }
});
