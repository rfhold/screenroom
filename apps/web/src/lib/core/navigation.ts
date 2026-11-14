import type { Item, Library } from '../core/models';
import { records, getItem, libraries, capabilities, scope, loadSettings } from './client';
import { defaultMethods, type Methods, type Types, type Source } from '../core/search';
import { MetadataOperation } from './operation.mjs';

export type Location = {
  screen: 'home' | 'browse' | 'search' | 'maintenance';
  library: Library;
  path: Item[];
  query: string;
  types?: Types;
  methods: Methods;
  source: Source;
  sourceId: string;
  title?: Item;
};
export function readLocation(url: { searchParams: Pick<URLSearchParams, 'get'> }, catalog = [...records.values()]): Location {
  const p = url.searchParams;
  const screen = ['browse', 'search', 'maintenance'].includes(p.get('screen') ?? '') ? p.get('screen') as Location['screen'] : 'home';
  const library = (screen !== 'home' && libraries.find(entry => entry.id === p.get('library')) || libraries[0]) ?? { id: '', title: '' };
  const series = catalog.find(item => item.id === p.get('series') && item.kind === 'Series');
  const season = series && catalog.find(item => item.id === p.get('season') && item.kind === 'Season' && item.parent === series.id);
  const path = screen === 'home' ? [] : [...(series ? [series] : []), ...(season ? [season] : [])];
  const rawTypes = p.get('types')?.split(',');
  const types = rawTypes && rawTypes.length && rawTypes.every(type => ['Movie', 'Series', 'Episode'].includes(type))
    ? { Movie: rawTypes.includes('Movie'), Series: rawTypes.includes('Series'), Episode: rawTypes.includes('Episode') } : undefined;
  const rawMethods = p.get('methods')?.split(',');
  let methods = defaultMethods();
  if (rawMethods?.some(method => ['text', 'fuzzy', 'semantic'].includes(method)) && rawMethods.every(method => ['titleOnly', 'text', 'fuzzy', 'semantic'].includes(method))) {
    methods = { titleOnly: rawMethods.includes('titleOnly'), text: rawMethods.includes('text'), fuzzy: rawMethods.includes('fuzzy'), semantic: rawMethods.includes('semantic') };
    if (methods.titleOnly) { methods.semantic = false; if (!methods.text && !methods.fuzzy) methods.text = true; }
  }
  let title = catalog.find(item => item.id === p.get('title') && ['Movie', 'Episode'].includes(item.kind));
  if (screen === 'browse' && path.length && title) {
    const parentId = title.parent;
    const parent = catalog.find(item => item.id === parentId);
    if (title.kind !== 'Episode' || parent?.parent !== path[0].id || (path[1] && parent?.id !== path[1].id)) title = undefined;
  }
  const source: Source = capabilities?.sources.find(source => source === p.get('source')) ?? capabilities?.sources[0] ?? 'all';
  return { screen, library, path, query: (p.get('q') ?? '').slice(0, capabilities?.maximum_query_length ?? 160), types, methods, source, sourceId: p.get('source_id') || '', title };
}
export async function resolveLocation(url: URL, operation = new MetadataOperation()): Promise<Location> {
  const settings = await loadSettings(operation);
  operation.check();
  if (!url.searchParams.has('library') && url.searchParams.get('screen') !== null && settings.scope?.library_id) url.searchParams.set('library', settings.scope.library_id);
  if (!url.searchParams.has('source')) url.searchParams.set('source', settings.source);
  if (!url.searchParams.has('methods') && settings.methods) url.searchParams.set('methods', Object.entries({ titleOnly: settings.methods.title_only, text: settings.methods.text, fuzzy: settings.methods.fuzzy, semantic: settings.methods.semantic }).filter(([, value]) => value).map(([key]) => key).join(','));
  const context = scope(url.searchParams.get('library') || 'all', url.searchParams.get('source_id') || settings.scope?.source_id || '');
  if (context.source_id) url.searchParams.set('source_id', context.source_id);
  const selected = (await operation.read(() => Promise.all(['series', 'season', 'title'].map(async key => { const id = url.searchParams.get(key); return id ? getItem(id, context, operation) : undefined; })))).filter((entry): entry is Item => !!entry);
  const title = selected.find(entry => entry.id === url.searchParams.get('title'));
  if (title?.parent) { const parent = await getItem(title.parent, context, operation); if (parent) selected.push(parent); }
  operation.check();
  return readLocation(url, selected);
}
export function href(location: Location, changes: Partial<Location> = {}): string {
  const next = { ...location, ...changes };
  const p = new URLSearchParams();
  if (next.sourceId) p.set('source_id', next.sourceId);
  if (next.screen !== 'home') p.set('screen', next.screen);
  if (next.screen === 'browse' || next.screen === 'search') {
    p.set('library', next.library.id);
    if (next.path[0]) p.set('series', next.path[0].id);
    if (next.path[1]) p.set('season', next.path[1].id);
  }
  if (next.screen === 'search') {
    if (next.source !== 'all') p.set('source', next.source);
    if (next.query) p.set('q', next.query);
    if (next.types) p.set('types', Object.entries(next.types).filter(([, value]) => value).map(([key]) => key).join(','));
    p.set('methods', Object.entries(next.methods).filter(([, value]) => value).map(([key]) => key).join(','));
  }
  if (next.title) p.set('title', next.title.id);
  return p.size ? `/?${p}` : '/';
}
export function libraryHref(library: Library, sourceId = ''): string {
  return `/?screen=browse&library=${encodeURIComponent(library.id)}${sourceId ? `&source_id=${encodeURIComponent(sourceId)}` : ''}`;
}
export function itemHref(location: Location, item: Item): string {
  if (item.kind === 'Movie' || item.kind === 'Episode') return href(location, { title: item });
  const parent = item.kind === 'Season' ? records.get(item.parent!)! : item;
  return href(location, { screen: 'browse', path: item.kind === 'Season' ? [parent, item] : [item], title: undefined });
}
