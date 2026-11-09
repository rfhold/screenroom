import { client, item, preferences, updatePreferences } from './client';
import { collectPages } from './pagination';
import type { Library } from './models';
export type Types = { Movie: boolean; Series: boolean; Episode: boolean };
export type Methods = { titleOnly: boolean; text: boolean; fuzzy: boolean; semantic: boolean };
export type Source = string;
export const defaultTypes = (): Types => ({ Movie: true, Series: true, Episode: false });
export const defaultMethods = (): Methods => ({ titleOnly: false, text: true, fuzzy: true, semantic: true });
export async function loadTypes(): Promise<Types> { const p = await preferences(); return p.search_types ? { Movie: p.search_types.movie, Series: p.search_types.series, Episode: p.search_types.episode } : defaultTypes(); }
export function saveTypes(types: Types) { return updatePreferences({ search_types: { movie: types.Movie, series: types.Series, episode: types.Episode } }, ['search_types']); }
export async function search(query: string, library: Library, types: Types, methods: Methods, source: Source, transport: Awaited<ReturnType<typeof client>>, active: () => boolean) {
  const results = await collectPages(async page => {
    const response = await transport.search({ query, library_id: library.id, types: { movie: types.Movie, series: types.Series, episode: types.Episode }, methods: { title_only: methods.titleOnly, text: methods.text, fuzzy: methods.fuzzy, semantic: methods.semantic }, source, page });
    return { entries: response.items, next_page_token: response.next_page_token };
  }, active);
  return results.map(item);
}
