import type { Item, Library } from '../core/models';
import { records, getItem, libraries, capabilities, scope, loadSettings } from './client';
import { defaultMethods, type Methods, type Types, type Source } from '../core/search';
import { MetadataOperation } from './operation.mjs';

export type Location = {
  screen: 'home' | 'browse' | 'search' | 'maintenance' | 'play';
  library: Library;
  path: Item[];
  query: string;
  types?: Types;
  methods: Methods;
  source: Source;
  sourceId: string;
  title?: Item;
  notFound?: boolean;
};
export function routeParameters(url: { pathname?: string; searchParams: Pick<URLSearchParams, 'get'> }): URLSearchParams {
  const p = new URLSearchParams();
  for (const key of ['screen', 'library', 'series', 'season', 'title', 'source_id', 'source', 'q', 'types', 'methods']) {
    const value = url.searchParams.get(key); if (value !== null) p.set(key, value);
  }
  const parts = (url.pathname ?? '/').split('/').filter(Boolean).map(part => { try { return decodeURIComponent(part); } catch { return '\0'; } });
  if (parts[0] === 'libraries') { p.set('screen', 'browse'); p.set('library', parts[1] ?? 'all'); }
  if (parts[0] === 'search' || parts[0] === 'maintenance' || parts[0] === 'play') p.set('screen', parts[0]);
  if (parts[0] === 'titles' || parts[0] === 'play') { p.set('title', parts[1] ?? ''); if (parts[0] === 'titles') p.set('screen', 'browse'); }
  return p;
}
export function readLocation(url: { pathname?: string; searchParams: Pick<URLSearchParams, 'get'> }, catalog = [...records.values()]): Location {
  const p = routeParameters(url);
  const screen = ['browse', 'search', 'maintenance', 'play'].includes(p.get('screen') ?? '') ? p.get('screen') as Location['screen'] : 'home';
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
  if ((screen === 'browse' || screen === 'play') && path.length && title) {
    const parentId = title.parent;
    const parent = catalog.find(item => item.id === parentId);
    if (title.kind !== 'Episode' || parent?.parent !== path[0].id || (path[1] && parent?.id !== path[1].id)) title = undefined;
  }
  const source: Source = capabilities?.sources.find(source => source === p.get('source')) ?? capabilities?.sources[0] ?? 'all';
  const selected = catalog.find(item => item.id === p.get('title'));
  let invalidAncestry = false;
  if (selected?.kind === 'Series' && ((series && series.id !== selected.id) || p.get('season'))) invalidAncestry = true;
  if (selected?.kind === 'Series') path.splice(0, path.length, selected);
  if (selected?.kind === 'Season') {
    const parent = catalog.find(item => item.id === selected.parent && item.kind === 'Series');
    if (!parent || (series && series.id !== parent.id) || (season && season.id !== selected.id)) invalidAncestry = true;
    if (parent) path.splice(0, path.length, parent, selected);
  }
  if (title?.kind === 'Episode') {
    const parent = catalog.find(item => item.id === title.parent && item.kind === 'Season');
    const ancestor = parent && catalog.find(item => item.id === parent.parent && item.kind === 'Series');
    if (parent && ancestor) path.splice(0, path.length, ancestor, parent);
    else invalidAncestry = true;
  }
  const notFound = !!(invalidAncestry || (p.get('library') && !libraries.some(l => l.id === p.get('library'))) ||
    (screen === 'play' && !title) ||
    (p.get('title') && !title && !path.some(item => item.id === p.get('title'))) ||
    (p.get('series') && !series) || (p.get('season') && !season));
  return { screen, library, path, query: (p.get('q') ?? '').slice(0, capabilities?.maximum_query_length ?? 160), types, methods, source, sourceId: p.get('source_id') || '', title, notFound };
}
export async function resolveLocation(url: URL, operation = new MetadataOperation()): Promise<Location> {
  url = new URL(url);
  url.search = routeParameters(url).toString();
  url.pathname = '/';
  const settings = await loadSettings(operation);
  operation.check();
  if (!url.searchParams.has('source')) url.searchParams.set('source', settings.source);
  if (!url.searchParams.has('methods') && settings.methods) url.searchParams.set('methods', Object.entries({ titleOnly: settings.methods.title_only, text: settings.methods.text, fuzzy: settings.methods.fuzzy, semantic: settings.methods.semantic }).filter(([, value]) => value).map(([key]) => key).join(','));
  const context = scope(url.searchParams.get('library') || 'all', url.searchParams.get('source_id') || settings.scope?.source_id || '');
  if (context.source_id) url.searchParams.set('source_id', context.source_id);
  const selected = (await operation.read(() => Promise.all(['series', 'season', 'title'].map(async key => { const id = url.searchParams.get(key); return id ? getItem(id, context, operation) : undefined; })))).filter((entry): entry is Item => !!entry);
  const title = selected.find(entry => entry.id === url.searchParams.get('title'));
  if (title?.parent) {
    const parent = await getItem(title.parent, context, operation);
    if (parent) {
      selected.push(parent);
      if (parent.parent) { const ancestor = await getItem(parent.parent, context, operation); if (ancestor) selected.push(ancestor); }
    }
  }
  operation.check();
  return readLocation(url, selected);
}
export function href(location: Location, changes: Partial<Location> = {}): string {
  const next = { ...location, ...changes };
  const p = new URLSearchParams();
  if (next.sourceId) p.set('source_id', next.sourceId);
  let pathname = next.screen === 'home' ? '/' : next.screen === 'browse' ? '/libraries' : `/${next.screen}`;
  if (next.screen === 'browse' || next.screen === 'search') {
    if (next.library.id && next.library.id !== 'all') p.set('library', next.library.id);
    if (next.screen === 'browse' && next.library.id !== 'all' && next.library.id) pathname += `/${encodeURIComponent(next.library.id)}`;
    if (next.screen === 'browse' && next.path.length) pathname = `/titles/${encodeURIComponent(next.path.at(-1)!.id)}`;
  }
  if (next.screen === 'search') {
    if (next.source !== 'all') p.set('source', next.source);
    if (next.query) p.set('q', next.query);
    if (next.types) p.set('types', Object.entries(next.types).filter(([, value]) => value).map(([key]) => key).join(','));
    p.set('methods', Object.entries(next.methods).filter(([, value]) => value).map(([key]) => key).join(','));
  }
  if (next.title) {
    pathname = `/titles/${encodeURIComponent(next.title.id)}`;
    if (next.library.id && next.library.id !== 'all') p.set('library', next.library.id);
  }
  return pathname + (p.size ? `?${p}` : '');
}
export function libraryHref(library: Library, sourceId = ''): string {
  return `/libraries${library.id === 'all' ? '' : `/${encodeURIComponent(library.id)}`}${sourceId ? `?source_id=${encodeURIComponent(sourceId)}` : ''}`;
}

export function safeReturn(value: string | null, fallback: string): string {
  if (!value || !value.startsWith('/') || value.startsWith('//') || /[\\\u0000-\u0020]/.test(value)) return fallback;
  const url = new URL(value, 'https://screenroom.invalid');
  return url.origin === 'https://screenroom.invalid' && /^\/(?:titles\/[^/]+|libraries(?:\/[^/]+)?|search)?$/.test(url.pathname) ? url.pathname + url.search : fallback;
}
let launch: string | undefined;
export function grantPlayback(destination: string) { launch = destination; }
export function consumePlayback(destination: string) { const allowed = launch === destination; launch = undefined; return allowed; }
export function clearPlaybackIntent() { launch = undefined; }
export function retainPlaybackIntent(destination: string) { if (launch !== destination) launch = undefined; }
export function itemHref(location: Location, item: Item): string {
  if (item.kind === 'Movie' || item.kind === 'Episode') return href(location, { title: item });
  return href(location, { screen: 'browse', path: [item], title: undefined });
}
