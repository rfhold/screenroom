export class OwnedReads {
  private pending = new Set<Promise<unknown>>();
  private disposed = false;
  private owner: { cancel_all: () => unknown; free: () => void };
  constructor(owner: { cancel_all: () => unknown; free: () => void }) { this.owner = owner; }
  read<T>(start: () => Promise<T>): Promise<T> {
    if (this.disposed) return Promise.reject(new Error('Read owner disposed.'));
    const request = start();
    this.pending.add(request);
    void request.then(() => this.pending.delete(request), () => this.pending.delete(request));
    return request;
  }
  dispose() {
    if (this.disposed) return;
    this.disposed = true;
    this.owner.cancel_all();
    void Promise.allSettled([...this.pending]).then(() => this.owner.free());
  }
}
