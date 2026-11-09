<script lang="ts">
  import type { Item } from '../core/models';
  let { item }: { item: Item } = $props();
</script>
{#each [{ title: 'Title rights', rights: item.rights }, { title: 'Metadata rights', rights: item.metadataRights }] as entry}
  <section aria-label={entry.title}><h2>{entry.title}</h2><p>{entry.rights.label}</p><p>{entry.rights.note}</p>
    {#if entry.rights.attribution}<p>{entry.rights.attribution}</p>{/if}
    {#if entry.rights.source}<a href={entry.rights.source} target="_blank" rel="noopener noreferrer">Rights source</a>{/if}
    {#if entry.rights.licenseUrl} · <a href={entry.rights.licenseUrl} target="_blank" rel="noopener noreferrer">License</a>{/if}
  </section>
{/each}
{#if item.metadataRetrievedAt}<p>Metadata retrieved {item.metadataRetrievedAt}</p>{/if}
{#each item.provenanceNotes as note}<p>{note}</p>{/each}
<style>
  h2 { font-size: 16px; margin: 12px 0 8px; }
  p { font-size: 12px; line-height: 1.6; color: var(--muted); }
  a { color: var(--focus); font-size: 13px; }
</style>
