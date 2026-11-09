export type PageRequest = { size: number; token: string };
export async function collectPages<T extends { id: string }>(
  read: (page: PageRequest) => Promise<{ entries: T[]; next_page_token: string }>,
  active: () => boolean = () => true,
  identity: (entry: T) => string = entry => entry.id,
): Promise<T[]> {
  const entries = new Map<string, T>();
  const seen = new Set<string>();
  let token = '';
  do {
    if (!active()) return [];
    if (seen.has(token)) throw new Error('Server repeated a pagination token.');
    seen.add(token);
    const page = await read({ size: 100, token });
    if (!active()) return [];
    for (const entry of page.entries) {
      const key = identity(entry);
      if (!entries.has(key)) entries.set(key, entry);
    }
    token = page.next_page_token;
  } while (token);
  return [...entries.values()];
}
