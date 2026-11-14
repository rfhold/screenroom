import type { OperatorConnectionDiscovery } from '../../../../../target/client-core/wasm/screenroom_client_core';
export function selectionError(discovery: OperatorConnectionDiscovery | null, profile: string, libraries: string[]): string {
  if (!discovery) return 'Connect to your server first.';
  if (!discovery.profiles.some(p => p.id === profile)) return 'Choose an available Jellyfin profile.';
  if (!libraries.length || libraries.length > 32 || new Set(libraries).size !== libraries.length) return 'Choose between 1 and 32 distinct libraries.';
  const selected = libraries.map(id => discovery.libraries.find(l => l.id === id));
  if (selected.some(l => !l?.supported || l.kind === 'unsupported')) return 'Choose only supported libraries.';
  if (selected.filter(l => l?.kind === 'collections').length > 1) return 'Choose at most one collections library.';
  return '';
}
export function connectionUrlError(value: string): string {
  try { const url = new URL(value); if (!['http:','https:'].includes(url.protocol) || url.username || url.password || url.search || url.hash) throw new Error(); return ''; }
  catch { return 'Enter an HTTP or HTTPS server URL without credentials, a query, or a fragment.'; }
}
