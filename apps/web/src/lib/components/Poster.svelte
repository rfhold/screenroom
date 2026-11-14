<script lang="ts">
  import type { Item } from '../core/models';
  import RemoteImage from './RemoteImage.svelte';
  import { cardLabels } from '../core/card-labels';
  let { item, heading }: { item: Item; heading?: string } = $props();
</script>

<div class="poster" style={`--ink: ${item.colors[0]}; --light: ${item.colors[1]}`}>
  {#if item.artwork}<RemoteImage art={item.artwork} />{:else}
  <span class="orbit" aria-hidden="true"></span><span class="landscape" aria-hidden="true"></span>
  <span class="art-title">{heading ?? cardLabels(item).heading}</span>
  <span class="art-bottom">POSTER NOT INCLUDED</span>
  {/if}
  {#if item.watched === true}<span class="watched" title="Watched"><svg viewBox="0 0 24 24" aria-hidden="true"><path d="m5 12 4 4L19 6" /></svg><span class="watched-label">Watched</span></span>{/if}
  {#if item.progress}<span class="progress"><span style={`width: ${item.progress}%`}></span></span>{/if}
</div>

<style>
  .poster { aspect-ratio: 2 / 3; position: relative; overflow: hidden; border-radius: 10px; background: linear-gradient(165deg, var(--ink) 20%, var(--light) 160%); box-shadow: inset 0 0 0 1px #ffffff12; isolation: isolate; }
  .orbit { position: absolute; width: 85%; aspect-ratio: 1; border: 1px solid color-mix(in srgb, var(--light), transparent 40%); border-radius: 50%; top: 14%; left: 28%; box-shadow: 0 0 70px color-mix(in srgb, var(--light), transparent 75%), inset 0 0 45px #ffffff08; }
  .landscape { position: absolute; inset: 37% -35% -40%; transform: rotate(-32deg); background: linear-gradient(90deg, #0002, var(--light)); border-radius: 40% 0 0; opacity: .45; }
  .art-title { position: absolute; top: 42%; left: 12%; right: 10%; font-family: Georgia, serif; font-size: 28px; line-height: 1.05; font-weight: 400; color: #fff5e5; text-shadow: 0 2px 20px #0008; }
  .art-bottom { position: absolute; bottom: 18px; left: 12%; font-size: 6px; letter-spacing: 2px; color: #fff9; }
  .watched { position: absolute; top: 8px; right: 8px; display: grid; place-items: center; width: 26px; height: 26px; border-radius: 50%; background: #16251fee; color: #e3f4e9; box-shadow: 0 1px 5px #0005; pointer-events: none; }
  .watched svg { width: 18px; height: 18px; fill: none; stroke: currentColor; stroke-width: 2.5; stroke-linecap: round; stroke-linejoin: round; }
  .watched-label { position: absolute; width: 1px; height: 1px; padding: 0; overflow: hidden; clip-path: inset(50%); white-space: nowrap; }
  .progress { position: absolute; inset: auto 0 0; height: 4px; background: #ffffff35; }
  .progress span { display: block; height: 100%; background: #e9e9e9; }
</style>
