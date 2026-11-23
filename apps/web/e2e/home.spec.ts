import {expect,test,type Page} from '@playwright/test';
import {rpcRoute,rpcPath,isRpcPath,type RpcMethod} from './rpc-paths';
import { mkdir } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { routeId } from './route-id.mjs';
const menu=(page:Page)=>page.getByRole('navigation',{name:'Main menu',exact:true});
async function healthy(page:Page){await page.goto('/');await expect(page.getByRole('heading',{name:'Home',exact:true})).toBeVisible();await expect(page.getByText('Loading Screenroom…',{exact:true})).toHaveCount(0);}
test.beforeEach(async({page})=>{await page.addInitScript(namespace=>localStorage.setItem('screenroom.settingsNamespace',namespace),`web-${crypto.randomUUID()}`);});
test('real catalog showcase: home, sidebar, scoped library, search and detail @happy-path', async ({ page }, info) => {
  test.setTimeout(180_000);
  const errors: string[] = [];
  const requests: string[] = [];
  const artworkResponses: string[] = [];
  const captureAlerts: string[] = [];
  const endpoint = new URL(process.env.SCREENROOM_E2E_API_ENDPOINT || process.env.SCREENROOM_API_ENDPOINT || 'http://127.0.0.1:50051');
  page.on('pageerror', error => errors.push(error.message));
  page.on('request', request => requests.push(request.url()));
  page.on('response', response => { const url = new URL(response.url()); if (url.origin === endpoint.origin && url.pathname.startsWith('/artwork/') && response.ok()) artworkResponses.push(response.url()); });
  function assertRequests() {
    const web = new URL(page.url());
    expect(requests.filter(value => {
      const url = new URL(value);
      if (['blob:', 'data:'].includes(url.protocol)) return false;
      if (url.origin === web.origin) return url.pathname.startsWith('/__screenroom') || url.pathname.startsWith('/__operator');
      return url.origin !== endpoint.origin || !(url.pathname.startsWith('/artwork/') || (isRpcPath(url.pathname) && !/\/(Set|Clear)/.test(url.pathname)));
    }), 'All showcase scenes must use only the configured public core API and artwork origin').toEqual([]);
  }
  const directory = fileURLToPath(new URL('./build/screenshots/', import.meta.url));
  await mkdir(directory, { recursive: true });
  async function capture(flow: string) {
    captureAlerts.push(...(await page.locator('[role="alert"]').allTextContents()).map(message => `${flow}: ${message}`));
    await page.evaluate(() => document.fonts.ready);
    await expect(page.locator('main .poster img').first(), `${flow} requires actual catalog artwork`).toBeVisible();
    await expect(page.locator('main .image-state'), `${flow} must not contain loading or unavailable artwork placeholders`).toHaveCount(0, { timeout: 15_000 });
    await expect.poll(() => page.locator('main .poster img').evaluateAll(images => images.length > 0 && images.every(image => (image as HTMLImageElement).complete && (image as HTMLImageElement).naturalWidth > 0)), { timeout: 15_000 }).toBe(true);
    assertRequests();
    await page.screenshot({ path: `${directory}/${flow}-${info.project.name}.png`, animations: 'disabled' });
  }
  await healthy(page);
  await expect(page.locator('.rows .card').first(), 'Showcase requires a populated real Home').toBeVisible();
  await capture('home');
  await page.getByRole('button', { name: 'Expand sidebar', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Refresh', exact: true })).toBeVisible();
  await capture('sidebar');
  const refresh = page.waitForRequest(request => new URL(request.url()).pathname === rpcPath('BrowseService/GetHome'), { timeout: 10_000 });
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await refresh;
  await expect(page.getByRole('status').filter({ hasText: 'Refreshed from server.' })).toBeVisible();
  await page.getByRole('button', { name: 'Collapse sidebar', exact: true }).click();
  const libraries = await menu(page).locator('a[href^="/libraries/"]').evaluateAll(links => links.map(link => ({ href: link.getAttribute('href')!, title: link.getAttribute('aria-label')! })));
  let selected: { href: string; title: string; libraryHref: string; libraryTitle: string } | undefined;
  for (const library of libraries.slice(0, 8)) {
    await page.goto(library.href);
    await expect(page.locator('[aria-label="Catalog"]')).toBeVisible();
    for (let index = 0; index < 3; index++) {
      const movies = page.locator('[aria-label="Catalog"] a[href^="/titles/"][aria-label*=" · Movie · "]');
      const candidate = movies.filter({ has: page.locator('.poster img') }).first();
      if (await movies.count()) await expect(candidate, 'Discovery requires a real movie with loaded artwork').toBeVisible({ timeout: 15_000 });
      if (await candidate.count()) {
        selected = { href: (await candidate.getAttribute('href'))!, title: (await candidate.locator('.title').innerText()).trim(), libraryHref: library.href, libraryTitle: library.title };
        break;
      }
      const more = page.getByRole('button', { name: 'Load more', exact: true });
      if (index === 2 || !await more.count()) break;
      const previous = await page.locator('[aria-label="Catalog"] .card').evaluateAll(cards => cards.map(card => card.getAttribute('href')));
      const response = page.waitForResponse(response => new URL(response.url()).pathname === rpcPath('BrowseService/ListLibraryItems'), { timeout: 10_000 });
      // Keyboard activation avoids scrolling the automatic paging sentinel into view.
      await Promise.all([response, more.press('Enter', { timeout: 5_000 })]);
      await expect(page.getByRole('button', { name: 'Loading…', exact: true })).toHaveCount(0);
      const appended = await page.locator('[aria-label="Catalog"] .card').evaluateAll(cards => cards.map(card => card.getAttribute('href')));
      expect(previous.every(href => appended.includes(href)), 'Pagination must retain already loaded catalog cards').toBe(true);
    }
    if (selected) break;
  }
  expect(selected, 'No real movie for search/detail capture within 8 libraries × 3 bounded discovery steps').toBeDefined();
  const item = selected!;
  const scope = new URL(item.libraryHref, page.url()).searchParams;
  const libraryId = routeId(item.libraryHref, 'libraries', page.url());
  await expect(page.getByRole('heading', { name: item.libraryTitle, exact: true })).toBeVisible();
  expect(routeId(page.url(), 'libraries', page.url())).toBe(libraryId);
  await capture('library');
  await menu(page).getByRole('link', { name: 'Search titles', exact: true }).click();
  await expect(page.getByRole('searchbox')).toHaveAttribute('aria-label', `Search ${item.libraryTitle}`);
  // Use advertised text search rather than depending on optional semantic models.
  const searchURL = new URL(page.url());
  searchURL.searchParams.set('methods', 'titleOnly,text');
  searchURL.searchParams.set('types', 'Movie,Series,Episode');
  await page.goto(searchURL.href);
  await page.getByRole('searchbox').fill(item.title);
  const id = routeId(item.href, 'titles', page.url());
  const selectedResult = page.locator('[aria-label="Search results"] a').filter({ hasText: item.title });
  await expect(selectedResult.first(), 'Selected real title must be present in scoped search results').toBeVisible();
  const matching = page.locator(`[aria-label="Search results"] a[href^=${JSON.stringify(`/titles/${encodeURIComponent(id)}?`)}]`);
  await expect(matching).toBeVisible();
  expect(routeId((await matching.getAttribute('href'))!, 'titles', page.url())).toBe(id);
  expect(new URL(page.url()).searchParams.get('library')).toBe(libraryId);
  if (scope.has('source_id')) expect(new URL(page.url()).searchParams.get('source_id')).toBe(scope.get('source_id'));
  await expect(page.locator('.search-toolbar')).toHaveAttribute('aria-busy', 'false');
  await expect(matching.locator('.poster img'), 'Selected real item must have artwork in search').toBeVisible();
  const searchState = new URL(page.url()).searchParams;
  function assertScope(detail: boolean) {
    const actual = new URL(page.url()).searchParams;
    for (const key of ['source_id', 'library', 'q', 'methods', 'types', 'source']) expect(actual.get(key), `Preserve ${key}`).toBe(searchState.get(key));
    if (detail) expect(routeId(page.url(), 'titles', page.url())).toBe(id);
    else expect(new URL(page.url()).pathname).toBe('/search');
  }
  await capture('search');
  await matching.click();
  await expect(page.getByRole('article', { name: `${item.title} details`, exact: true })).toBeVisible();
  await expect(page.getByRole('heading', { name: item.title, exact: true })).toBeVisible();
  assertScope(true);
  await page.reload();
  await expect(page.getByRole('heading', { name: item.title, exact: true })).toBeVisible();
  assertScope(true);
  await capture('detail');
  await page.goBack();
  await expect(matching, 'Back must restore the selected real item in scoped search').toBeVisible();
  assertScope(false);
  await expect(page.getByRole('searchbox')).toHaveValue(item.title);
  assertRequests();
  for (const method of ['BrowseService/GetHome', 'BrowseService/ListLibraries', 'BrowseService/ListLibraryItems', 'SearchService/Search', 'CatalogService/GetItem', 'CatalogService/ListCredits'] as RpcMethod[]) expect(requests.some(value => new URL(value).pathname === rpcPath(method)), `Required real public read: ${method}`).toBe(true);
  expect(artworkResponses.length, 'Showcase requires successful artwork HTTP responses').toBeGreaterThan(0);
  await info.attach('real-catalog-selection', { body: JSON.stringify({ id, title: item.title, search: searchState.toString(), requests: requests.filter(value => new URL(value).origin === endpoint.origin), artworkResponses }), contentType: 'application/json' });
  expect(captureAlerts, 'Showcase captures must not contain unresolved catalog read errors').toEqual([]);
  expect(errors).toEqual([]);
});
test('healthy source-driven Home or honest first-run empty state @happy-path',async({page})=>{const errors:string[]=[];page.on('pageerror',e=>errors.push(e.message));await healthy(page);const empty=page.getByRole('heading',{name:'Your library starts here',exact:true});if(await empty.count()){await expect(empty).toBeVisible();await expect(page.getByText(/No libraries are available|Add your Jellyfin server/)).toBeVisible();}else await expect(page.locator('.rows')).toBeVisible();expect(errors).toEqual([]);});
test('public reads stay on the configured backend and artwork stays core loaded @regression',async({page})=>{const urls:string[]=[];page.on('request',r=>urls.push(r.url()));await healthy(page);const endpoint=new URL(process.env.SCREENROOM_E2E_API_ENDPOINT||process.env.SCREENROOM_API_ENDPOINT||'http://127.0.0.1:50051');const web=new URL(page.url());expect(urls.filter(value=>{const url=new URL(value);return !['blob:','data:'].includes(url.protocol)&&url.origin!==web.origin&&!(url.origin===endpoint.origin&&(isRpcPath(url.pathname)||url.pathname.startsWith('/artwork/')));})).toEqual([]);expect(urls.some(value=>new URL(value).pathname===rpcPath('BrowseService/GetHome'))).toBe(true);});
test('source-driven browse lists libraries and can return Home @happy-path',async({page})=>{await healthy(page);await page.getByRole('link',{name:'Browse libraries',exact:true}).click();await expect(page.getByRole('heading',{name:'All libraries',exact:true})).toBeVisible();await menu(page).getByRole('link',{name:'Home',exact:true}).click();await expect(page.getByRole('heading',{name:'Home',exact:true})).toBeVisible();});
test('network failure renders a retryable error without catalog snapshots @retry',async({page})=>{await page.route(rpcRoute('BrowseService/ListLibraries'),route=>route.abort());await page.goto('/');await expect(page.getByRole('heading',{name:'Unable to load Screenroom',exact:true})).toBeVisible();await page.unroute(rpcRoute('BrowseService/ListLibraries'));await page.getByRole('button',{name:'Retry',exact:true}).click();await expect(page.getByRole('heading',{name:'Home',exact:true})).toBeVisible();});
test('empty or populated search follows server capabilities rather than fixture titles @regression',async({page})=>{await healthy(page);await menu(page).getByRole('link',{name:'Search titles',exact:true}).click();await expect(page.getByRole('searchbox')).toBeVisible();await page.getByRole('searchbox').fill('screenroom-search-no-assumed-catalog');await expect(page).toHaveURL(/q=/);await expect(page.getByRole('searchbox')).toHaveValue('screenroom-search-no-assumed-catalog');});
test('settings are isolated and survive navigation without fixed provider names @regression',async({page})=>{await healthy(page);await menu(page).getByRole('link',{name:'Search titles',exact:true}).click();await page.getByRole('button',{name:'Search settings',exact:true}).click();const fuzzy=page.getByRole('checkbox',{name:'Fuzzy titles',exact:true});if(await fuzzy.isEnabled()){await fuzzy.uncheck();await page.reload();await page.getByRole('button',{name:'Search settings',exact:true}).click();await expect(fuzzy).not.toBeChecked();}});
test('libraries route lists available scoped libraries @regression',async({page})=>{await page.goto('/libraries');await expect(page.getByRole('heading',{name:'All libraries',exact:true})).toBeVisible();});
