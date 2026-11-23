import { readFile, writeFile, rename, mkdir } from 'node:fs/promises';
import { readFileSync } from 'node:fs';

const uuid = /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/i;
/** @typedef {{instanceID: string, item: {id: string | undefined, title: string | undefined, year: number}}} Selection */
/** @typedef {{id: string, label: string, status: string, viewport: string, image?: string, reason?: string, observation: string}} State */
/** @typedef {{schema_version: number, journey: string, backend_instance_id: string, item: Selection['item'], platforms: {slug: string, runtime: string, outcome: string, states: State[]}[]}} Manifest */
/** @param {{width: number, height: number}} viewport */
export function measuredViewport(viewport) {
  if (!Number.isInteger(viewport.width) || viewport.width <= 0 || !Number.isInteger(viewport.height) || viewport.height <= 0) throw new Error('Invalid measured playback viewport.');
  return `${viewport.width}x${viewport.height} CSS pixels`;
}
export function playbackSelection(env = process.env) {
  if (env.SCREENROOM_BACKEND_ATTACH !== '1' || env.SCREENROOM_BACKEND_PROJECT !== 'screenroom-jellyfin-local') throw new Error('Playback requires explicit attachment to screenroom-jellyfin-local.');
  const endpoint = env.SCREENROOM_E2E_API_ENDPOINT || env.SCREENROOM_API_ENDPOINT;
  if (endpoint !== 'http://127.0.0.1:50052' || (env.SCREENROOM_API_ENDPOINT && env.SCREENROOM_API_ENDPOINT !== endpoint)) throw new Error('Playback requires the approved public API origin on 50052.');
  const item = { id: env.SCREENROOM_SHOWCASE_ITEM_ID, title: env.SCREENROOM_SHOWCASE_TITLE, year: Number(env.SCREENROOM_SHOWCASE_YEAR) };
  if (!item.id || !item.title || !Number.isInteger(item.year) || item.year < 1800 || item.year > 2200) throw new Error('Playback requires explicit SCREENROOM_SHOWCASE_ITEM_ID, SCREENROOM_SHOWCASE_TITLE and SCREENROOM_SHOWCASE_YEAR.');
  const path = env.SCREENROOM_BACKEND_DESCRIPTOR || new URL('../../../build/backend-runtime/screenroom-jellyfin-local/descriptor.json', import.meta.url);
  const descriptor = JSON.parse(readFileSync(path, 'utf8'));
  if (!descriptor.ready || !uuid.test(descriptor.instanceID) || !uuid.test(descriptor.userID) || descriptor.publicOrigin !== endpoint || (env.SCREENROOM_USER_ID && env.SCREENROOM_USER_ID !== descriptor.userID)) throw new Error('Playback installation descriptor mismatch.');
  if (!env.SCREENROOM_BACKEND_INSTANCE_ID || env.SCREENROOM_BACKEND_INSTANCE_ID !== descriptor.instanceID) throw new Error('Playback requires SCREENROOM_BACKEND_INSTANCE_ID matching the attached descriptor.');
  return { item, instanceID: env.SCREENROOM_BACKEND_INSTANCE_ID, endpoint };
}

/** @param {string} value @param {string} origin @param {string} endpoint */
export function allowedPlaybackRequest(value, origin, endpoint) {
  const url = new URL(value);
  if (['blob:', 'data:'].includes(url.protocol)) return true;
  if (url.origin === origin) return !/^\/__(?:screenroom|operator)/.test(url.pathname);
  if (url.origin !== endpoint || url.username || url.password || url.hash) return false;
  if (/^\/artwork\//.test(url.pathname)) return true;
  if (/^\/screenroom\.catalog\.v1\.(?:BrowseService\/(?:GetHome|ListLibraries|ListLibraryItems)|CatalogService\/(?:GetCatalogInfo|GetItem|ListChildren|ListCredits|ListArtwork)|SearchService\/(?:GetSearchCapabilities|Search))$/.test(url.pathname)) return true;
  if (/^\/screenroom\.user\.v1\.UserService\/(?:GetCurrentUser|ListJellyfinSources|ListJellyfinProfiles|GetJellyfinLink)$/.test(url.pathname)) return true;
  if (/^\/screenroom\.playback_gateway\.v1\.PlaybackService\/(?:CreatePlaybackSession|GetPlaybackSession|UpdatePlaybackSession|ReportPlaybackEvent|StopPlaybackSession)$/.test(url.pathname)) return true;
  if (!/^\/playback\/v1\/media\/[a-f0-9-]{36}\/\d+\/[a-f0-9-]{36}$/.test(url.pathname)) return false;
  const fields = new Map();
  // Match the gateway's raw ResourceQuery grammar, not percent-decoded query aliases.
  for (const field of url.search.slice(1).split('&')) {
    const parts = field.split('=');
    if (parts.length !== 2 || !['cap', 'anchor', 'sig'].includes(parts[0]) || fields.has(parts[0])) return false;
    fields.set(parts[0], parts[1]);
  }
  if (!/^[a-f0-9]{64}$/.test(fields.get('cap') || '')) return false;
  if (fields.size === 1) return true;
  const anchor = fields.get('anchor') || '';
  return fields.size === 3 && uuid.test(anchor) && anchor !== '00000000-0000-0000-0000-000000000000'
    && /^[a-f0-9]{64}$/.test(fields.get('sig') || '');
}

/** @param {Manifest | null} previous @param {Selection} selection @param {string} profile @param {State[]} states @param {boolean} blocked */
export function mergeViewport(previous, selection, profile, states, blocked) {
  /** @type {Manifest} */
  const manifest = previous ?? { schema_version: 1, journey: 'playback', backend_instance_id: selection.instanceID, item: selection.item, platforms: [{ slug: 'web', runtime: 'Playwright Chromium · desktop 1440×900 and mobile 390×844', outcome: 'observed', states: [] }] };
  if (manifest.schema_version !== 1 || manifest.journey !== 'playback' || manifest.backend_instance_id !== selection.instanceID || JSON.stringify(manifest.item) !== JSON.stringify(selection.item) || manifest.platforms.length !== 1 || manifest.platforms[0].slug !== 'web') throw new Error('Foreign playback manifest.');
  const platform = manifest.platforms[0];
  const ids = new Set(platform.states.map(state => state.id));
  for (const state of states) {
    if (!state.id.startsWith(`${profile}-`) || ids.has(state.id)) throw new Error('Duplicate or foreign viewport state.');
    ids.add(state.id);
    if (typeof state.viewport !== 'string' || !state.viewport || !state.observation || (state.status === 'captured' ? !state.image || !/^[a-z0-9-]+\.png$/.test(state.image) : !state.reason || state.image)) throw new Error('Invalid playback state evidence.');
    platform.states.push(state);
  }
  if (blocked) platform.outcome = 'blocked';
  return manifest;
}

/** @param {string} directory @param {Selection} selection @param {string} profile @param {State[]} states @param {boolean} blocked */
export async function saveViewport(directory, selection, profile, states, blocked) {
  await mkdir(directory, { recursive: true });
  const path = `${directory}/manifest.json`;
  const previous = await readFile(path, 'utf8').then(JSON.parse).catch(error => { if (error.code === 'ENOENT') return null; throw error; });
  const manifest = mergeViewport(previous, selection, profile, states, blocked);
  await writeFile(`${path}.tmp`, `${JSON.stringify(manifest, null, 2)}\n`);
  await rename(`${path}.tmp`, path);
}
