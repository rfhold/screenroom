import { client, item, loadSettings, saveSettings } from './client';
import type { PageRequest } from './pagination';
import type { Library } from './models';
import { MetadataOperation } from './operation.mjs';
export type Types = { Movie: boolean; Series: boolean; Episode: boolean };
export type Methods = { titleOnly: boolean; text: boolean; fuzzy: boolean; semantic: boolean };
export type Source = string;
export const defaultTypes = (): Types => ({ Movie: true, Series: true, Episode: false });
export const defaultMethods = (): Methods => ({ titleOnly: false, text: true, fuzzy: true, semantic: true });
export async function loadTypes(operation?: MetadataOperation): Promise<Types> { const p = await loadSettings(operation); return p.types ? { Movie: p.types.movie, Series: p.types.series, Episode: p.types.episode } : defaultTypes(); }
export function saveTypes(types: Types) { return saveSettings({ types: { movie: types.Movie, series: types.Series, episode: types.Episode } }); }
export async function search(query: string, library: Library, types: Types, methods: Methods, source: Source, transport: Pick<Awaited<ReturnType<typeof client>>, 'search'>, active: () => boolean, source_id = '', operation = new MetadataOperation(active), page: PageRequest = { size: 100, token: '' }) {
  const response = await operation.read(() => transport.search({ query, source_id, library_id: library.id === 'all' ? '' : library.id, types: { movie: types.Movie, series: types.Series, episode: types.Episode }, methods: { title_only: methods.titleOnly, text: methods.text, fuzzy: methods.fuzzy, semantic: methods.semantic }, source, page }));
  const degradation = response.degradation_reason;
  const effective = Object.entries(response.effective_methods ?? {}).filter(([, value]) => value).map(([key]) => key).join(', ');
  operation.check();
  return { entries: response.items.map(record => { operation.check(); return item(record); }), next_page_token: response.next_page_token, degradation, effective };
}
