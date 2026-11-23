import { MetadataOperation } from './operation.mjs';

export type PageRequest = { size: number; token: string };
export type PageSnapshot<T> = { entries: T[]; more: boolean; token: string; seen: string[]; count: number; status?: string };
export class IncrementalPages<T extends { id: string }> {
  entries: T[] = [];
  more = true;
  loading = false;
  private token = '';
  private seen = new Set<string>();
  private count = 0;
  private stopped = false;
  private operation?: MetadataOperation;
  private flight?: Promise<void>;
  private read: (page: PageRequest) => Promise<{ entries: T[]; next_page_token: string }>;
  private active: () => boolean;
  constructor(read: (page: PageRequest) => Promise<{ entries: T[]; next_page_token: string }>, active = () => true) { this.read = read; this.active = active; }
  snapshot(): PageSnapshot<T> { return { entries: this.entries, more: this.more, token: this.token, seen: [...this.seen], count: this.count }; }
  restore(value: PageSnapshot<T>) { this.entries = value.entries; this.more = value.more; this.token = value.token; this.seen = new Set(value.seen); this.count = value.count; }
  cancel() { this.stopped = true; this.operation?.cancel(); }
  load(operation = new MetadataOperation(this.active)): Promise<void> {
    if (this.flight) return this.flight;
    if (this.stopped || !this.active() || !this.more) return Promise.resolve();
    this.loading = true;
    this.operation = operation;
    this.flight = (async () => {
      try {
        const response = await operation.read(() => this.read({ size: 100, token: this.token }));
        if (this.stopped || !this.active()) return;
        operation.check();
        const count = this.count + response.entries.length;
        const next = response.next_page_token;
        if (count > 10_000 || (next && this.seen.size + 1 >= 512) || (next && (next === this.token || this.seen.has(next)))) {
          this.more = false;
          throw new Error('Pagination token cycle or limit exceeded.');
        }
        const ids = new Set(this.entries.map(entry => entry.id));
        const added = response.entries.filter(entry => { if (ids.has(entry.id)) return false; ids.add(entry.id); return true; });
        this.entries = [...this.entries, ...added];
        this.count = count;
        this.seen.add(this.token);
        this.token = next;
        this.more = !!next;
      } finally { this.loading = false; this.flight = undefined; this.operation = undefined; }
    })();
    return this.flight;
  }
}

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
