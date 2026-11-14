<script lang="ts">
  import Poster from './Poster.svelte';
  import People from './People.svelte';
  import type { Item } from '../core/models';
  let { item }: { item: Item } = $props();
  const score = $derived(item.rating ? item.rating.count === 0 ? 'Unrated' : `${item.rating.value.toFixed(1)}/${item.rating.scale}` : '—');
</script>

<article class="detail" aria-label={`${item.title} details`}>
  <div class="detail-header">
  <div class="art"><Poster {item} /></div>
  <div class="information">
    <p class="muted">{item.kind} · {item.year}</p><h1>{item.title}</h1>
    <p class="muted">{item.meta}{#if item.kind === 'Episode'} · Aired {item.releaseDate}{/if}</p>
    <div class="metadata" role="group" aria-label="Rating and catalogs">
      <div class="badges">
      {#each item.providers as provider}
        {#if provider.url}
          <a class="badge" href={provider.url} target="_blank" rel="noopener noreferrer" aria-label={`${provider.provider}: ${provider.status}${item.rating?.provider === provider.provider ? `; user rating ${score}` : ''}`} title={`${provider.provider}: ${provider.status}`}>
            <span>{provider.provider}</span>{#if item.rating?.provider === provider.provider}<svg class="rating-icon" viewBox="0 0 24 24" aria-hidden="true"><path d="m12 3 2.8 5.7 6.2.9-4.5 4.4 1.1 6.2-5.6-3-5.6 3 1.1-6.2L3 9.6l6.2-.9Z" /></svg><strong>{score}</strong>{/if}
          </a>
        {:else}<span class="badge" role="status" aria-label={`${provider.provider}: ${provider.status}`} title={`${provider.provider}: ${provider.status}`}><svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="8" /><path d="M12 7v6m0 3v1" /></svg><span>{provider.provider} · {provider.status}</span></span>{/if}
      {/each}
      {#if item.rating && !item.providers.some(provider => provider.provider === item.rating?.provider && provider.url)}<span class="badge" aria-label={`${item.rating.provider} user rating ${score}`}>{item.rating.provider} <strong>{score}</strong></span>{/if}
      {#key item.id}<details class="metadata-info">
        <summary aria-label="Rating and catalog details" title="Rating and catalog details"><svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="9" /><path d="M12 11v6m0-10v1" /></svg></summary>
        <div class="info-content">
          {#if item.rating}<p>{item.rating.provider} user rating: {item.rating.count === 0 ? 'Unrated' : `${item.rating.value.toFixed(1)} / ${item.rating.scale}`} · {item.rating.count === null ? 'Vote count unavailable' : `${item.rating.count.toLocaleString('en-US')} votes`}.</p><p>Snapshot · {item.rating.retrievedAt.slice(0, 10)} · Not live.</p>{/if}
          {#each item.providers as provider}<p>{provider.provider} · {provider.status}</p>{/each}
          <p>A catalog listing does not mean this title is available to play.</p>
        </div>
      </details>{/key}
      </div>
    </div>
    <p>{item.overview}</p>
  </div>
  </div>
    <People title="Cast" people={item.cast} />
    <People title="Crew" people={item.crew} />
</article>

<style>
  .detail { max-width: 1120px; container-type: inline-size; padding-top: 8px; }
  .detail-header { display: grid; grid-template-columns: 1fr; gap: 20px; align-items: start; }
  .art { width: 160px; }
  .information { min-width: 0; }
  h1 { margin: 8px 0; font-size: 28px; }
  p { font-size: 14px; line-height: 1.6; }
  .muted { color: var(--muted); font-size: 12px; }
  .metadata { position: relative; margin-block: 18px; min-height: 44px; }
  .badges { display: flex; flex-wrap: wrap; align-items: start; gap: 8px; }
  .badge { display: inline-flex; align-items: center; justify-content: center; gap: 7px; min-height: 44px; padding: 8px 10px; border: 1px solid var(--border); border-radius: 9px; background: #ffffff05; font-size: 12px; color: var(--muted); white-space: nowrap; box-sizing: border-box; }
  a.badge { text-decoration: none; margin: 0; color: var(--text); }
  .badge strong { font-size: 12px; font-variant-numeric: tabular-nums; }
  .badge svg { width: 16px; height: 16px; flex: none; fill: none; stroke: currentColor; stroke-width: 1.6; }
  .badge .rating-icon { fill: #d4b67b; stroke: none; }
  summary svg { fill: none; stroke: currentColor; stroke-width: 1.6; stroke-linecap: round; stroke-linejoin: round; }
  .metadata-info { display: contents; }
  summary { display: grid; place-items: center; width: 44px; height: 44px; border: 1px solid var(--border); border-radius: 9px; cursor: pointer; color: var(--muted); list-style: none; }
  summary::-webkit-details-marker { display: none; }
  summary svg { width: 20px; height: 20px; }
  summary:hover, details[open] summary { background: #ffffff09; color: var(--text); }
  summary:focus-visible, a.badge:focus-visible { outline: 2px solid var(--focus); outline-offset: 3px; }
  .info-content { flex-basis: 100%; border: 1px solid var(--border); border-radius: 9px; padding: 10px 14px; color: var(--muted); }
  .info-content p { font-size: 12px; margin: 5px 0; overflow-wrap: anywhere; }
  a { color: var(--focus); font-size: 13px; display: inline-block; padding-block: 5px; margin-right: 15px; }
  @container (min-width: 640px) { .detail-header { grid-template-columns: 200px minmax(0, 1fr); gap: 30px; } .art { width: 200px; } h1 { font-size: 30px; } }
</style>
