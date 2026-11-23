<script lang="ts">
  import Hls from 'hls.js';
  import { flushSync, onMount, tick } from 'svelte';
  import { client, newClient, records } from '../core/client';
  import { webConfig } from '../core/config';
  import { Playback, mediaURL, seconds } from '../core/playback';
  import { registerPlayback } from '../core/playback-lifecycle';
  import PlayerIcon from './PlayerIcon.svelte';
  import type { SourceLibraryScope, PlaybackTransport, PlaybackSession } from '../../../../../target/client-core/wasm/screenroom_client_core';
  let { itemId, title, context, onClose, fullscreen = false, fullscreenSupported = false, onFullscreen }: { itemId: string; title: string; context: SourceLibraryScope; onClose: () => void; fullscreen?: boolean; fullscreenSupported?: boolean; onFullscreen?: () => void } = $props();
  let video: HTMLVideoElement;
  let playback = $state<Playback>();
  let session = $state<PlaybackSession | null>(null);
  let status = $state('Preparing player…');
  let busy = $state(true);
  let autoplayBlocked = $state(false);
  let paused = $state(true);
  let muted = $state(false);
  let volume = $state(1);
  let audibleVolume = 1;
  let currentTime = $state(0);
  let mediaDuration = $state(0);
  let ready = $state(false);
  let blocked = $state(false);
  let detailsOpen = $state(false);
  let surface: HTMLElement;
  let settingsButton: HTMLButtonElement;
  let settingsPanel = $state<HTMLDivElement>();
  let idle = $state(false);
  let keyboardFocus = $state(false);
  let keyboardInteraction = false;
  let scrubbing = $state(false);
  let idleTimer: ReturnType<typeof setTimeout> | undefined;
  const controlsVisible = $derived(!idle || paused || busy || !ready || blocked || autoplayBlocked || detailsOpen || scrubbing || keyboardFocus);
  function reveal() {
    idle = false;
    clearTimeout(idleTimer);
    idleTimer = setTimeout(() => {
      if (surface?.querySelector('header')?.contains(document.activeElement) || surface?.querySelector('.controls')?.contains(document.activeElement)) {
        if (!paused && !busy && ready && !blocked && !autoplayBlocked && !detailsOpen && !scrubbing && !keyboardFocus) surface.focus({ preventScroll: true });
      }
      idle = true;
    }, 3000);
  }
  $effect(() => {
    if (paused || busy || !ready || blocked || autoplayBlocked || detailsOpen || scrubbing || keyboardFocus) {
      idle = false;
      clearTimeout(idleTimer);
    } else reveal();
    return () => clearTimeout(idleTimer);
  });
  export function dismissMenu() {
    if (!detailsOpen) return false;
    detailsOpen = false;
    settingsButton?.focus({ preventScroll: true });
    reveal();
    return true;
  }
  async function toggleSettings() {
    if (dismissMenu()) return;
    detailsOpen = true;
    await tick();
    settingsPanel?.focus({ preventScroll: true });
  }
  function pointerDown(event: PointerEvent) {
    keyboardInteraction = false;
    keyboardFocus = false;
    reveal();
    if (detailsOpen && !settingsPanel?.contains(event.target as Node) && !settingsButton?.contains(event.target as Node)) dismissMenu();
  }
  function keyDown(event: KeyboardEvent) {
    keyboardInteraction = true;
    keyboardFocus = isChromeFocus(document.activeElement);
    if (event.key === 'Escape' && dismissMenu()) { event.preventDefault(); event.stopPropagation(); return; }
    flushSync(reveal);
  }
  function isChromeFocus(target: EventTarget | null) {
    return target instanceof Node && !!(surface?.querySelector('header')?.contains(target) || surface?.querySelector('.controls')?.contains(target));
  }
  const offset = $derived(session?.transport ? seconds(session.transport.timeline_offset_ms) : 0);
  const duration = $derived(session?.transport?.duration_ms ? seconds(session.transport.duration_ms) : mediaDuration + offset);
  const logicalTime = $derived(Math.min(duration || Infinity, Math.max(0, currentTime + offset)));
  const disabled = $derived(!ready || busy || blocked || !session?.transport);
  const unavailable = $derived(!busy && (!session?.transport || blocked));
  const failure = $derived(status.includes('item_unavailable') ? 'This title is not available to play right now.'
    : status.includes('unknown') || status.includes('not confirmed') ? 'Playback could not be confirmed. It has not been restarted.'
    : status.includes('Profile changed') ? 'Your profile changed. Reopen this title to continue.'
    : 'Playback is unavailable right now.');
  function syncMedia() {
    currentTime = video.currentTime;
    mediaDuration = Number.isFinite(video.duration) ? video.duration : 0;
    paused = video.paused; muted = video.muted; volume = video.volume;
    if (volume > 0) audibleVolume = volume;
    ready = video.readyState >= 2 && (!!video.getAttribute('src') || !!hls);
  }
  function formatTime(value: number) {
    const total = Math.max(0, Math.floor(value));
    const hours = Math.floor(total / 3600), minutes = Math.floor(total / 60) % 60;
    return `${hours ? `${hours}:` : ''}${String(hours ? minutes : Math.floor(total / 60)).padStart(hours ? 2 : 1, '0')}:${String(total % 60).padStart(2, '0')}`;
  }
  function togglePlay() {
    if (disabled) return;
    if (!video.paused) video.pause();
    else void video.play().catch(() => { autoplayBlocked = true; });
  }
  function toggleMute() {
    if (disabled) return;
    if (video.muted || video.volume === 0) {
      if (video.volume === 0) video.volume = audibleVolume;
      video.muted = false;
    } else {
      audibleVolume = video.volume;
      video.muted = true;
    }
  }
  function setVolume(value: number) {
    if (value > 0) audibleVolume = value;
    video.volume = value;
    video.muted = false;
  }
  function seek(value: number) {
    if (disabled || !Number.isFinite(value)) return;
    const target = Math.max(offset, Math.min(duration, value)) - offset;
    if (Number.isSafeInteger(Math.round(target * 1000))) video.currentTime = target;
  }
  let hls: Hls | undefined;
  let installing = false;
  let mediaGeneration = 0;
  const { endpoint } = webConfig();
  function detach() {
    mediaGeneration++;
    installing = true;
    hls?.destroy(); hls = undefined;
    video.pause();
    video.removeAttribute('src');
    video.querySelectorAll('track').forEach(track => track.remove());
    video.load();
    ready = false;
  }
  async function attach(transport: PlaybackTransport, next: PlaybackSession, restore?: { position_ms: string; paused: boolean }) {
    const generation = ++mediaGeneration;
    installing = true;
    video.crossOrigin = 'anonymous';
    video.setAttribute('referrerpolicy', 'no-referrer');
    const url = mediaURL(transport.media_url, endpoint, next);
    const start = seconds(restore?.position_ms ?? transport.start_position_ms) - seconds(transport.timeline_offset_ms);
    if (!Number.isFinite(start) || start < 0) throw new Error('Unsupported playback timeline.');
    const ready = () => {
      if (generation !== mediaGeneration || playback?.closed) return;
      video.currentTime = start;
      installing = false;
      if (restore?.paused) { video.pause(); return; }
      void video.play().catch(() => { autoplayBlocked = true; status = 'Playback is ready. Press Play to allow media playback.'; });
    };
    video.addEventListener('loadedmetadata', ready, { once: true });
    for (const track of next.tracks.filter(t => t.selected && t.available && t.subtitle_mode === 'webvtt' && t.media_url)) {
      const element = document.createElement('track');
      element.kind = 'subtitles'; element.label = track.display_name; element.srclang = track.language || 'und'; element.default = true;
      element.onload = () => {
        if (generation !== mediaGeneration) return;
        const offset = seconds(transport.timeline_offset_ms);
        for (const cue of Array.from(element.track.cues || [])) {
          if (cue.endTime <= offset) element.track.removeCue(cue);
          else {
            cue.startTime = Math.max(0, cue.startTime - offset); cue.endTime -= offset;
            if (cue instanceof VTTCue && cue.line === 'auto') {
              cue.snapToLines = false; cue.line = 75; cue.lineAlign = 'end';
            }
          }
        }
      };
      element.src = mediaURL(track.media_url!, endpoint, next);
      video.append(element);
    }
    if (transport.mime_type === 'video/mp4' || video.canPlayType(transport.mime_type)) video.src = url;
    else if (Hls.isSupported()) {
      hls = new Hls({
        xhrSetup(xhr, resource) { mediaURL(resource, endpoint, next); xhr.withCredentials = false; },
        fetchSetup(context, init) { mediaURL(context.url, endpoint, next); return new Request(context.url, { ...init, credentials: 'omit', referrerPolicy: 'no-referrer', cache: 'no-store' }); },
      });
      hls.on(Hls.Events.ERROR, (_event, data) => { if (generation === mediaGeneration && data.fatal) void playback?.update(null, true); });
      hls.loadSource(url); hls.attachMedia(video);
    } else { status = 'This browser cannot play this HLS transport.'; playback?.exit('unsupported_transport'); }
  }
  function updateView() {
    session = playback?.session ?? null; status = playback?.status ?? status; busy = playback?.busy ?? true;
    blocked = !!(playback?.blocked || playback?.closed);
  }
  onMount(() => {
    let disposed = false;
    let owned: Awaited<ReturnType<typeof newClient>> | undefined;
    const unregister = registerPlayback(() => {
      disposed = true;
      if (playback) playback.exit('profile_changed');
      else { status = 'Profile changed. Close and reopen the player.'; busy = false; }
    });
    video.crossOrigin = 'anonymous';
    void newClient().then(c => {
      if (disposed) { c.free(); return; }
      owned = c;
      playback = new Playback(c, {
        get currentTime() { return video.currentTime; }, get paused() { return video.paused; },
        get positionReady() { return !installing; },
        get muted() { return video.muted; }, get volume() { return video.volume; }, detach, attach,
      }, updateView, () => {
        records.clear();
        void client().then(c => c.invalidate_reads());
      });
      const mp4 = !!video.canPlayType('video/mp4; codecs="avc1.42E01E, mp4a.40.2"');
      void playback.create(itemId, context, { mp4, h264: mp4, aac: mp4, native_hls: !!video.canPlayType('application/vnd.apple.mpegurl'), mse_hls: Hls.isSupported(), webvtt: 'textTracks' in video });
    }).catch(() => {
      if (!disposed) { status = 'Playback client unavailable. Close and reopen the player.'; busy = false; }
    });
    return () => {
      disposed = true; unregister(); playback?.exit(); detach();
      if (playback) void playback.settled().finally(() => owned?.free());
      else owned?.free();
    };
  });
  function track(kind: 'audio' | 'subtitle', id: string) {
    void playback?.update({ audio_track_id: kind === 'audio' ? id : null, subtitle_track_id: kind === 'subtitle' && id ? id : null, subtitles_off: kind === 'subtitle' && !id });
  }
</script>

<svelte:window onkeydown={keyDown} onpointerup={() => { scrubbing = false; }} onpointercancel={() => { scrubbing = false; }} />
<section bind:this={surface} aria-label={`${title} player`} tabindex="-1"
  onpointermove={reveal} onpointerdown={pointerDown}
  onfocusin={event => { keyboardFocus = keyboardInteraction && isChromeFocus(event.target); }}
  onfocusout={event => { if (!isChromeFocus(event.relatedTarget)) keyboardFocus = false; }}>
  <header class:hidden={!controlsVisible} inert={!controlsVisible}>
    <button class="icon" data-player-back onclick={() => { dismissMenu(); onClose(); }} aria-label="Back to details" title="Back to details"><PlayerIcon name="back" /></button>
    <h1 title={title}>{title}</h1>
    <button class="icon" onclick={() => { dismissMenu(); onClose(); }} aria-label="Close player" title="Close player"><PlayerIcon name="close" /></button>
  </header>
  <div class="stage">
  <video bind:this={video} crossorigin="anonymous" preload="metadata" playsinline aria-label={`${title} video`}
    onloadeddata={syncMedia} ondurationchange={syncMedia} onvolumechange={syncMedia}
    onplay={syncMedia} onemptied={syncMedia}
    onplaying={() => { autoplayBlocked = false; if (!installing) void playback?.event(playback.started ? 'resumed' : 'started'); }}
    onpause={() => { syncMedia(); if (!installing) void playback?.event('paused'); }}
    onseeked={() => { syncMedia(); if (!installing) void playback?.event('seeked'); }}
    ontimeupdate={() => { syncMedia(); if (!installing && session?.transport) playback?.progress(); }}
    onended={() => playback?.exit('ended')}
    onerror={() => { if (video.getAttribute('src')) void playback?.update(null, true); }}>
    Your browser does not support video playback.
  </video>
  {#if !ready || unavailable}
    <div class="state" role="status">
      <div class="state-content">
        <span class="eyebrow">{unavailable ? 'Unable to play' : 'Screenroom'}</span>
        <h2>{unavailable ? session ? 'Playback stopped' : 'Unable to start playback' : 'Preparing playback'}</h2>
        <p>{unavailable ? failure : 'Your video will appear here when it’s ready.'}</p>
        <div class="state-actions">
          {#if !session && !busy}<button onclick={() => playback?.retryCreate()}>Retry playback request</button>{/if}
          {#if unavailable}<button onclick={onClose}>Return to details</button>{/if}
        </div>
      </div>
    </div>
  {/if}
  </div>
  <div class="controls" class:hidden={!controlsVisible} inert={!controlsVisible} aria-label="Playback controls">
    {#if autoplayBlocked}<p class="hint" role="status">Ready when you are. Press Play to begin.</p>{/if}
    <div class="timeline">
      <input aria-label="Playback position" type="range" min={offset} max={duration || offset + 1} step="0.1" value={logicalTime} disabled={disabled || !duration} aria-valuetext={`${formatTime(logicalTime)} of ${formatTime(duration)}`} onpointerdown={() => { scrubbing = true; }} oninput={event => seek(Number(event.currentTarget.value))} />
      <span class="time" aria-label="Playback time">{formatTime(logicalTime)} <span>/ {formatTime(duration)}</span></span>
    </div>
    <div class="toolbar">
      <button class="icon" disabled={disabled} onclick={togglePlay} aria-label={paused ? 'Play' : 'Pause'} title={paused ? 'Play' : 'Pause'}><PlayerIcon name={paused ? 'play' : 'pause'} /></button>
      <button class="icon" disabled={disabled} onclick={() => seek(logicalTime - 30)} aria-label="Back 30 seconds" title="Back 30 seconds"><PlayerIcon name="rewind" /></button>
      <button class="icon" disabled={disabled} onclick={() => seek(logicalTime + 30)} aria-label="Forward 30 seconds" title="Forward 30 seconds"><PlayerIcon name="forward" /></button>
      <button class="icon" disabled={disabled} onclick={toggleMute} aria-label={muted || volume === 0 ? 'Unmute' : 'Mute'} title={muted || volume === 0 ? 'Unmute' : 'Mute'}><PlayerIcon name={muted || volume === 0 ? 'muted' : 'volume'} /></button>
      <input class="volume" aria-label="Volume" type="range" min="0" max="1" step="0.05" value={muted ? 0 : volume} disabled={disabled} oninput={event => setVolume(Number(event.currentTarget.value))} />
      <div class="spacer"></div>
      <button bind:this={settingsButton} class="icon" aria-label="Playback settings" title="Playback settings" aria-expanded={detailsOpen} aria-controls="player-settings" onclick={toggleSettings}><PlayerIcon name="settings" /></button>
      <button class="icon" disabled={!fullscreenSupported} aria-label={!fullscreenSupported ? 'Fullscreen unavailable' : fullscreen ? 'Exit fullscreen' : 'Fullscreen'} title={fullscreenSupported ? fullscreen ? 'Exit fullscreen' : 'Fullscreen' : 'Fullscreen unavailable'} onclick={() => { dismissMenu(); onFullscreen?.(); }}><PlayerIcon name={fullscreen ? 'shrink' : 'fullscreen'} /></button>
    </div>
  {#if detailsOpen}
  <div bind:this={settingsPanel} class="settings" id="player-settings" role="region" tabindex="-1" aria-label="Playback settings">
  <h2>Playback settings</h2>
  <label class="compact-volume">Volume
    <input aria-label="Volume" type="range" min="0" max="1" step="0.05" value={muted ? 0 : volume} disabled={disabled} oninput={event => setVolume(Number(event.currentTarget.value))} />
  </label>
  {#if session?.tracks.some(t => t.kind === 'audio')}
    <label>Audio<select aria-label="Audio" disabled={busy || blocked} value={session.tracks.find(t => t.kind === 'audio' && t.selected)?.id ?? ''} onchange={e => track('audio', e.currentTarget.value)}>
      {#each session.tracks.filter(t => t.kind === 'audio') as choice}<option value={choice.id} disabled={!choice.available}>{choice.display_name}{!choice.available ? ' (unavailable)' : ''}</option>{/each}
    </select></label>
  {/if}
  {#if session?.tracks.some(t => t.kind === 'subtitle')}
    <label>Subtitles<select aria-label="Subtitles" disabled={busy || blocked} value={session.tracks.find(t => t.kind === 'subtitle' && t.selected)?.id ?? ''} onchange={e => track('subtitle', e.currentTarget.value)}>
      <option value="">Off</option>
      {#each session.tracks.filter(t => t.kind === 'subtitle') as choice}<option value={choice.id} disabled={!choice.available || choice.subtitle_mode === 'unavailable'}>{choice.display_name}{choice.subtitle_mode === 'burn_in' ? ' (burn-in)' : !choice.available || choice.subtitle_mode === 'unavailable' ? ' (unsupported)' : ''}</option>{/each}
    </select></label>
  {/if}
  <details><summary>Session information</summary>
    <p role="status">{status}</p>
    {#if session && !busy && !playback?.closed}<button onclick={() => playback?.check()}>Check session state</button>{/if}
    {#if session && !busy && playback?.canRetryTransport}<button onclick={() => playback?.retryUpdate()}>Retry transport request</button>{/if}
  </details>
  </div>
  {/if}
  </div>
</section>

<style>
  section { position: relative; flex: 1; min-height: 0; min-width: 0; background: #000; color: #fff; overflow: hidden; }
  header { position: absolute; z-index: 3; inset: 0 0 auto; display: flex; align-items: center; gap: 12px; padding: max(16px, env(safe-area-inset-top)) 20px 32px; background: linear-gradient(#000b, transparent); }
  h1 { flex: 1; min-width: 0; margin: 0; font-size: 15px; font-weight: 500; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  header, .controls { transition: opacity .2s ease, visibility .2s; }
  .hidden { opacity: 0; visibility: hidden; pointer-events: none; }
  .stage, video { position: absolute; inset: 0; width: 100%; height: 100%; }
  video { display: block; object-fit: contain; background: #000; }
  .state { position: absolute; inset: 80px 20px 140px; display: grid; place-items: center; text-align: center; overflow: auto; }
  .state-content { max-width: 420px; margin: auto; }
  .eyebrow { color: var(--muted, #a0a0a0); font-size: 12px; }
  h2 { margin: 12px 0; font-size: 20px; font-weight: 500; }
  p { color: var(--muted, #a0a0a0); font-size: 14px; line-height: 1.6; }
  .state-actions { display: flex; justify-content: center; flex-wrap: wrap; gap: 8px; margin-top: 24px; }
  .controls { position: absolute; z-index: 2; bottom: 0; left: 0; right: 0; padding: 40px 24px max(16px, env(safe-area-inset-bottom)); background: linear-gradient(transparent, #000c); }
  .timeline { display: flex; gap: 16px; align-items: center; }
  .timeline input { flex: 1; min-width: 0; }
  input[type=range] { accent-color: var(--text, #f2f2f2); height: 44px; margin: 0; cursor: pointer; }
  .time { font-size: 12px; font-variant-numeric: tabular-nums; white-space: nowrap; }
  .time span { color: var(--muted, #a0a0a0); }
  .toolbar { display: flex; align-items: center; gap: 8px; }
  .spacer { flex: 1; }
  .volume { width: 90px; }
  .compact-volume { display: none; }
  button, select { min-height: 44px; color: inherit; font: inherit; background: var(--surface, #171717); border: 1px solid var(--border, #ffffff1c); border-radius: 8px; padding: 8px 12px; }
  button, summary { cursor: pointer; }
  button:hover { background: #ffffff0c; }
  button:disabled, input:disabled { opacity: .4; cursor: default; }
  .icon { display: grid; place-items: center; width: 44px; height: 44px; flex: none; padding: 0; background: transparent; border-color: transparent; }
  button:focus-visible, input:focus-visible, select:focus-visible, summary:focus-visible { outline: 2px solid var(--focus, #a8c9ff); outline-offset: 2px; }
  .settings { position: absolute; right: 24px; bottom: calc(100% - 24px); width: 340px; max-width: calc(100% - 48px); max-height: max(44px, calc(100dvh - 220px)); overflow: auto; overscroll-behavior: contain; box-sizing: border-box; padding: 20px; background: #181818f5; border: 1px solid #ffffff24; border-radius: 12px; box-shadow: 0 8px 32px #0008; }
  .settings h2 { margin: 0 0 20px; font-size: 15px; }
  label { display: grid; gap: 8px; margin-bottom: 16px; font-size: 13px; }
  select { width: 100%; min-width: 0; text-overflow: ellipsis; }
  .settings p { overflow-wrap: anywhere; }
  summary { display: flex; align-items: center; min-height: 44px; color: var(--muted, #a0a0a0); font-size: 13px; }
  details button { width: 100%; margin-top: 8px; font-size: 13px; }
  .hint { margin: 0; font-size: 13px; }
  @media (max-width: 600px) { header { padding-inline: 12px; gap: 8px; } .controls { padding-inline: 12px; } .timeline { gap: 8px; } .toolbar { gap: 0; } .volume { width: 64px; } .settings { right: 12px; max-width: calc(100% - 24px); } h2 { font-size: 18px; } }
  @media (max-width: 360px) { .volume { display: none; } .compact-volume { display: grid; } .time { font-size: 11px; } }
  @media (max-height: 420px) { header { padding-top: 4px; padding-bottom: 8px; } .controls { padding-top: 8px; padding-bottom: 4px; } .settings { bottom: 100%; max-height: calc(100dvh - 148px); padding: 12px; } .state { inset: 56px 16px 104px; } .state-content h2 { margin: 4px; font-size: 16px; } .state-content p { margin: 4px; } .state-actions { margin-top: 8px; } }
  @media (prefers-reduced-motion: reduce) { header, .controls { transition: none; } }
</style>
