<script lang="ts">
  import { shells, type PlaybackState } from '../../../lib/showcase';
  import type { PageData } from './$types';

  let { data }: { data: PageData } = $props();
  const highlightLabels = new Map([
    ['desktop-playing', 'Desktop playback'],
    ['mobile-playing', 'Mobile playback'],
    ['playing', 'Playing'],
    ['desktop-audio-tracks', 'Audio track menu'],
    ['tracks', 'Track menu'],
    ['paused', 'Paused']
  ]);

  function playbackHighlights(states: PlaybackState[]): PlaybackState[] {
    const captures = states.filter((state) => state.status === 'captured');
    const featured = [...highlightLabels.keys()].flatMap((id) => captures.filter((state) => state.id === id));
    return [...new Set([...featured, ...captures])].slice(0, 3);
  }
</script>

<svelte:head>
  <title>{data.shell.name} showcase · Screenroom</title>
  <meta name="description" content={`Screenroom ${data.shell.name} screenshots: browsing and Jellyfin playback.`} />
</svelte:head>

<header>
  <p><a href="/showcase/">All client shells</a></p>
  <p class="eyebrow">Client preview</p>
  <h1>{data.shell.name}</h1>
  <p>{data.shell.description}</p>
  <p>Browse screenshots use an example catalog. Playback screenshots use Jellyfin.</p>
</header>

<nav class="flow-nav" aria-label="Screenshot flows">
  <ul>
    {#each data.shell.flows as flow}
      <li><a href={`#${flow.slug}`}>{flow.name}</a></li>
    {/each}
  </ul>
</nav>

<div class="flows">
  {#each data.shell.flows as flow, flowIndex}
    <section id={flow.slug} aria-labelledby={`${flow.slug}-heading`}>
      <h2 id={`${flow.slug}-heading`}>{flow.name}</h2>
      {#if flow.playback}
        {@const highlights = playbackHighlights(flow.playback.states)}
        <p>{flow.playback.item.title} ({flow.playback.item.year}) · Jellyfin</p>
        {#if flow.playback.outcome === 'blocked'}<p>This run did not demonstrate successful playback.</p>{/if}
        {#if highlights.length}
          <div class="playback-highlights">
            {#each highlights as state (state.id)}
              {@const label = highlightLabels.get(state.id) ?? state.label}
              <figure id={`playback-${state.id}`}>
                <a class="playback-frame" href={state.image} aria-label={`Open original: ${label}`}>
                  <img src={state.image} alt={`${data.shell.name}: ${label}`} loading="lazy" />
                </a>
                <figcaption>{label}</figcaption>
              </figure>
            {/each}
          </div>
        {:else}
          <p>No playback screenshots from this run.</p>
        {/if}
        <details class="capture-details">
          <summary>All playback captures and test notes</summary>
          <p>{flow.playback.provenance}. Runtime: {flow.playback.runtime}. Platform outcome: {flow.playback.outcome}.</p>
          <p>Screenshots document individual states, not full playback acceptance.</p>
          <ul>
            {#each flow.playback.states as state}
              <li id={highlights.includes(state) ? undefined : `playback-${state.id}`}>
                {#if state.image}<a href={state.image}>{state.label}</a>{:else}<strong>{state.label}</strong>{/if}
                <span> · {state.status.replaceAll('-', ' ')} · {state.viewport}</span>
                <p>{state.observation ?? state.reason}</p>
              </li>
            {/each}
          </ul>
        </details>
      {:else}
        <p>{flow.caption}</p>
      {/if}
      {#if data.shell.slug === 'web' && flow.screenshots.length}
        <nav class="viewport-nav" aria-label={`${flow.name} viewports`}>
          <ul>
            {#each flow.screenshots as screenshot}
              <li><a href={`#${flow.slug}-${screenshot.viewport}`}>{screenshot.label}</a></li>
            {/each}
          </ul>
        </nav>
      {/if}
      <div class="captures">
        {#each flow.screenshots as screenshot, screenshotIndex}
          <figure id={screenshot.viewport ? `${flow.slug}-${screenshot.viewport}` : undefined}>
            {#if screenshot.viewport}<h3>{screenshot.label}</h3>{/if}
            <div class="capture-frame">
              <img
                src={screenshot.src}
                alt={screenshot.alt}
                loading={flowIndex === 0 && screenshotIndex === 0 ? 'eager' : 'lazy'}
              />
            </div>
            <figcaption>
              Historical fixture E2E screenshot · <a href={screenshot.src}>Open {screenshot.label} {flow.name.toLowerCase()} original PNG</a>
            </figcaption>
          </figure>
        {/each}
      </div>
    </section>
  {/each}
</div>

<nav class="shell-nav" aria-label="Client shells">
  <ul>
    {#each shells as shell}
      <li>
        <a href={`/showcase/${shell.slug}/`} aria-current={shell.slug === data.shell.slug ? 'page' : undefined}>
          {shell.name}
        </a>
      </li>
    {/each}
  </ul>
</nav>

<style>
  .playback-highlights {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 16rem), 1fr));
    gap: var(--space-3);
  }
  .playback-frame {
    display: block;
    position: relative;
    aspect-ratio: 4 / 3;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--surface-inset);
  }
  .playback-frame img { position: absolute; inset: 0; width: 100%; height: 100%; object-fit: contain; }
  .capture-details { margin-top: var(--space-4); }
  .capture-details summary { cursor: pointer; }
  .capture-details li p { color: var(--text-secondary); }
</style>
