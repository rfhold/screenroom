import init, { Client, default_settings, normalize_saved_settings, validate_settings, type ClientSettings, type SourceLibraryScope, type ClientFailure, type Item as CoreItem, type Credit, type Artwork as CoreArtwork, type MediaLibrary, type SearchCapabilities } from '../../../../../target/client-core/wasm/screenroom_client_core';
import wasmUrl from '../../../../../target/client-core/wasm/screenroom_client_core_bg.wasm?url';
import type { Item, Artwork, PersonCredit, Rights } from './models';
import { collectPages } from './pagination';
import { writable } from 'svelte/store';
import { MetadataOperation } from './operation.mjs';
import { createArtworkLoader } from './artwork';
import { webConfig } from './config';

let initialized: Promise<unknown> | undefined;
let instance: Promise<Client> | undefined;
export function client(): Promise<Client> {
  return instance ??= newClient();
}
export async function newClient(): Promise<Client> {
    await (initialized ??= init({ module_or_path: wasmUrl }));
    return new Client({ ...webConfig(), timeout_ms: 3000 });
}
export let libraries: MediaLibrary[] = [];
export let capabilities: SearchCapabilities | undefined;
export function configure(available: MediaLibrary[], search: SearchCapabilities) { libraries = available; search.types = search.types.map(type => ({ movie: 'Movie', series: 'Series', episode: 'Episode' })[type] || type); capabilities = search; }
export function failure(error: unknown): string {
  const e = error as Partial<ClientFailure>;
  return `${e.code ?? 'unavailable'}: ${e.message ?? 'The server could not be reached.'}`;
}
export const records = new Map<string, Item>();
export const rights = (r: CoreItem['title_rights']): Rights => ({ label: r?.label || 'Rights not supplied', note: r?.note || '', source: r?.source || '', licenseUrl: r?.license_url, attribution: r?.attribution });
export const artwork = (a: CoreArtwork | null): Artwork | undefined => a ? { ...a, type: a.kind, licenseUrl: a.license_url } : undefined;
export const credit = (c: Credit): PersonCredit => ({ id: c.id, name: c.name, role: c.role, artwork: artwork(c.artwork) });
export function item(record: CoreItem): Item {
  const value: Item = { id: record.id, title: record.title, kind: record.kind as Item['kind'], year: record.year, parent: record.parent_id ?? undefined, meta: record.subtitle, overview: record.overview, tags: record.tags.join(' '), colors: record.colors, addedOrder: record.added_order, releaseDate: record.release_date, progress: record.progress_percent ?? undefined, rights: rights(record.title_rights), metadataRights: rights(record.metadata_rights), provenanceNotes: record.provenance_notes, metadataRetrievedAt: record.metadata_retrieved_at, artwork: artwork(record.artwork), cast: [], crew: [], rating: record.rating ? { provider: record.rating.provider, value: record.rating.value, scale: record.rating.scale, count: record.rating.votes, retrievedAt: record.rating.retrieved_at } : undefined, providers: record.provider_references };
  value.watched = record.watched ?? null;
  records.set(value.id, value);
  return value;
}
export const scope = (library = 'all', source_id = ''): SourceLibraryScope => ({ source_id, library_id: library === 'all' ? '' : library });
export async function getItem(id: string, context = scope(), operation = new MetadataOperation()): Promise<Item | undefined> {
  const c = await operation.read(client);
  const record = await operation.read(() => c.get_item(id, context).catch(error => {
    if ((error as ClientFailure).code === 'not_found' || (error as ClientFailure).code === 'invalid_argument') return;
    throw error;
  }));
  operation.check();
  return record ? item(record) : undefined;
}
export async function allCredits(id: string, context = scope(), operation = new MetadataOperation()) {
  const c = await operation.read(client);
  return collectPages(async page => { const result = await c.list_credits(id, page, context); return { entries: result.credits, next_page_token: result.next_page_token }; }, undefined, credit => JSON.stringify([credit.id, credit.department, credit.role]), operation);
}
export const settingsWarning = writable('');
const settingsCache = new Map<string, ClientSettings>();
const persistenceWarning = 'Settings cannot be saved on this device. Current choices remain available until this page is closed.';
function settingsKey() {
  let namespace = 'default';
  try { namespace = localStorage.getItem('screenroom.settingsNamespace') || namespace; }
  catch { settingsWarning.set(persistenceWarning); }
  const { endpoint, user_id } = webConfig();
  return `screenroom.settings.v1:${JSON.stringify([location.origin, endpoint, user_id, namespace])}`;
}
export async function loadSettings(operation = new MetadataOperation()): Promise<ClientSettings> {
  await operation.read(client);
  operation.check();
  const key = settingsKey();
  const cached = settingsCache.get(key);
  if (cached) return cached;
  const defaults = () => ({ ...default_settings(), types: { movie: true, series: true, episode: false } });
  let settings: ClientSettings = defaults();
  let raw: string | null = null;
  try { raw = localStorage.getItem(key); }
  catch { settingsWarning.set(persistenceWarning); }
   try { if (raw) settings = normalize_saved_settings(JSON.parse(raw)); }
  catch { settings = defaults(); }
  operation.check();
  settingsCache.set(key, settings);
  return settings;
}
let settingsWrites = Promise.resolve();
export function saveSettings(change: Partial<ClientSettings>, active = () => true, budget = new MetadataOperation()): Promise<void> {
  const write = settingsWrites.then(async () => {
    budget.check();
    if (!active()) return;
    const current = await loadSettings(budget);
    const next = validate_settings({ ...current, ...change });
    if (!active()) return;
    budget.check();
    const key = settingsKey();
    settingsCache.set(key, next);
    try { localStorage.setItem(key, JSON.stringify(next)); settingsWarning.set(''); }
    catch { settingsWarning.set(persistenceWarning); }
  });
  settingsWrites = write.catch(() => {});
  return budget.read(() => write);
}

let artworkClient: Promise<Client> | undefined;
export const acquireArtwork = createArtworkLoader(url => (artworkClient ??= newClient()).then(c => c.load_artwork(url)));
