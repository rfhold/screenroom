<script lang="ts">
  import Poster from './Poster.svelte';
  import type { Item } from '../core/models';
  let { item, href }: { item: Item; href: string } = $props();
</script>

<a class="card" {href} aria-label={`${item.title} · ${item.kind} · ${item.meta}${item.progress ? ` · ${item.progress}% watched` : ''}`}>
  <Poster {item} />
  <span class="title">{item.title}</span>
  <span class="metadata">{item.kind === 'Movie' ? item.year : item.meta}</span>
</a>

<style>
  .card { display: block; text-decoration: none; padding: 0; border: 0; background: transparent; text-align: left; min-width: 0; border-radius: 10px; color: inherit; }
  .card:hover :global(.poster) { filter: brightness(1.15); }
  .title { display: block; font-weight: 600; font-size: 14px; margin-top: 10px; line-height: 1.35; }
  .metadata { display: block; color: var(--muted); font-size: 12px; margin-top: 4px; line-height: 1.4; }
</style>
