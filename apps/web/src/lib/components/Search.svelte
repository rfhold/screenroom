<script lang="ts">
  import { onDestroy, onMount } from 'svelte';
  import { beforeNavigate, goto } from '$app/navigation';
  import { href, itemHref, type Location } from '../core/navigation';
  import { defaultTypes, loadTypes, saveTypes, search, type Types, type Methods, type Source } from '../core/search';
  import { client, newClient, failure, saveSettings } from '../core/client';
  import type { SearchCapabilities } from '../../../../../target/client-core/wasm/screenroom_client_core';
  import type { Item } from '../core/models';
  import PosterCard from './PosterCard.svelte';
  import { MetadataOperation, MetadataCancelled } from '../core/operation.mjs';

  let { location: route, capabilities, operation, refreshVersion = 0, onReadComplete, onRetry }: { location: Location; capabilities: SearchCapabilities; operation: MetadataOperation; refreshVersion?: number; onReadComplete: (error?: string) => void; onRetry: () => void } = $props();
  let pending = $state<Location | null>(null);
  let revision = 0;
  let generation = 0;
  let active = true;
  let queue = Promise.resolve();
  const location = $derived(pending ?? route);
  let savedTypes = $state(defaultTypes());
  const types = $derived.by(() => {
    const selected = Object.fromEntries(Object.entries(location.types ?? savedTypes).map(([key, value]) => [key, value && capabilities.types.includes(key)])) as Types;
    if (!Object.values(selected).some(Boolean) && capabilities.types[0]) selected[capabilities.types[0] as keyof Types] = true;
    return selected;
  });
  const methods = $derived.by(() => {
    const selected = Object.fromEntries(Object.entries(location.methods).map(([key, value]) => [key, value && capabilities.methods.includes(key === 'titleOnly' ? 'title_only' : key)])) as Methods;
    if (![selected.text, selected.fuzzy, selected.semantic].some(Boolean)) {
      const first = capabilities.methods.find(method => method !== 'title_only');
      if (first) selected[first as keyof Methods] = true;
    }
    return selected;
  });
  let settled = $state('');
  let options = $state(false);
  let input: HTMLInputElement;
  let settingsButton: HTMLButtonElement;
  let results = $state<Item[]>([]);
  let searching = $state(false);
  let error = $state('');
  let searchStatus = $state('');
  const semanticDescription = $derived(capabilities?.semantic_description ?? '');
  const maximumQueryLength = $derived(capabilities?.maximum_query_length ?? 160);
  const typeLabels: Record<string, string> = { Movie: 'Movies', Series: 'Shows', Episode: 'Episodes' };
  const methodLabels: Record<string, string> = { title_only: 'Title only', text: 'Text (titles and metadata)', fuzzy: 'Fuzzy titles', semantic: 'Semantic' };
  const methodKeys: Record<string, keyof Methods> = { title_only: 'titleOnly', text: 'text', fuzzy: 'fuzzy', semantic: 'semantic' };
  const sourceLabels: Record<string, string> = { all: 'All catalogs', tmdb: 'TMDB', jellyfin: 'Jellyfin' };
  let searchGeneration = 0;
  let lastRefresh = -1;
  let searchClient: Awaited<ReturnType<typeof client>> | undefined;

  onMount(() => {
    input.focus();
    const budget = operation;
    void loadTypes(budget).then(value => { budget.check(); if (!active) return; savedTypes = value; if (!route.types && revision === 0) navigate({ types: value }, true); }).catch(cause => { if (active && budget.active() && !(cause instanceof MetadataCancelled)) error = failure(cause); });
  });
  onDestroy(() => { active = false; searchGeneration++; searchClient?.cancel_all(); });
  beforeNavigate(navigation => {
    if (navigation.type !== 'goto') { generation++; revision++; pending = null; }
  });
  $effect(() => { if (pending && href(route) === href(pending)) pending = null; });
  $effect(() => {
    const query = location.query;
    const library = location.library; const selectedTypes = types; const selectedMethods = methods; const source = location.source;
    if (lastRefresh !== refreshVersion) { searchClient?.invalidate_reads(); lastRefresh = refreshVersion; }
    const current = ++searchGeneration;
    searchClient?.cancel_all();
    results = []; searching = !!query.trim(); error = '';
    const budget = operation;
    if (href(location) !== href(route)) return;
    const timer = setTimeout(async () => {
      try {
        budget.check();
        if (query.trim()) {
          searchClient ??= await budget.read(newClient);
          if (!active) { searchClient.free(); searchClient = undefined; return; }
          if (current !== searchGeneration) return;
           const response = await search(query, library, selectedTypes, selectedMethods, source, searchClient, () => active && current === searchGeneration, location.sourceId, budget);
           budget.check();
           if (active && current === searchGeneration) { results = response.items; searchStatus = `Effective methods: ${response.effective}.${response.degradation ? ` ${response.degradation}` : ''}`; }
        }
        budget.check();
        if (active && current === searchGeneration) { settled = query; searching = false; onReadComplete(); }
      } catch (cause) { if (active && current === searchGeneration && budget.active() && !(cause instanceof MetadataCancelled)) { error = failure(cause); searching = false; onReadComplete(error); } }
    }, Math.max(0, Math.min(350, budget.deadline - budget.now())));
    return () => { clearTimeout(timer); searchGeneration++; };
  });
  function navigate(changes: Partial<Location>, replace = false) {
    const next = { ...location, types, ...changes };
    void saveSettings({ source: next.source, types: { movie: next.types.Movie, series: next.types.Series, episode: next.types.Episode }, methods: { title_only: next.methods.titleOnly, text: next.methods.text, fuzzy: next.methods.fuzzy, semantic: next.methods.semantic } }).catch(cause => error = failure(cause));
    pending = next;
    const current = ++revision;
    const batch = generation;
    queue = queue.then(async () => {
      if (!active || batch !== generation || (replace && current !== revision)) return;
      await goto(href(next), { replace, reset: false });
    });
  }
  function updateQuery(query: string) { navigate({ query: query.slice(0, maximumQueryLength) }, true); }
  function toggleType(key: keyof Types) {
    if (types[key] && Object.values(types).filter(Boolean).length === 1) return;
    const next = { ...types, [key]: !types[key] };
    savedTypes = next;
    void saveTypes(next).catch(cause => error = failure(cause));
    navigate({ types: next });
  }
  function toggleMethod(key: keyof Methods) {
    if (key === 'semantic' && methods.titleOnly) return;
    if (key !== 'titleOnly' && methods[key] && [methods.text, methods.fuzzy, methods.semantic].filter(Boolean).length === 1) return;
    const next = { ...methods, [key]: !methods[key] };
    if (next.titleOnly) { next.semantic = false; if (!next.text && !next.fuzzy) next.text = true; }
    navigate({ methods: next });
  }
</script>

<svelte:window onkeydown={(event) => { if (event.key === 'Escape') { event.preventDefault(); if (options) { options = false; settingsButton.focus(); } } }} />
<h1>Search</h1>
<div class="search-toolbar" aria-busy={pending !== null || searching}>
  <div class="query">
    <input bind:this={input} type="search" aria-label={`Search ${location.library.title}`} placeholder={`Search ${location.library.title}`} maxlength={maximumQueryLength} value={location.query} oninput={(event) => updateQuery(event.currentTarget.value)} />
    {#if location.query}<button onclick={() => { updateQuery(''); settled = ''; input.focus(); }}>Clear query</button>{/if}
    <button bind:this={settingsButton} class="settings" aria-label="Search settings" aria-expanded={options} aria-controls="search-settings" onclick={() => options = !options}>
      <svg viewBox="0 0 24 24" fill="none" aria-hidden="true"><path d="M4 7h16M4 17h16" /><circle cx="9" cy="7" r="3" /><circle cx="15" cy="17" r="3" /></svg>
      <span>Settings</span>
    </button>
  </div>
  {#if options}
    <section id="search-settings" class="settings-panel" aria-label="Search settings">
      <div class="source-control"><h2><label for="catalog-source">Catalog</label></h2><select id="catalog-source" value={location.source} onchange={(event) => navigate({ source: event.currentTarget.value as Source })}>{#each capabilities?.sources ?? [] as source}<option value={source}>{sourceLabels[source] ?? source}</option>{/each}</select><span class="muted">Catalog membership, not playback availability.</span></div>
      <section aria-label="Media types; keep at least one selected">
        <h2>Media types</h2>
        <div class="controls">
          {#each capabilities?.types ?? [] as key}
            <label><input type="checkbox" checked={types[key as keyof Types]} disabled={types[key as keyof Types] && Object.values(types).filter(Boolean).length === 1} onchange={() => toggleType(key as keyof Types)} />{typeLabels[key] ?? key}</label>
          {/each}
        </div>
      </section>
      <section aria-label="Search methods">
        <h2>Search methods</h2>
        <p class="muted">Keep at least one method selected. {semanticDescription}</p>
        {#each capabilities?.methods ?? [] as method}
          {@const key = methodKeys[method]}
          <label class="option"><input type="checkbox" checked={methods[key]} disabled={(key === 'semantic' && methods.titleOnly) || (key !== 'titleOnly' && methods[key] && [methods.text, methods.fuzzy, methods.semantic].filter(Boolean).length === 1)} onchange={() => toggleMethod(key)} /><span>{method === 'text' && methods.titleOnly ? 'Text (titles only)' : methodLabels[method] ?? method}{#if key === 'titleOnly'}<small>Limit text to titles and turn off Semantic.</small>{/if}</span></label>
        {/each}
      </section>
    </section>
  {/if}
</div>
<div aria-live="polite" aria-atomic="true" class="result-status">
  {#if error}<span role="alert">{error}</span><button onclick={onRetry}>Retry search</button>
  {:else if !location.query.trim()}Enter a title to search {location.library.title}.
  {:else if searching || location.query !== settled}Searching titles…
  {:else if results.length === 0}No matches in {location.library.title}. Try another search.
  {:else}{results.length} {results.length === 1 ? 'result' : 'results'} in {location.library.title}{/if}
</div>
{#if !searching && !error && location.query.trim() && searchStatus}<p role="status">{searchStatus}</p>{/if}
{#if !searching && !error && location.query.trim() && location.query === settled}
  <div class="grid" aria-label="Search results">{#each results as item (item.id)}<PosterCard {item} href={itemHref(location, item)} />{/each}</div>
{/if}

<style>
  .search-toolbar { margin-top: 24px; max-width: 800px; }
  .query { display: flex; align-items: stretch; gap: 10px; }
  .query input { flex: 1; width: 100%; }
  .settings { display: flex; align-items: center; justify-content: center; gap: 8px; }
  .settings svg { width: 20px; height: 20px; stroke: currentColor; stroke-width: 1.5; }
  .settings circle { fill: var(--surface); }
  .controls { display: flex; flex-wrap: wrap; align-items: center; gap: 20px; margin-top: 16px; }
  .controls label { display: flex; align-items: center; gap: 6px; font-size: 14px; }
  .source-control { display: flex; align-items: center; flex-wrap: wrap; gap: 10px; font-size: 13px; }
  .source-control label { font-weight: 600; font-size: 16px; }
  .source-control span { flex-basis: 100%; font-size: 12px; }
  select { font: inherit; color: var(--text); background: var(--surface); border: 1px solid var(--border); border-radius: 8px; padding: 9px 12px; }
  .result-status { color: var(--muted); margin: 28px 0 20px; font-size: 14px; }
  .settings-panel { margin-top: 20px; padding: 20px; border: 1px solid var(--border); border-radius: 12px; background: var(--surface); }
  .settings-panel h2 { font-size: 16px; margin: 0; }
  .settings-panel p { font-size: 12px; }
  .settings-panel > section { border-top: 1px solid var(--border); padding-top: 20px; margin-top: 22px; }
  .option { display: flex; align-items: start; gap: 10px; margin-top: 20px; font-size: 14px; }
  .option small { display: block; font-size: 12px; line-height: 1.5; color: var(--muted); margin-top: 6px; }
  @media (max-width: 600px) { .query { flex-wrap: wrap; } .query input { flex-basis: calc(100% - 60px); } .settings span { display: none; } .query > button:not(.settings) { order: 3; } }
</style>
