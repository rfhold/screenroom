<script lang="ts">
  import { shells } from '../../lib/showcase';
</script>

<svelte:head>
  <title>Showcase · Screenroom</title>
  <meta name="description" content="Five E2E flows across eight Screenroom client shells and nine viewport profiles, with a shared Rust core and local mock-server catalog." />
</svelte:head>

<header>
  <p class="eyebrow">Client previews</p>
  <h1>Showcase</h1>
  <p>Eight client shells, nine viewport profiles, and five E2E flows: Home, expanded sidebar, Movies library, Search, and title details.</p>
  <p>The Web showcase includes desktop and mobile browser views. Each card starts with Home.</p>
  <p class="preview-note">These historical captures retain their original fixture-backed mock-server provenance. Current clients use a persistent local backend with offline adapters; these images do not establish that cutover. Production providers, authentication, and playback are not available. This page serves static screenshots only.</p>
</header>

<ul class="shell-grid" aria-label="Client shells">
  {#each shells as shell}
    <li class="shell-card">
      <a class="thumbnail" href={`/showcase/${shell.slug}/`} aria-label={`View ${shell.name} screenshots`}>
        <img src={shell.flows[0].screenshots[0].src} alt="" loading="lazy" />
      </a>
      <h2><a href={`/showcase/${shell.slug}/`}>{shell.name}</a></h2>
      <p>{shell.description}</p>
    </li>
  {/each}
</ul>
