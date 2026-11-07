<script lang="ts">
  let { children } = $props();
</script>

<nav aria-label="Main">
  <a href="/">Screenroom</a>
  <a href="/download">Download</a>
  <a href="/docs">Docs</a>
  <a href="/about">About</a>
</nav>

{@render children()}
