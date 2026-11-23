import type { PageSnapshot } from './pagination';

const pages = new Map<string, PageSnapshot<{ id: string }>>();
const positions = new Map<string, { x: number; y: number; focus?: string }>();
let restoring = '';
export function savePages<T extends { id: string }>(key: string, value: PageSnapshot<T>) {
  if (value.entries.length > 1000) return;
  pages.delete(key); pages.set(key, value);
  while (pages.size > 8) pages.delete(pages.keys().next().value!);
}
export function savedPages<T extends { id: string }>(key: string): PageSnapshot<T> | undefined {
  return restoring === key ? pages.get(key) as PageSnapshot<T> | undefined : undefined;
}
export function rememberView(key: string) {
  const element = document.activeElement;
  const focus = element instanceof HTMLAnchorElement ? `a[href="${CSS.escape(element.getAttribute('href') ?? '')}"]`
    : element instanceof HTMLInputElement && element.type === 'search' ? 'input[type="search"]'
    : element instanceof HTMLButtonElement && element.closest('article.detail') ? 'article.detail button' : undefined;
  positions.delete(key); positions.set(key, { x: scrollX, y: scrollY, focus });
  while (positions.size > 32) positions.delete(positions.keys().next().value!);
}
export function restoreView(key: string, popstate: boolean) { restoring = popstate ? key : ''; }
export function viewReady(key: string) {
  if (restoring !== key) return;
  const position = positions.get(key);
  if (position) {
    document.querySelector<HTMLElement>(position.focus ?? '#main')?.focus({ preventScroll: true });
    window.scrollTo(position.x, position.y);
  }
}
export function clearViews() { pages.clear(); positions.clear(); restoring = ''; }
