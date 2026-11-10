<script lang="ts">
  import { shells } from '../../../lib/showcase';
  import type { PageData } from './$types';

  let { data }: { data: PageData } = $props();
</script>

<svelte:head>
  <title>{data.shell.name} showcase · Screenroom</title>
  <meta name="description" content={`Screenroom ${data.shell.name} E2E screenshots: Home, expanded sidebar, Movies library, Search, and title details with a local mock-server catalog.`} />
</svelte:head>

<header>
  <p><a href="/showcase/">All client shells</a></p>
  <p class="eyebrow">Client preview</p>
  <h1>{data.shell.name}</h1>
  <p>{data.shell.description}</p>
  <p class="preview-note">These historical captures retain their original fixture-backed mock-server provenance. The current client uses a persistent local backend with offline adapters; these images do not establish that cutover. Production providers, authentication, and playback are not available. This page serves static screenshots only.</p>
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
      <p>{flow.caption}</p>
      {#if data.shell.slug === 'web'}
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
              E2E screenshot · <a href={screenshot.src}>Open {screenshot.label} {flow.name.toLowerCase()} original PNG</a>
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
