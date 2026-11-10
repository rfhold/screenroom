export class MetadataCancelled extends Error {
  constructor() { super('Metadata operation cancelled.'); }
}

export class MetadataOperation {
  /** @type {Error | undefined} */
  stopped;
  /** @type {Set<(error: Error) => void>} */
  waiting = new Set();
  /** @param {() => boolean} active @param {{ now?: () => number; timeoutMs?: number }} options */
  constructor(active = () => true, { now = () => performance.now(), timeoutMs = 120_000 } = {}) {
    this.active = active;
    this.now = now;
    this.deadline = now() + timeoutMs;
  }
  check() {
    if (!this.active()) { this.cancel(); throw new MetadataCancelled(); }
    if (this.stopped) throw this.stopped;
    if (this.now() >= this.deadline) {
      const error = Object.assign(new Error('Metadata operation deadline exceeded.'), { code: 'deadline_exceeded' });
      this.stop(error);
      throw error;
    }
  }
  /** @param {Error} error */
  stop(error) {
    if (this.stopped) return;
    this.stopped = error;
    for (const reject of this.waiting) reject(error);
    this.waiting.clear();
  }
  cancel() { this.stop(new MetadataCancelled()); }
  /** @template T @param {() => T | PromiseLike<T>} start @returns {Promise<T>} */
  async read(start) {
    this.check();
    /** @type {ReturnType<typeof setTimeout> | undefined} */
    let timer;
    /** @type {(error: Error) => void} */
    let rejectWait = () => {};
    const expired = new Promise((_, reject) => {
      rejectWait = reject;
      this.waiting.add(rejectWait);
      timer = setTimeout(() => {
        this.stop(Object.assign(new Error('Metadata operation deadline exceeded.'), { code: 'deadline_exceeded' }));
      }, Math.max(1, this.deadline - this.now()));
    });
    try {
      const value = await Promise.race([Promise.resolve().then(() => { this.check(); return start(); }), expired]);
      this.check();
      return /** @type {T} */ (value);
    } catch (cause) {
      this.stop(cause instanceof Error ? cause : new Error(String(cause)));
      throw cause;
    } finally {
      clearTimeout(timer);
      this.waiting.delete(rejectWait);
    }
  }
}
