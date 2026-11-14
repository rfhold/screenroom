import type { Item } from './models';

export function cardLabels(item: Item, seasonContext = false): { heading: string; primary: string; number?: string; subtitle: string } {
  if (item.kind !== 'Episode') return { heading: item.title, primary: item.title, subtitle: item.kind === 'Movie' ? String(item.year) : item.meta };

  // Public items currently carry episode numbering only in the catalog subtitle.
  const match = /^(?:(.+) · )?S(\d+)\s*E(\d+)(?: · .*)?$/u.exec(item.meta.trim());
  const episode = match ? `E${match[3].padStart(2, '0')}` : '';
  if (!match) return { heading: 'Episode', primary: 'Episode', subtitle: item.title };
  const number = seasonContext ? episode : `S${match[2].padStart(2, '0')}:${episode}`;
  const primary = seasonContext ? '' : match[1] || '';
  const heading = [primary, number].filter(Boolean).join(' · ');
  return { heading, primary, number, subtitle: item.title };
}
