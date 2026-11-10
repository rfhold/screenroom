<script lang="ts">
  import '../lib/theme.css';
  import SiteHeader from '../lib/SiteHeader.svelte';
  import SiteFooter from '../lib/SiteFooter.svelte';
  let { children } = $props();
</script>

<svelte:head><meta name="theme-color" content="#131412" /></svelte:head>

<SiteHeader />
<main id="main-content" tabindex="-1">{@render children()}</main>
<SiteFooter />
