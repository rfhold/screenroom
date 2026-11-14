<script lang="ts">
  import type { Library } from '../core/models';
  import JellyfinProfile from './JellyfinProfile.svelte';
  import { operatorConfig } from '../core/operator-config';
  import { href, libraryHref, type Location } from '../core/navigation';
   let { location, libraries, expanded = $bindable(false), refresh }: { location: Location; libraries: Library[]; expanded: boolean; refresh: () => void } = $props();
  const searchHref = $derived(href(location, { screen: 'search', query: '', types: undefined, title: undefined }));
</script>

<aside class:expanded aria-label="Sidebar">
  <div class="rail-top"><button class="toggle" aria-label={expanded ? 'Collapse sidebar' : 'Expand sidebar'} aria-expanded={expanded} aria-controls="sidebar-content" onclick={() => expanded = !expanded}>
    <svg viewBox="0 0 24 24" fill="none" aria-hidden="true"><rect x="3" y="4" width="18" height="16" rx="3" /><path d="M9 4v16M5.5 8h1M5.5 12h1" /></svg>
  </button></div>
  <div id="sidebar-content">
    <nav aria-label="Main menu">
      <a href="/" aria-label="Home" aria-current={location.screen === 'home' ? 'page' : undefined}><span class="icon" aria-hidden="true">⌂</span><span class="label">Home</span></a>
      <a href={searchHref} aria-label="Search titles" aria-current={location.screen === 'search' ? 'page' : undefined}><span class="icon" aria-hidden="true">⌕</span><span class="label">Search titles</span></a>
      {#if operatorConfig.enabled}<a href="/?screen=maintenance" aria-label="Maintenance" aria-current={location.screen === 'maintenance' ? 'page' : undefined}><span class="icon" aria-hidden="true"><svg viewBox="0 0 24 24" fill="none"><path d="m4 20 8-8m-5-8 3 3-3 3-3-3a6 6 0 0 0 8 7l6 6a2 2 0 0 0 3-3l-6-6a6 6 0 0 0-8-7Z" /></svg></span><span class="label">Maintenance</span></a>{/if}
      {#if expanded}<h2>Libraries</h2>{/if}
      {#each libraries as library}
        <a href={libraryHref(library, location.sourceId)} aria-label={library.title} aria-current={location.screen === 'browse' && location.library.id === library.id ? 'page' : undefined}><span class="icon" aria-hidden="true">▤</span><span class="label">{library.title}</span></a>
      {/each}
    </nav>
    {#if expanded}
      <JellyfinProfile onChanged={refresh} />
      <div class="actions"><button onclick={refresh}>Refresh</button></div>
    {/if}
  </div>
</aside>

<style>
  aside { position: sticky; top: 0; align-self: flex-start; height: 100svh; overflow-y: auto; width: 64px; min-width: 64px; background: var(--surface); border-right: 1px solid var(--border); padding: 16px 10px; }
  aside.expanded { width: 264px; min-width: 264px; }
  .rail-top { height: 48px; }
  .toggle { display: flex; align-items: center; justify-content: center; width: 40px; height: 40px; padding: 8px; background: transparent; border-color: transparent; }
  svg { width: 24px; height: 24px; stroke: currentColor; stroke-width: 1.5; }
  nav { display: grid; gap: 6px; margin-top: 16px; }
  nav a { display: flex; align-items: center; gap: 12px; min-height: 42px; padding: 10px; text-decoration: none; color: var(--muted); border-radius: 8px; font-size: 14px; }
  nav a:hover, nav a[aria-current] { background: #ffffff0c; color: var(--text); }
  .icon { width: 20px; flex: 0 0 20px; text-align: center; font-size: 18px; }
  .label { display: none; }
  .expanded .label { display: inline; }
   h2 { color: var(--muted); font-size: 12px; font-weight: 500; }
  h2 { margin: 20px 10px 6px; }
  .actions { display: grid; gap: 8px; margin-top: 20px; }
  @media (max-width: 600px) {
    aside, aside.expanded { position: static; align-self: stretch; height: auto; overflow: visible; width: 100%; min-width: 0; border-right: 0; border-bottom: 1px solid var(--border); padding: 12px 16px; }
    aside:not(.expanded) { display: flex; align-items: start; gap: 12px; }
    .rail-top { height: 40px; flex: 0 0 40px; }
    aside:not(.expanded) nav { display: flex; gap: 2px; margin: 0; }
    aside:not(.expanded) nav a { padding: 8px; }
    aside.expanded #sidebar-content { max-width: 480px; }
  }
</style>
