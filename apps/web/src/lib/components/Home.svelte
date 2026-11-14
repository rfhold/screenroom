<script lang="ts">
  import { page } from '$app/state';
  import { onDestroy } from 'svelte';
  import Maintenance from './Maintenance.svelte';
  import { operatorConfig } from '../core/operator-config';
  import type { Item, Library } from '../core/models';
  import { client, newClient, credit, allCredits, failure, item, loadSettings, saveSettings, settingsWarning, configure, scope } from '../core/client';
  import { collectPages, IncrementalPages } from '../core/pagination';
  import LoadMore from './LoadMore.svelte';
  import { OwnedReads } from '../core/owned-reads';
  import { MetadataOperation, MetadataCancelled } from '../core/operation.mjs';
  import { itemHref, readLocation, resolveLocation, type Location } from '../core/navigation';
  import type { SearchCapabilities } from '../../../../../target/client-core/wasm/screenroom_client_core';
  import PosterCard from './PosterCard.svelte';
  import Menu from './Menu.svelte';
  import Search from './Search.svelte';
  import Detail from './Detail.svelte';
  import Breadcrumbs from './Breadcrumbs.svelte';
  import { progressError } from '../core/jellyfin.mjs';
  let location = $state<Location>(readLocation(page.url));
  let expanded = $state(false);
  let status = $state('');
  let error = $state('');
  let ready = $state(false);
  let version = $state(0);
  let libraries = $state<Library[]>([]);
  let homeRows = $state<{title: string; items: Item[]; readError: string}[]>([]);
  let items = $state<Item[]>([]);
  let generation = 0;
  let searchCapabilities = $state<SearchCapabilities>();
  let refreshing = $state(false);
  let metadataOperation = $state<MetadataOperation>();
  let more = $state(false);
  let loadingMore = $state(false);
  let pageError = $state('');
  let loadMore = $state<() => void>(() => {});
  let cancelPages = () => {};
  $effect(() => {
    const url = new URL(page.url.href);
    if (url.searchParams.get('screen') === 'maintenance') { generation++; cancelPages(); location = readLocation(url); return; }
    version;
    const current = ++generation;
    cancelPages();
    more = false; loadingMore = false; pageError = '';
    const operation = new MetadataOperation(() => current === generation);
    void load(url, current, operation);
    return () => { operation.cancel(); cancelPages(); };
  });
  async function load(url: URL, current: number, operation: MetadataOperation) {
    try {
      error = '';
      const c = await operation.read(client);
      operation.check();
      if (refreshing) c.invalidate_reads();
      const prefs = await loadSettings(operation);
      const sourceId = url.searchParams.get('source_id') ?? prefs.scope?.source_id ?? '';
       const [availableRecords, capabilities] = await operation.read(() => Promise.all([
        collectPages(async page => { const response = await c.list_libraries(sourceId, page); return { entries: response.libraries, next_page_token: response.next_page_token }; }, undefined, entry => entry.id, operation),
         operation.read(() => c.get_search_capabilities()),
      ]));
      const available = [{ id: 'all', title: 'All libraries' }, ...availableRecords];
      operation.check();
      if (current !== generation) return;
      configure(available, capabilities);
      const next = await resolveLocation(url, operation);
      operation.check();
      if (current !== generation) return;
      const context = scope(next.library.id, next.sourceId);
      let rows: typeof homeRows = [];
      let list: Item[] = [];
      if (next.title) {
        const credits = await allCredits(next.title.id, context, operation);
        operation.check();
        const cast = credits.filter(person => person.department.toLowerCase() === 'cast').map(credit);
        const crew = credits.filter(person => person.department.toLowerCase() !== 'cast').map(credit);
        operation.check();
        next.title.cast = cast; next.title.crew = crew;
      } else if (next.screen === 'home') {
        const response = await operation.read(() => c.get_home(context));
        operation.check();
        rows = response.map(row => ({ title: row.title, readError: row.read_error ? progressError(row.read_error) : '', items: row.items.map(record => { operation.check(); return item(record); }) }));
      } else if (next.screen === 'browse') {
        const owned = await operation.read(newClient);
        if (current !== generation) { owned.free(); return; }
        const reads = new OwnedReads(owned);
        const isCurrent = () => current === generation && operation.active() && !operation.stopped;
        const pages = new IncrementalPages(async page => {
          const response = await reads.read(() => next.path.length ? owned.list_children(next.path.at(-1)!.id, page, context) : owned.list_library_items(context, page));
          return { entries: response.items, next_page_token: response.next_page_token };
        }, isCurrent);
        let disposed = false;
        cancelPages = () => { if (disposed) return; disposed = true; pages.cancel(); reads.dispose(); };
        const display = () => pages.entries.map(item).filter(record => next.path.length || !record.parent);
        loadMore = async () => {
          if (!isCurrent() || pages.loading || !pages.more) return;
          loadingMore = true; pageError = '';
          try { await pages.load(); if (isCurrent()) items = display(); }
          catch (cause) { if (isCurrent() && !(cause instanceof MetadataCancelled)) pageError = failure(cause); }
          finally { if (isCurrent()) { loadingMore = false; more = pages.more; } }
        };
        await pages.load(operation);
        operation.check();
        list = display(); more = pages.more;
      }
      await saveSettings({ scope: context }, () => current === generation, operation);
      operation.check();
      if (current !== generation) return;
      libraries = available;
      searchCapabilities = capabilities;
      metadataOperation = operation;
      homeRows = rows; items = list; location = next; ready = true;
      if (refreshing && (next.title || next.screen !== 'search' || !next.query.trim())) completeRefresh();
    } catch (cause) { if (current === generation && !(cause instanceof MetadataCancelled)) { operation.stop(cause instanceof Error ? cause : new Error(String(cause))); error = failure(cause); ready = true; refreshing = false; status = ''; } }
  }
  function refresh() { refreshing = true; status = 'Refreshing…'; version++; }
  function completeRefresh(cause?: string) { if (!refreshing) return; status = cause ? '' : 'Refreshed from server.'; refreshing = false; }
  onDestroy(() => { generation++; metadataOperation?.cancel(); cancelPages(); });
</script>

<svelte:head><title>{location.screen === 'maintenance' ? 'Maintenance' : location.title?.title ?? (location.screen === 'search' ? 'Search' : location.screen === 'home' ? 'Home' : location.path.at(-1)?.title ?? location.library.title)} · Screenroom</title></svelte:head>
{#if page.url.searchParams.get('screen') === 'maintenance'}
  <Maintenance />
{:else}
<a class="skip-link" href="#main">Skip to content</a>
<div class="shell">
   <Menu {location} {libraries} bind:expanded {refresh} />
  <main id="main" tabindex="-1" class:detail-page={!!location.title}>
    <Breadcrumbs {location} />
     {#if status}<p role="status" class="status">{status}</p>{/if}
     {#if $settingsWarning}<p role="status" class="settings-warning">{$settingsWarning}</p>{/if}
    {#if error}<section role="alert"><h1>Unable to load Screenroom</h1><p>{error}</p><button onclick={() => version++}>Retry</button></section>
    {:else if !ready}<p role="status">Loading Screenroom…</p>
    {:else if location.title}
      <Detail item={location.title} />
    {:else if location.screen === 'search'}
      {#if searchCapabilities && metadataOperation}<Search {location} capabilities={searchCapabilities} operation={metadataOperation} refreshVersion={version} onReadComplete={completeRefresh} onRetry={() => version++} />{/if}
    {:else}
      <div class="page-header"><h1>{location.screen === 'home' ? 'Home' : location.path.at(-1)?.title ?? location.library.title}</h1>
        {#if location.screen === 'home'}<a class="button browse" href="/?screen=browse&library=all">Browse libraries</a>{/if}
      </div>
      {#if location.screen === 'home'}
        {#if libraries.length <= 1 && !homeRows.some(row => row.items.length)}<section class="empty-library"><h2>Your library starts here</h2><p>{operatorConfig.enabled ? 'Add your Jellyfin server to bring your library into Screenroom.' : 'No libraries are available. Server setup is unavailable on this web attachment.'}</p>{#if operatorConfig.enabled}<a class="button" href="/?screen=maintenance">Add server</a>{/if}</section>{/if}
        <div class="rows">{#each homeRows as row, index (row.title)}
          <section aria-labelledby={`row-${index}`}><h2 id={`row-${index}`}>{row.title}</h2>{#if row.readError}<p role="alert">{row.readError}</p><button onclick={refresh}>Retry row</button>{/if}<div class="poster-row">{#each row.items as item (item.id)}<PosterCard {item} href={itemHref(location, item)} />{/each}</div></section>
        {/each}</div>
      {:else}<div class="grid catalog" aria-label="Catalog">{#each items as item (item.id)}<PosterCard {item} href={itemHref(location, item)} seasonContext={location.path.at(-1)?.kind === 'Season' && item.parent === location.path.at(-1)?.id} />{/each}</div><p>{items.length} loaded</p><LoadMore {more} loading={loadingMore} error={pageError} load={loadMore} />{/if}
    {/if}
   </main>
</div>
{/if}

<style>
  .shell { display: flex; min-height: 100svh; }
  main { flex: 1; min-width: 0; padding: 24px 32px 40px; }
  .page-header { display: flex; align-items: center; justify-content: space-between; gap: 16px; min-height: 40px; }
  .browse { font-size: 13px; }
  .rows { display: grid; gap: 30px; margin-top: 32px; }
  section { min-width: 0; }
  h2 { margin: 0 0 16px 12px; }
  .poster-row { display: grid; grid-auto-flow: column; grid-auto-columns: 160px; gap: 28px; overflow-x: auto; padding: 8px 12px 16px; overscroll-behavior-x: contain; scrollbar-width: thin; scrollbar-color: #ffffff25 transparent; }
  .catalog { margin-top: 32px; }
  .status { color: var(--muted); font-size: 13px; }
  .skip-link { position: absolute; top: 8px; left: 16px; padding: 10px; background: var(--surface); color: var(--text); transform: translateY(-200%); z-index: 1; }
  .skip-link:focus { transform: none; }
  @media (max-width: 600px) {
    .shell { flex-direction: column; }
    main { padding: 20px 16px 32px; }
    .page-header { flex-wrap: wrap; gap: 12px; }
    .rows { margin-top: 28px; }
    h2 { font-size: 20px; margin-left: 8px; }
    .poster-row { padding-left: 8px; padding-right: 8px; }
  }
</style>
