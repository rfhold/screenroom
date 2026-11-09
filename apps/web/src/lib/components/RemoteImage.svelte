<script lang="ts">
  import { acquireArtwork } from '../core/client';
  import type { Artwork } from '../core/models';
  let { art }: { art: Artwork } = $props();
  let source = $state('');
  let missing = $state(false);
  $effect(() => {
    source = ''; missing = false;
    let active = true;
    const image = acquireArtwork(art.url);
    image.loaded.then(url => { if (active) source = url; }, () => { if (active) missing = true; });
    return () => { active = false; image.release(); };
  });
</script>
{#if source}<img src={source} alt={art.label} onerror={() => { missing = true; source = ''; }} />
{:else}<div class="image-state" role="img" aria-label={missing ? `${art.label}: artwork unavailable` : `Loading ${art.label}`}>{missing ? 'Artwork unavailable' : ''}</div>{/if}
<style>
  img, .image-state { display: block; width: 100%; height: 100%; object-fit: cover; border-radius: inherit; }
  .image-state { min-height: 24px; color: var(--muted); font-size: 11px; }
</style>
