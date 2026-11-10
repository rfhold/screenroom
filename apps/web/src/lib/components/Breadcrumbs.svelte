<script lang="ts">
  import { href, libraryHref, type Location } from '../core/navigation';
  let { location }: { location: Location } = $props();
  const browse = $derived({ ...location, screen: 'browse' as const, title: undefined });
</script>

<nav aria-label="Breadcrumb"><ol>
  <li><a href="/" aria-current={location.screen === 'home' && !location.title ? 'page' : undefined}>Home</a></li>
    {#if location.screen !== 'home'}
      <li><a href={libraryHref(location.library, location.sourceId)} aria-current={!location.path.length && location.screen === 'browse' && !location.title ? 'page' : undefined}>{location.library.title}</a></li>
      {#each location.path as item, index}
        <li><a href={href(browse, { path: location.path.slice(0, index + 1) })} aria-current={index === location.path.length - 1 && location.screen === 'browse' && !location.title ? 'page' : undefined}>{item.title}</a></li>
       {/each}
    {/if}
    {#if location.screen === 'search'}<li><a href={href(location, { title: undefined })} aria-current={!location.title ? 'page' : undefined}>Search</a></li>{/if}
    {#if location.title}<li><a href={href(location)} aria-current="page">{location.title.title}</a></li>{/if}
</ol></nav>

<style>
  nav { margin-bottom: 28px; min-height: 20px; }
  ol { display: flex; flex-wrap: wrap; gap: 8px; padding: 0; margin: 0; list-style: none; font-size: 13px; line-height: 1.6; }
  li { display: flex; align-items: baseline; gap: 8px; min-width: 0; }
  li + li::before { content: ''; width: 5px; height: 5px; border-top: 1px solid var(--muted); border-right: 1px solid var(--muted); transform: rotate(45deg); align-self: center; flex: none; margin-right: 2px; }
  a { color: var(--muted); text-decoration: none; overflow-wrap: anywhere; }
  a:hover { color: var(--text); text-decoration: underline; }
  a[aria-current] { color: var(--text); }
</style>
