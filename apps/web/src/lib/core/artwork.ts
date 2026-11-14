type Entry = {
  key: string;
  refs: number;
  state: 'queued' | 'running' | 'settled';
  loaded: Promise<string>;
  resolve: (url: string) => void;
  reject: (error: unknown) => void;
  url?: string;
};

export function createArtworkLoader(load: (url: string) => Promise<Uint8Array>) {
  const images = new Map<string, Entry>();
  const queue = new Set<Entry>();
  let running = 0;

  function discard(entry: Entry) {
    if (images.get(entry.key) === entry) images.delete(entry.key);
    if (entry.url) URL.revokeObjectURL(entry.url);
  }

  async function run(entry: Entry) {
    try {
      const bytes = await load(entry.key);
      if (entry.refs) {
        const data = new Uint8Array(bytes.length); data.set(bytes);
        const type = data[0] === 0x89 ? 'image/png' : data[0] === 0xff ? 'image/jpeg' : 'image/svg+xml';
        entry.url = URL.createObjectURL(new Blob([data], { type }));
      }
      entry.resolve(entry.url ?? '');
    } catch (error) {
      entry.reject(error);
    } finally {
      entry.state = 'settled';
      if (!entry.refs) discard(entry);
      running--;
      pump();
    }
  }

  function pump() {
    while (running < 8 && queue.size) {
      const entry = queue.values().next().value!;
      queue.delete(entry);
      entry.state = 'running';
      running++;
      void run(entry);
    }
  }

  return function acquireArtwork(key: string): { loaded: Promise<string>; release: () => void } {
    let entry = images.get(key);
    if (!entry) {
      let resolve!: Entry['resolve'];
      let reject!: Entry['reject'];
      const loaded = new Promise<string>((yes, no) => { resolve = yes; reject = no; });
      // A released consumer may no longer observe a failed in-flight request.
      void loaded.catch(() => {});
      entry = { key, refs: 0, state: 'queued', loaded, resolve, reject };
      images.set(key, entry);
      queue.add(entry);
    }
    const current = entry;
    current.refs++;
    pump();
    let released = false;
    return { loaded: current.loaded, release: () => {
      if (released) return;
      released = true;
      if (--current.refs) return;
      if (current.state === 'queued') {
        queue.delete(current);
        current.resolve('');
        discard(current);
      } else if (current.state === 'settled') {
        discard(current);
      }
    } };
  };
}
