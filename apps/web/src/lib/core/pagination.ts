import { MetadataOperation } from './operation.mjs';

export type PageRequest = { size: number; token: string };
export async function collectPages<T extends { id: string }>(
  read: (page: PageRequest) => Promise<{ entries: T[]; next_page_token: string }>,
  active: () => boolean = () => true,
  identity: (entry: T) => string = entry => entry.id,
  operation = new MetadataOperation(active),
): Promise<T[]> {
  const entries = new Map<string, T>();
  const seen = new Set<string>();
  let token = '';
  let count = 0;
  try {
    do {
      if (!active()) return [];
      operation.check();
      if (seen.has(token)) throw new Error('Server repeated a pagination token.');
      seen.add(token);
      if (seen.size > 512) throw new Error('Pagination limit exceeded.');
      const page = await operation.read(() => read({ size: 100, token }));
      if (!active()) return [];
      operation.check();
      for (const entry of page.entries) {
        operation.check();
        if (++count > 10_000) throw new Error('Pagination item limit exceeded.');
        const key = identity(entry);
        if (!entries.has(key)) entries.set(key, entry);
      }
      token = page.next_page_token;
    } while (token);
    operation.check();
    return [...entries.values()];
  } catch (cause) {
    operation.stop(cause instanceof Error ? cause : new Error(String(cause)));
    throw cause;
  }
}
