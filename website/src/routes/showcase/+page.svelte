<script lang="ts">
  import { shells } from '../../lib/showcase';
</script>

<svelte:head>
  <title>Showcase · Screenroom</title>
  <meta name="description" content="Screenshots of the eight Screenroom apps for Web, mobile, desktop, and TV." />
</svelte:head>

<header>
  <p class="eyebrow">Client previews</p>
  <h1>Showcase</h1>
  <p>Explore eight Screenroom apps. Browse screenshots use an example catalog, and playback screenshots use Jellyfin.</p>
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
