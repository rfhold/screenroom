<script lang="ts">
  let { more, loading, error = '', load }: { more: boolean; loading: boolean; error?: string; load: () => void } = $props();
  let sentinel: HTMLDivElement;
  let automatic = $state(0);
  $effect(() => {
    if (!more || loading || error || automatic >= 2 || !sentinel || typeof IntersectionObserver === 'undefined') return;
    const observer = new IntersectionObserver(entries => { if (entries.some(entry => entry.isIntersecting)) { automatic++; load(); } });
    observer.observe(sentinel);
    return () => observer.disconnect();
  });
</script>
<svelte:window onscroll={() => automatic = 0} />
<div bind:this={sentinel} class="pagination">
  {#if error}<p role="alert">{error}</p>{/if}
  {#if more}<button disabled={loading} onclick={load}>{loading ? 'Loading…' : error ? 'Retry load more' : 'Load more'}</button>{/if}
</div>
<style>.pagination { margin: 24px 0; }</style>
