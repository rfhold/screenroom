import { expect, type Page } from '@playwright/test';

export async function observe(page: Page) {
  return page.evaluate(() => {
    const video = document.querySelector('video');
    const controls = document.querySelector('dialog[open] .controls');
    const appearance = (selector: string) => {
      const node = document.querySelector(selector);
      if (!node) return null;
      const style = getComputedStyle(node);
      return { opacity: Number(style.opacity), visibility: style.visibility, display: style.display, animations: node.getAnimations().filter(a => a.pending || a.playState === 'running').length };
    };
    const icon = (name: string) => controls?.querySelector(`button[aria-label="${name}"] svg path`)?.getAttribute('d') ?? null;
    const position = document.querySelector<HTMLInputElement>('[aria-label="Playback position"]');
    return {
      viewport: { width: innerWidth, height: innerHeight, devicePixelRatio },
      videoCount: document.querySelectorAll('video').length,
      media: video ? { paused: video.paused, muted: video.muted, volume: video.volume, currentTime: video.currentTime, readyState: video.readyState, seeking: video.seeking, width: video.videoWidth, height: video.videoHeight } : null,
      logicalTime: position ? Number(position.value) : null,
      minimum: position ? Number(position.min) : null,
      maximum: position ? Number(position.max) : null,
      fullscreen: !!document.fullscreenElement,
      fullscreenPlayer: document.fullscreenElement?.matches('dialog .viewport') ?? false,
      controls: appearance('dialog[open] .controls'),
      header: appearance('dialog[open] header'),
      focusedChrome: !!(controls?.contains(document.activeElement) || document.querySelector('dialog[open] header')?.contains(document.activeElement)),
      settings: document.querySelector('[aria-label="Playback settings"][aria-expanded]')?.getAttribute('aria-expanded') === 'true',
      sessionInformationExpanded: !!document.querySelector<HTMLDetailsElement>('#player-settings details')?.open,
      controlsEnabled: !!controls?.querySelector<HTMLButtonElement>('button[aria-label="Play"]:not(:disabled), button[aria-label="Pause"]:not(:disabled)') && !!position && !position.disabled,
      audioSelectedIndex: document.querySelector<HTMLSelectElement>('select[aria-label="Audio"]')?.selectedIndex ?? null,
      subtitleSelectedIndex: document.querySelector<HTMLSelectElement>('select[aria-label="Subtitles"]')?.selectedIndex ?? null,
      audioEnabled: !!document.querySelector<HTMLSelectElement>('select[aria-label="Audio"]:not(:disabled)'),
      subtitleEnabled: !!document.querySelector<HTMLSelectElement>('select[aria-label="Subtitles"]:not(:disabled)'),
      uncertain: [...document.querySelectorAll('#player-settings details [role="status"]')].some(node => /\b(?:unknown|uncertain|pending)\b|not confirmed/i.test(node.textContent || '')),
      muteControl: controls?.querySelector('button[aria-label="Mute"], button[aria-label="Unmute"]')?.getAttribute('aria-label') ?? null,
      volumeValues: [...document.querySelectorAll<HTMLInputElement>('input[aria-label="Volume"]')].filter(node => node.checkVisibility()).map(node => Number(node.value)),
      pauseIcon: icon('Pause'), playIcon: icon('Play'),
      detailTitle: [...document.querySelectorAll('article h1')].find(node => node.checkVisibility())?.textContent?.trim() ?? null,
      detailPlayVisible: [...document.querySelectorAll('article button')].some(node => node.textContent?.trim() === 'Play' && node.checkVisibility()),
      loadingVisible: [...document.querySelectorAll('[role="status"]')].some(node => node.textContent?.includes('Loading Screenroom') && node.checkVisibility()),
      error: [...document.querySelectorAll('dialog[open] h2')].some(node => /^(Playback stopped|Unable to start playback)$/.test(node.textContent || '') && node.checkVisibility()),
      preparing: [...document.querySelectorAll('dialog[open] h2')].some(node => node.textContent === 'Preparing playback' && node.checkVisibility()),
    };
  });
}
export type Observation = Awaited<ReturnType<typeof observe>>;

export function assertChrome(state: Observation, hidden: boolean) {
  for (const appearance of [state.header, state.controls]) {
    expect(appearance).not.toBeNull();
    expect(appearance!.opacity).toBe(hidden ? 0 : 1);
    expect(appearance!.visibility).toBe(hidden ? 'hidden' : 'visible');
    expect(appearance!.display).not.toBe('none');
    expect(appearance!.animations).toBe(0);
  }
  if (hidden) {
    expect(state.focusedChrome).toBe(false);
    expect(state.settings).toBe(false);
  }
}
export async function settled(page: Page, hidden = false) {
  await expect.poll(async () => {
    const state = await observe(page);
    return [state.header, state.controls].every(a => a && a.opacity === (hidden ? 0 : 1) && a.visibility === (hidden ? 'hidden' : 'visible') && a.animations === 0);
  }, { timeout: 15_000 }).toBe(true);
}
export async function reveal(page: Page) {
  const size = page.viewportSize()!;
  await page.mouse.move(size.width / 2, size.height / 2);
  // Always generate a pointer move, even if the previous action ended at center.
  await page.mouse.move(size.width / 2 + 1, size.height / 2 + 1);
  await settled(page);
}
