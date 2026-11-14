<script lang="ts">
  import Poster from './Poster.svelte';
  import type { Item } from '../core/models';
  import { cardLabels } from '../core/card-labels';
  let { item, href, seasonContext = false }: { item: Item; href: string; seasonContext?: boolean } = $props();
  const labels = $derived(cardLabels(item, seasonContext));
</script>

<a class="card" {href} aria-label={`${labels.heading} · ${item.kind} · ${item.kind === 'Episode' ? labels.subtitle : item.meta}${item.watched === true ? ' · Watched' : ''}${item.progress ? ` · Playback progress ${item.progress}%` : ''}`}>
  <Poster {item} heading={labels.heading} />
  <span class="title">{#if labels.number}{#if labels.primary}<span class="series-name">{labels.primary}</span>{/if}<span class="episode-number">{labels.number}</span>{:else}{labels.primary}{/if}</span>
  <span class="metadata">{labels.subtitle}</span>
</a>

<style>
  .card { display: block; text-decoration: none; padding: 0; border: 0; background: transparent; text-align: left; min-width: 0; border-radius: 10px; color: inherit; }
  .card:hover :global(.poster) { filter: brightness(1.15); }
  .title { display: block; font-weight: 600; font-size: 14px; margin-top: 10px; line-height: 1.35; }
  .episode-number { color: var(--muted); font-size: 12px; font-weight: 400; }
  .series-name + .episode-number { margin-left: 6px; }
  .metadata { display: block; color: var(--muted); font-size: 12px; margin-top: 4px; line-height: 1.4; }
</style>
