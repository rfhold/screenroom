import { expect, test } from '@playwright/test';
import { mkdir, rm } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { allowedPlaybackRequest, measuredViewport, playbackSelection, saveViewport } from './playback-contract.mjs';
import { assertChrome, observe, reveal, settled, type Observation } from './playback-observation';
import { trackReadiness } from './playback-tracks.mjs';
import { muteSettled, volumeSettled } from './playback-controls.mjs';

test('real selected title playback showcase @happy-path', async ({ page }, info) => {
  test.setTimeout(300_000);
  const selection = playbackSelection();
  const profile = info.project.name;
  expect(['desktop', 'mobile']).toContain(profile);
  const directory = fileURLToPath(new URL('./build/screenshots/playback/', import.meta.url));
  await mkdir(directory, { recursive: true });
  const states: Parameters<typeof saveViewport>[3] = [];
  const unsafe: string[] = [];
  const errors: string[] = [];
  const methods = new Set<string>();
  let mediaResponses = 0;
  let stopResponse = false;
  let updateRequests = 0;
  let blocked = false;
  let clicked = false;
  let closed = false;
  page.on('pageerror', () => errors.push('Uncaught application error'));
  page.on('request', request => {
    if (!allowedPlaybackRequest(request.url(), String(info.project.use.baseURL), selection.endpoint)) unsafe.push('Request outside approved public endpoint allowlist');
    const path = new URL(request.url()).pathname;
    if (path.startsWith('/screenroom.playback_gateway.v1.PlaybackService/')) methods.add(path.split('/').at(-1)!);
    if (path === '/screenroom.playback_gateway.v1.PlaybackService/UpdatePlaybackSession') updateRequests++;
  });
  page.on('response', response => {
    if (new URL(response.url()).origin === selection.endpoint && new URL(response.url()).pathname.startsWith('/playback/v1/media/') && response.ok()) mediaResponses++;
    if (new URL(response.url()).pathname === '/screenroom.playback_gateway.v1.PlaybackService/StopPlaybackSession') stopResponse = true;
  });
  await page.addInitScript(namespace => localStorage.setItem('screenroom.settingsNamespace', namespace), `playback-${crypto.randomUUID()}`);
  const dialog = page.locator('dialog[open]');
  const button = (name: string) => dialog.getByRole('button', { name, exact: true });
  const detailPath = `/titles/${encodeURIComponent(selection.item.id!)}`;
  const normalIDs = ['playing', 'idle', 'paused', 'seek-back', 'seek-forward', 'muted', 'unmuted', 'volume-min', 'volume-mid', 'volume-max', 'settings', 'session-information', 'audio-tracks', 'audio-popup', 'subtitle-tracks', 'subtitle-popup', 'subtitles-off', 'fullscreen', 'fullscreen-exit'];
  async function row(id: string, label: string, status: string, reason: string) {
    const state = await observe(page);
    states.push({ id: `${profile}-${id}`, label: `${profile}: ${label}`, status, viewport: measuredViewport(state.viewport), reason, observation: reason });
  }
  async function capture(id: string, label: string, check: (state: Observation) => void | Promise<void>, extra = '', transientPreparing = false) {
    await page.evaluate(() => document.fonts.ready);
    const before = await observe(page);
    if (transientPreparing && !before.preparing) return row(id, label, 'not-observed', 'Natural preparation passed before screenshot; not forced.');
    await check(before);
    expect(unsafe).toEqual([]);
    const image = `${profile}-${id}.png`;
    await page.screenshot({ path: `${directory}/${image}` });
    const after = await observe(page);
    if (transientPreparing && !after.preparing) {
      await rm(`${directory}/${image}`);
      return row(id, label, 'not-observed', 'Natural preparation did not persist across the screenshot; no preparation image claimed.');
    }
    await check(after);
    expect(after.viewport).toEqual(before.viewport);
    states.push({ id: `${profile}-${id}`, label: `${profile}: ${label}`, status: 'captured', viewport: measuredViewport(before.viewport), image, observation: `${extra}${JSON.stringify({ before, after })}` });
  }
  function ready(state: Observation) {
    expect(state.videoCount).toBe(1);
    expect(state.media!.readyState).toBeGreaterThanOrEqual(2);
    expect(state.media!.width).toBeGreaterThan(0);
    expect(state.error).toBe(false);
  }
  async function click(name: string) { await reveal(page); await button(name).click(); }
  async function pause(value: boolean) {
    await reveal(page);
    if ((await observe(page)).media!.paused !== value) await button(value ? 'Pause' : 'Play').click();
    await expect.poll(async () => (await observe(page)).media?.paused).toBe(value);
    await expect.poll(async () => value ? (await observe(page)).playIcon : (await observe(page)).pauseIcon).toBe(value ? 'm8 5 11 7-11 7Z' : 'M8 5v14M16 5v14');
    await settled(page);
  }
  async function settings(open: boolean) {
    await reveal(page);
    if ((await button('Playback settings').getAttribute('aria-expanded') === 'true') !== open) await button('Playback settings').click();
    await expect(button('Playback settings')).toHaveAttribute('aria-expanded', String(open));
  }
  async function close() {
    if (!clicked || closed) return;
    await reveal(page);
    await button('Close player').click();
    closed = true;
    await expect(page).toHaveURL(url => url.pathname === detailPath);
    await expect(page.locator('video')).toHaveCount(0);
    await expect(page.getByRole('heading', { name: selection.item.title!, exact: true })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Play', exact: true })).toBeVisible();
    await expect(page.getByText('Loading Screenroom…', { exact: true })).toHaveCount(0);
    if (methods.has('ReportPlaybackEvent')) await expect.poll(() => stopResponse, { timeout: 15_000 }).toBe(true);
    await capture('closed', 'Closed player; completed title details', state => {
      expect(state.videoCount).toBe(0);
      expect(state.media).toBeNull();
      expect(state.detailTitle).toBe(selection.item.title);
      expect(state.detailPlayVisible).toBe(true);
      expect(state.loadingVisible).toBe(false);
    }, 'Normal Close; visible selected title and Play, completed details route, zero DOM videos. Provider stop confirmation is not inferred from DOM teardown. ');
  }
  try {
    await page.goto(detailPath);
    const article = page.getByRole('article', { name: `${selection.item.title} details`, exact: true });
    await expect(article).toBeVisible();
    await expect(page.getByRole('heading', { name: selection.item.title!, exact: true })).toBeVisible();
    await expect(article).toContainText(String(selection.item.year));
    await page.getByRole('button', { name: 'Play', exact: true }).click();
    clicked = true;
    await expect(dialog).toBeVisible();
    if ((await observe(page)).preparing) {
      await capture('preparing', 'Natural preparing state', state => expect(state.preparing).toBe(true), '', true);
    } else await row('preparing', 'Preparing state', 'not-observed', 'Natural preparation passed before capture; not forced.');
    await expect.poll(async () => {
      const state = await observe(page);
      return state.error ? 'error' : state.media && state.media.readyState >= 2 && state.media.width > 0 ? 'ready' : 'waiting';
    }, { timeout: 90_000 }).not.toBe('waiting');
    if ((await observe(page)).error) {
      blocked = true;
      await capture('error', 'Natural playback failure', state => expect(state.error).toBe(true), 'Natural application error; no injected failures, retries or fabricated playback success. ');
      for (const id of normalIDs) await row(id, id, 'blocked', 'Natural playback failure prevented this state.');
      await row('seek-active', 'Seek in progress', 'blocked', 'Playback failed before a seek.');
    } else {
      await pause(false);
      const playingTime = (await observe(page)).media!.currentTime;
      await expect.poll(async () => (await observe(page)).media!.currentTime, { timeout: 10_000 }).toBeGreaterThan(playingTime);
      await reveal(page);
      await capture('playing', 'Playing with settled Pause control', state => { ready(state); assertChrome(state, false); expect(state.media!.paused).toBe(false); expect(state.pauseIcon).toBe('M8 5v14M16 5v14'); });
      const size = page.viewportSize()!;
      await page.mouse.click(size.width / 2, size.height / 2);
      await page.mouse.move(1, size.height / 2);
      await settled(page, true);
      await capture('idle', 'Playing with fully hidden idle chrome', state => { ready(state); assertChrome(state, true); expect(state.media!.paused).toBe(false); });
      await pause(true);
      await capture('paused', 'Paused with Play control', state => { ready(state); assertChrome(state, false); expect(state.media!.paused).toBe(true); expect(state.playIcon).toBe('m8 5 11 7-11 7Z'); });
      for (const [id, name, delta] of [['seek-forward', 'Forward 30 seconds', 30], ['seek-back', 'Back 30 seconds', -30]] as const) {
        const before = await observe(page);
        const target = Math.max(before.minimum!, Math.min(before.maximum!, before.logicalTime! + delta));
        await click(name);
        await expect.poll(async () => { const state = await observe(page); return !state.media!.seeking && Math.abs(state.logicalTime! - target) < 1; }, { timeout: 30_000 }).toBe(true);
        await capture(id, name, state => { ready(state); expect(state.media!.seeking).toBe(false); expect(Math.abs(state.logicalTime! - target)).toBeLessThan(1); }, `Logical timeline before=${before.logicalTime}; expected after=${target}. `);
      }
      await row('seek-active', 'Seek in progress', 'not-observed', 'Seek completion was verified; transient seeking was not captured.');
      for (const [id, name, muted] of [['muted', 'Mute', true], ['unmuted', 'Unmute', false]] as const) {
        await click(name);
        await expect.poll(async () => muteSettled(await observe(page), muted), { timeout: 10_000 }).toBe(true);
        await capture(id, name, state => { ready(state); expect(state.media!.muted).toBe(muted); expect(state.muteControl).toBe(muted ? 'Unmute' : 'Mute'); });
      }
      const volume = page.getByRole('slider', { name: 'Volume', exact: true }).filter({ visible: true }).first();
      for (const [id, value] of [['volume-min', 0], ['volume-mid', 0.5], ['volume-max', 1]] as const) {
        await volume.press('Home');
        for (let step = 0; step < value * 20; step++) await volume.press('ArrowRight');
        await expect.poll(async () => volumeSettled(await observe(page), value), { timeout: 10_000 }).toBe(true);
        await capture(id, `Volume ${value}`, state => { ready(state); expect(state.media!.volume).toBe(value); expect(state.media!.muted).toBe(false); expect(state.volumeValues).toContain(value); });
      }
      await settings(true);
      await capture('settings', 'Playback settings', state => { ready(state); expect(state.settings).toBe(true); });
      await page.getByText('Session information', { exact: true }).click();
      await expect(page.locator('#player-settings details')).toHaveAttribute('open', '');
      await capture('session-information', 'Expanded session information', state => { ready(state); expect(state.settings).toBe(true); expect(state.sessionInformationExpanded).toBe(true); }, 'Normal Session information disclosure is expanded. ');
      for (const [kind, label] of [['audio', 'Audio'], ['subtitle', 'Subtitles']] as const) {
        const select = page.getByRole('combobox', { name: label, exact: true });
        if (await select.count()) {
          const options = await select.locator('option').evaluateAll(nodes => nodes.map((node, index) => ({ index, value: (node as HTMLOptionElement).value, label: node.textContent?.trim() || 'Unnamed track', selected: (node as HTMLOptionElement).selected, available: !(node as HTMLOptionElement).disabled, off: (node as HTMLOptionElement).value === '' })));
          const publicOptions = options.map(({ index, label, selected, available, off }) => ({ index, label, selected, available, off }));
          if (blocked) await row(`${kind}-tracks`, `${label} track choices`, 'blocked', 'Earlier track change produced a natural blocked player.');
          else await capture(`${kind}-tracks`, `${label} track choices`, state => { ready(state); expect(state.settings).toBe(true); }, `Actual public UI options: ${JSON.stringify(publicOptions)}. Native select popup pixels are not asserted. `);
          await row(`${kind}-popup`, `${label} native popup`, 'unavailable', 'Browser page screenshots do not reliably capture OS-native select popups; actual DOM choices are recorded with settings evidence.');
          for (const option of options) {
            const id = kind === 'subtitle' && option.off ? 'subtitles-off' : `${kind}-option-${option.index + 1}`;
            const optionLabel = `${label}: ${option.label}`;
            if (!option.available) {
              await row(id, optionLabel, 'unavailable', 'Actual option is disabled in the normal track selector.');
              continue;
            }
            if (blocked) {
              await row(id, optionLabel, 'blocked', 'Earlier track change blocked playback; no further transport requests were attempted.');
              continue;
            }
            const requestedSelected = () => select.evaluate((node, value) => (node as HTMLSelectElement).value === value, option.value);
            const currentSelection = await requestedSelected();
            const requestCount = updateRequests;
            // The private URL stays inside a browser handle; only identity booleans leave it.
            const transport = await page.evaluateHandle(() => {
              const video = document.querySelector('video');
              return { video, source: video?.currentSrc || video?.getAttribute('src') || '' };
            });
            let updateResponded = false;
            try {
              if (!currentSelection) {
                const [response] = await Promise.all([
                  page.waitForResponse(response => new URL(response.url()).origin === selection.endpoint && new URL(response.url()).pathname === '/screenroom.playback_gateway.v1.PlaybackService/UpdatePlaybackSession', { timeout: 30_000 }),
                  select.selectOption({ value: option.value }),
                ]);
                updateResponded = response.ok();
              }
              const identity = () => transport.evaluate(saved => {
                const video = document.querySelector('video');
                const source = video?.currentSrc || video?.getAttribute('src') || '';
                return { sameVideo: !!video && video === saved.video, sourceAttached: !!source, transportChanged: !!source && source !== saved.source };
              });
              const readiness = async () => {
                const state = await observe(page);
                const media = await identity();
                return trackReadiness({
                  error: state.error, uncertain: state.uncertain,
                  mediaReady: !!state.media && state.media.readyState >= 2 && state.media.width > 0 && !state.media.seeking && media.sameVideo && media.sourceAttached,
                  controlsEnabled: state.controlsEnabled,
                  selectorEnabled: kind === 'audio' ? state.audioEnabled : state.subtitleEnabled,
                  requestedSelected: await requestedSelected(),
                  transportChanged: media.transportChanged, updateResponded,
                }, currentSelection);
              };
              await expect.poll(readiness, { timeout: 90_000 }).not.toBe('waiting');
              if (await readiness() === 'blocked') {
                blocked = true;
                await capture(`${id}-error`, `${optionLabel}: natural blocked playback`, state => expect(state.error || state.uncertain).toBe(true), 'Normal track selection produced a visible blocked or uncertain player; no automatic retry requested. ');
                await row(id, optionLabel, 'blocked', 'Requested configuration did not reach confirmed attached-media readiness.');
                continue;
              }
              await reveal(page);
              await capture(id, optionLabel, async state => {
                ready(state);
                assertChrome(state, false);
                expect(state.settings).toBe(true);
                expect(state.controlsEnabled).toBe(true);
                expect(state.uncertain).toBe(false);
                expect(kind === 'audio' ? state.audioEnabled : state.subtitleEnabled).toBe(true);
                expect(await readiness()).toBe('ready');
              }, `Track configuration evidence: ${JSON.stringify({ kind, optionIndex: option.index, operation: currentSelection ? 'capture-current-selection' : 'normal-ui-selection', requestedSelectionMatched: await requestedSelected(), updateResponded, ...(await identity()) })}. `);
              if (currentSelection) expect(updateRequests, 'Already selected option must not issue a redundant transport update').toBe(requestCount);
            } finally { await transport.dispose(); }
          }
          if (kind === 'subtitle' && !options.some(option => option.off)) await row('subtitles-off', 'Subtitles Off', 'unavailable', 'Actual subtitle selector has no Off choice.');
        } else {
          await row(`${kind}-tracks`, `${label} tracks`, 'unavailable', `No ${label} selector is exposed for this session.`);
          await row(`${kind}-popup`, `${label} native popup`, 'unavailable', `No ${label} selector is exposed.`);
          if (kind === 'subtitle') await row('subtitles-off', 'Subtitles Off', 'unavailable', 'No subtitle selector or Off choice is exposed for this session.');
        }
      }
      if (!blocked) await settings(false);
      const fullscreen = button('Fullscreen');
      if (blocked) {
        await row('fullscreen', 'Fullscreen', 'blocked', 'Natural track change failure prevented further normal playback states.');
        await row('fullscreen-exit', 'Exit fullscreen', 'blocked', 'Natural track change failure prevented fullscreen entry.');
      } else if (await fullscreen.count() && await fullscreen.isEnabled()) {
        await click('Fullscreen');
        await expect.poll(async () => (await observe(page)).fullscreen || await page.getByText('Fullscreen is unavailable. You can keep watching here.', { exact: true }).isVisible(), { timeout: 5_000 }).toBe(true);
        if ((await observe(page)).fullscreen) {
          await reveal(page);
          await capture('fullscreen', 'Fullscreen player', state => { ready(state); expect(state.fullscreenPlayer).toBe(true); });
          await click('Exit fullscreen');
          await expect.poll(async () => (await observe(page)).fullscreen).toBe(false);
          await capture('fullscreen-exit', 'Exited fullscreen', state => { ready(state); expect(state.fullscreen).toBe(false); });
        } else {
          await row('fullscreen', 'Fullscreen', 'unavailable', 'Normal Fullscreen action produced the visible application message that fullscreen is unavailable.');
          await row('fullscreen-exit', 'Exit fullscreen', 'unavailable', 'Fullscreen was requested normally but unavailable.');
        }
      } else {
        await row('fullscreen', 'Fullscreen', 'unavailable', 'Actual browser runtime does not expose an enabled Fullscreen action.');
        await row('fullscreen-exit', 'Exit fullscreen', 'unavailable', 'Fullscreen was unavailable.');
      }
      if (!blocked) await row('error', 'Natural playback error', 'not-observed', 'No natural playback error was observed; no faults injected.');
      expect(mediaResponses, 'Successful public media gateway response').toBeGreaterThan(0);
      expect(methods.has('CreatePlaybackSession')).toBe(true);
      expect(methods.has('ReportPlaybackEvent')).toBe(true);
    }
    await close();
    expect(unsafe).toEqual([]);
    expect(errors).toEqual([]);
    await row('stop', 'Provider stop confirmation', 'not-observed', 'Normal UI Close and local media teardown observed; provider-side stop confirmation is not exposed in completed title UI.');
    await row('stop-control', 'Dedicated Stop control', 'unavailable', 'This player exposes Close player and Back to details to end playback, not a dedicated Stop button.');
    await saveViewport(directory, selection, profile, states, blocked);
  } finally {
    // Close only this test's owned player, before Playwright tears down its context.
    if (clicked && !closed) await close();
  }
});
