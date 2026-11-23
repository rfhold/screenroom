<script lang="ts">
  let { name }: { name: 'back' | 'play' | 'pause' | 'rewind' | 'forward' | 'volume' | 'muted' | 'fullscreen' | 'shrink' | 'settings' | 'close' } = $props();
  const paths = {
    back: 'm14 6-6 6 6 6M8 12h12',
    play: 'm8 5 11 7-11 7Z',
    pause: 'M8 5v14M16 5v14',
    rewind: 'M7 3 3 7l4 4M3 7h10a8 8 0 1 1-7 12',
    forward: 'm17 3 4 4-4 4M21 7H11a8 8 0 1 0 7 12',
    volume: 'M11 5 6 9H3v6h3l5 4ZM15 8a6 6 0 0 1 0 8M18 5a10 10 0 0 1 0 14',
    muted: 'M11 5 6 9H3v6h3l5 4ZM16 9l6 6M22 9l-6 6',
    fullscreen: 'M9 3H3v6M15 3h6v6M3 15v6h6M21 15v6h-6',
    shrink: 'M3 9h6V3M21 9h-6V3M9 21v-6H3M15 21v-6h6',
    settings: 'M4 7h16M4 17h16M8 4v6M16 14v6',
    close: 'm6 6 12 12M18 6 6 18',
  };
</script>

<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d={paths[name]} />{#if name === 'rewind' || name === 'forward'}<text x="12" y="16" text-anchor="middle" fill="currentColor" stroke="none" font-size="8" font-family="sans-serif">30</text>{/if}</svg>

<style>svg { display: block; width: 20px; height: 20px; flex: none; }</style>
