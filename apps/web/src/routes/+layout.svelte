<script lang="ts">
  import Home from '../lib/components/Home.svelte';
  import { page } from '$app/state';
  import '../lib/theme.css';
  let { children } = $props();
</script>

{#if page.route.id !== null}<Home />{/if}
{@render children()}
