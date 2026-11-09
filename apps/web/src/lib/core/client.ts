import init, { Client, type ClientFailure, type Item as CoreItem, type Credit, type Artwork as CoreArtwork, type Preferences, type MediaLibrary, type SearchCapabilities } from '../../../../../target/client-core/wasm/screenroom_client_core';
import wasmUrl from '../../../../../target/client-core/wasm/screenroom_client_core_bg.wasm?url';
import type { Item, Artwork, PersonCredit, Rights } from './models';
import { collectPages } from './pagination';

let initialized: Promise<unknown> | undefined;
let instance: Promise<Client> | undefined;
export function client(): Promise<Client> {
  return instance ??= newClient();
}
export async function newClient(): Promise<Client> {
    await (initialized ??= init({ module_or_path: wasmUrl }));
    const endpoint = import.meta.env.VITE_SCREENROOM_API_ENDPOINT || 'http://127.0.0.1:50051';
    const profile = sessionStorage.getItem('screenroom.mockProfile') || import.meta.env.VITE_SCREENROOM_MOCK_PROFILE || 'web';
    return new Client({ endpoint, mock_profile: profile, timeout_ms: 3000 });
}
export let libraries: MediaLibrary[] = [];
export let capabilities: SearchCapabilities | undefined;
export function configure(available: MediaLibrary[], search: SearchCapabilities) { libraries = available; capabilities = search; }
export function failure(error: unknown): string {
  const e = error as Partial<ClientFailure>;
  return `${e.code ?? 'unavailable'}: ${e.message ?? 'The mock server could not be reached.'}`;
}
export const records = new Map<string, Item>();
export const rights = (r: CoreItem['title_rights']): Rights => ({ label: r?.label || 'Rights not supplied', note: r?.note || '', source: r?.source || '', licenseUrl: r?.license_url, attribution: r?.attribution });
export const artwork = (a: CoreArtwork | null): Artwork | undefined => a ? { ...a, type: a.kind, licenseUrl: a.license_url } : undefined;
export const credit = (c: Credit): PersonCredit => ({ id: c.id, name: c.name, role: c.role, artwork: artwork(c.artwork) });
export function item(record: CoreItem): Item {
  const value: Item = { id: record.id, title: record.title, kind: record.kind as Item['kind'], year: record.year, parent: record.parent_id ?? undefined, meta: record.subtitle, overview: record.overview, tags: record.tags.join(' '), colors: record.colors, addedOrder: record.added_order, releaseDate: record.release_date, progress: record.progress_percent ?? undefined, rights: rights(record.title_rights), metadataRights: rights(record.metadata_rights), provenanceNotes: record.provenance_notes, metadataRetrievedAt: record.metadata_retrieved_at, artwork: artwork(record.artwork), cast: [], crew: [], rating: record.rating ? { provider: record.rating.provider, value: record.rating.value, scale: record.rating.scale, count: record.rating.votes, retrievedAt: record.rating.retrieved_at } : undefined, providers: record.provider_references };
  records.set(value.id, value);
  return value;
}
export async function getItem(id: string): Promise<Item | undefined> {
  try { return item(await (await client()).get_item(id)); }
  catch (error) { if ((error as ClientFailure).code === 'not_found' || (error as ClientFailure).code === 'invalid_argument') return; throw error; }
}
export async function allCredits(id: string) {
  const c = await client();
  return collectPages(async page => { const result = await c.list_credits(id, page); return { entries: result.credits, next_page_token: result.next_page_token }; }, undefined, credit => JSON.stringify([credit.id, credit.department, credit.role]));
}
export async function preferences(): Promise<Preferences> { return (await client()).get_preferences(); }
let updates = Promise.resolve();
export function updatePreferences(change: Partial<Preferences>, mask: string[]): Promise<void> {
  const operation = updates.then(async () => { const c = await client(); const current = await c.get_preferences(); await c.update_preferences({ ...current, ...change }, mask); });
  updates = operation.catch(() => {});
  return operation;
}

const images = new Map<string, { promise: Promise<string>; refs: number; url?: string }>();
export function acquireArtwork(url: string): { loaded: Promise<string>; release: () => void } {
  let entry = images.get(url);
  if (!entry) {
    entry = { refs: 0, promise: Promise.resolve('') };
    const current = entry;
    entry.promise = client().then(c => c.load_artwork(url)).then(bytes => {
      const data = new Uint8Array(bytes.length); data.set(bytes);
      current.url = URL.createObjectURL(new Blob([data]));
      return current.url;
    });
    images.set(url, entry);
  }
  entry.refs++;
  return { loaded: entry.promise, release: () => {
    if (!entry || --entry.refs > 0) return;
    images.delete(url);
    void entry.promise.then(value => URL.revokeObjectURL(value), () => {});
  } };
}
