<script lang="ts">
  import { onMount } from 'svelte';
  import Player from './Player.svelte';
  import type { SourceLibraryScope } from '../../../../../target/client-core/wasm/screenroom_client_core';
  let { itemId, title, context, onClose }: { itemId: string; title: string; context: SourceLibraryScope; onClose: () => void } = $props();
  let screen: HTMLDialogElement;
  let viewport: HTMLDivElement;
  let player: Player;
  let closing = false;
  let fullscreen = $state(false);
  let fullscreenSupported = $state(false);
  let fullscreenError = $state('');
  function close() {
    if (closing) return;
    closing = true;
    onClose();
  }
  async function toggleFullscreen() {
    try {
      fullscreenError = '';
      if (document.fullscreenElement === viewport) await document.exitFullscreen();
      else await viewport.requestFullscreen();
    } catch { fullscreenError = 'Fullscreen is unavailable. You can keep watching here.'; }
  }
  onMount(() => {
    const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const overflow = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
    fullscreenSupported = !!document.fullscreenEnabled && typeof viewport.requestFullscreen === 'function';
    const changed = () => { fullscreen = document.fullscreenElement === viewport; };
    document.addEventListener('fullscreenchange', changed);
    screen.showModal();
    viewport.querySelector<HTMLButtonElement>('[data-player-back]')?.focus({ preventScroll: true });
    return () => {
      document.removeEventListener('fullscreenchange', changed);
      if (document.fullscreenElement === viewport) void document.exitFullscreen().catch(() => {});
      screen.close();
      document.body.style.overflow = overflow;
      if (previousFocus?.isConnected) previousFocus.focus({ preventScroll: true });
    };
  });
</script>

<dialog bind:this={screen} aria-label={`${title} playback`} oncancel={event => {
  event.preventDefault();
  if (player?.dismissMenu()) return;
  if (document.fullscreenElement === viewport) { void document.exitFullscreen(); return; }
  close();
}}>
  <div class="viewport" bind:this={viewport}>
  {#if fullscreenError}<p role="status">{fullscreenError}</p>{/if}
  <Player bind:this={player} {itemId} {title} {context} onClose={close} {fullscreen} {fullscreenSupported} onFullscreen={toggleFullscreen} />
  </div>
</dialog>

<style>
  dialog { position: fixed; inset: 0; width: 100%; max-width: none; height: 100dvh; max-height: none; margin: 0; padding: 0; border: 0; box-sizing: border-box; background: #000; color: var(--text, #f2f2f2); font-family: inherit; overflow: hidden; }
  dialog[open] { display: flex; flex-direction: column; }
  .viewport { position: relative; display: flex; flex-direction: column; width: 100%; height: 100%; min-height: 0; background: #000; }
  dialog::backdrop { background: #000; }
  p { position: absolute; z-index: 4; top: 80px; margin: 0 20px; padding: 12px; border-radius: 8px; background: #171717; color: var(--muted, #a0a0a0); font-size: 13px; }
</style>
