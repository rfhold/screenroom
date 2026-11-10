<script lang="ts">
  import { client, item, artwork, failure, scope } from '../core/client';
  import { collectPages } from '../core/pagination';
  import type { Item } from '../core/models';
  import type { CatalogInfo, Artwork } from '../../../../../target/client-core/wasm/screenroom_client_core';
  import RemoteImage from './RemoteImage.svelte';
  import ItemRights from './ItemRights.svelte';
  import { MetadataOperation, MetadataCancelled } from '../core/operation.mjs';
  let { info, refreshVersion }: { info: CatalogInfo; refreshVersion: number } = $props();
  let titles = $state<Item[]>([]);
  let artworks = $state<Artwork[]>([]);
  let open = $state(false);
  let error = $state('');
  let generation = 0;
  $effect(() => {
    refreshVersion;
    const current = ++generation;
    if (!open) return;
    const operation = new MetadataOperation(() => current === generation);
    error = '';
    titles = []; artworks = [];
    void operation.read(client).then(c => operation.read(() => Promise.all([collectPages(async page => {
      const response = await c.list_library_items(scope(), page);
      return { entries: response.items, next_page_token: response.next_page_token };
    }, () => current === generation, entry => entry.id, operation), collectPages(async page => {
      const response = await c.list_artwork(scope(), page);
      return { entries: response.artwork, next_page_token: response.next_page_token };
    }, () => current === generation, entry => entry.id, operation)]))).then(([records, arts]) => { operation.check(); const mapped = records.map(record => { operation.check(); return item(record); }); operation.check(); if (current === generation) { titles = mapped; artworks = arts; } }).catch(cause => { if (current === generation && !(cause instanceof MetadataCancelled)) { operation.stop(cause instanceof Error ? cause : new Error(String(cause))); error = failure(cause); } });
    return () => { generation++; operation.cancel(); };
  });
</script>

<details class="credits" bind:open>
  <summary>Data & artwork credits</summary>
  <div class="content">
    <h2>Data & artwork credits</h2>
    {#each info.branding.filter(art => art.kind === 'logo') as logo (logo.id)}
      <div class="brand"><RemoteImage art={artwork(logo)!} /></div>
    {/each}
    {#each info.notices as notice (notice.id)}
      <section data-section={notice.section}>
        {#if notice.title}<h3>{notice.title}</h3>{/if}
        <p>{notice.text}</p>
        {#if notice.link_url}<a href={notice.link_url} target="_blank" rel="noopener noreferrer">{notice.link_label || notice.link_url}</a>{/if}
      </section>
    {/each}
    {#if info.retrieved_at}<p>Retrieved {info.retrieved_at}</p>{/if}
    <h3>Artwork</h3>
    <ul>{#each artworks as art (art.id)}<li><strong>{art.label}</strong><br />{art.creator}<br /><a href={art.license_url} target="_blank" rel="noopener noreferrer">{art.license}</a> · <a href={art.source} target="_blank" rel="noopener noreferrer">Source & provenance</a></li>{/each}</ul>
    <h3>Title & metadata rights</h3>
    {#if error}<p role="alert">{error}</p><button onclick={() => refreshVersion++}>Retry credits</button>{/if}
    <ul>{#each titles as title (title.id)}<li><strong>{title.title}</strong><ItemRights item={title} /></li>{/each}</ul>
  </div>
</details>

<style>
  .credits { border-top: 1px solid var(--border); margin-top: 40px; padding-top: 20px; color: var(--muted); font-size: 12px; line-height: 1.7; }
  summary { cursor: pointer; width: fit-content; padding: 4px 0; }
  .content { max-width: 1120px; padding-top: 12px; overflow-wrap: anywhere; }
  h2, h3, strong { color: var(--text); }
  h2 { font-size: 20px; }
  h3 { font-size: 16px; margin-top: 24px; }
  .brand { width: 88px; margin-block: 8px; }
  .brand :global(img) { height: auto; object-fit: contain; }
  a { color: var(--focus); }
  ul { padding-left: 20px; }
  li { margin-bottom: 18px; }
</style>
