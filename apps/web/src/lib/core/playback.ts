import type {
  Client, PlaybackSession, PlaybackTransport, PlaybackEventKind, PlaybackTrackChoices,
  CreatePlaybackSessionRequest, UpdatePlaybackSessionRequest, SourceLibraryScope,
  PlaybackClientCapabilities, StopPlaybackSessionRequest,
} from '../../../../../target/client-core/wasm/screenroom_client_core';
import { observeOperation, type Context } from './telemetry';

export function milliseconds(value: string): bigint {
  if (!/^\d{1,20}$/.test(value)) throw new Error('Invalid playback timeline.');
  const ms = BigInt(value);
  if (ms > 922337203685477n) throw new Error('Playback timeline exceeds supported range.');
  return ms;
}
export function seconds(value: string): number {
  const ms = milliseconds(value);
  if (ms > BigInt(Number.MAX_SAFE_INTEGER)) throw new Error('Playback timeline cannot be represented safely.');
  const result = Number(ms) / 1000;
  if (Math.round(result * 1000) !== Number(ms)) throw new Error('Playback timeline cannot be represented safely.');
  return result;
}
export function position(currentTime: number, offset: string): string {
  const ms = Math.round(currentTime * 1000);
  if (!Number.isSafeInteger(ms) || ms < 0) throw new Error('Playback position cannot be represented safely.');
  const result = (BigInt(ms) + milliseconds(offset)).toString();
  milliseconds(result);
  return result;
}
export function mediaURL(value: string, endpoint: string, session: PlaybackSession): string {
  const url = new URL(value);
  const prefix = `/playback/v1/media/${session.session_id}/${session.transport_revision}/`;
  if (url.origin !== new URL(endpoint).origin || url.username || url.password || url.hash
    || !url.pathname.startsWith(prefix)
    || !/^[a-f0-9-]{36}$/.test(url.pathname.slice(prefix.length))
    || [...url.searchParams].length !== 1 || !/^[a-f0-9]{64}$/.test(url.searchParams.get('cap') || '')
    || url.searchParams.get('cap') === session.control_capability) throw new Error('Unsafe gateway media response.');
  return url.href;
}
export interface Media {
  currentTime: number;
  positionReady?: boolean;
  paused: boolean;
  muted: boolean;
  volume: number;
  detach(): void;
  attach(transport: PlaybackTransport, session: PlaybackSession, restore?: { position_ms: string; paused: boolean }): Promise<void>;
}
type PlaybackClient = Pick<Client, 'create_playback_session' | 'get_playback_session' | 'update_playback_session' | 'report_playback_event' | 'stop_playback_session' | 'cancel_all' | 'invalidate_reads'>;
const terminal = (session: PlaybackSession) => ['stopped', 'stopping', 'expired', 'failed', 'uncertain'].includes(session.state);
const unknown = (session: PlaybackSession) => !!session.pending_operation || ['pending', 'uncertain'].includes(session.report_outcome);

export class Playback {
  session: PlaybackSession | null = null;
  status = 'Ready to request playback.';
  busy = false;
  closed = false;
  started = false;
  blocked = false;
  fallbackUsed = false;
  private sequence = 0;
  private chain: Promise<unknown> = Promise.resolve();
  private creation: CreatePlaybackSessionRequest | null = null;
  private pendingUpdate: UpdatePlaybackSessionRequest | null = null;
  private stopBody: StopPlaybackSessionRequest | null = null;
  private heartbeat: ReturnType<typeof setInterval> | undefined;
  private lastProgress = 0;
  private uncertainStart = false;
  private attachedTransport: PlaybackTransport | null = null;
  private lastMediaState: { revision: number; position_ms: string; paused: boolean } | null = null;
  private exitPosition: string | null = null;
  private exitReason: string | null = null;
  constructor(private client: PlaybackClient, private media: Media, private changed: () => void, private personalChanged: () => void = () => {}, private traceContext?: Context) {}
  private notify() { this.changed(); }
  settled() { return this.chain; }
  get canRetryTransport() { return !!this.pendingUpdate && !this.closed && !this.blocked && !this.busy; }
  private enqueue(task: () => Promise<void>): Promise<void> {
    const next = this.chain.then(task);
    this.chain = next.catch(() => {});
    return next;
  }
  private snapshotPosition() {
    if (this.exitPosition !== null) return this.exitPosition;
    if (this.attachedTransport && this.media.positionReady !== false) {
      this.lastMediaState = {
        revision: this.attachedTransport.revision,
        position_ms: position(this.media.currentTime, this.attachedTransport.timeline_offset_ms),
        paused: this.media.paused,
      };
    }
    return this.lastMediaState?.position_ms ?? '0';
  }
  private detachMedia() {
    try { this.snapshotPosition(); } catch {}
    this.attachedTransport = null;
    this.media.detach();
  }
  private authority() {
    if (!this.session) throw new Error('No playback session.');
    return { session_id: this.session.session_id, control_capability: this.session.control_capability };
  }
  private async accept(session: PlaybackSession | null, replace: boolean) {
    if (!session) throw new Error('Missing playback session.');
    if (this.session && (session.session_id !== this.session.session_id || session.transport_revision < this.session.transport_revision)) throw new Error('Stale playback response.');
    this.session = session;
    this.sequence = Math.max(this.sequence, session.last_accepted_sequence);
    if (this.closed) { await this.stop(this.exitReason ?? 'player_exit'); return; }
    this.blocked = terminal(session) || unknown(session);
    this.status = session.failure_code ? `Playback unavailable: ${session.failure_code}.` : `Personal progress: ${session.report_outcome}.`;
    if (this.blocked) {
      this.detachMedia();
      if (unknown(session) || session.state === 'uncertain') this.status = 'Personal playback outcome is unknown. Playback has been stopped; no start report will be repeated.';
      if (this.heartbeat) clearInterval(this.heartbeat);
      if (unknown(session) || session.state === 'uncertain') { this.closed = true; await this.stop('uncertain_outcome'); }
    } else if (replace && session.transport && this.attachedTransport?.revision !== session.transport.revision) {
      this.detachMedia();
      const restore = {
        position_ms: this.lastMediaState?.revision === session.transport.revision ? this.lastMediaState.position_ms : session.transport.start_position_ms,
        paused: this.lastMediaState?.paused ?? false,
      };
      this.lastMediaState = { revision: session.transport.revision, ...restore };
      this.attachedTransport = session.transport;
      await this.media.attach(session.transport, session, restore);
    } else if (!session.transport) {
      this.detachMedia();
      this.status = 'Playback is not ready. Check session state; starting again is not automatic.';
    }
    this.notify();
  }
  private async reconcile(replace = false) {
    if (!this.session) return;
    const response = await this.client.get_playback_session(this.authority());
    await this.accept(response.session, replace);
  }
  async check() {
    if (this.closed) return;
    return this.enqueue(async () => {
      try { await this.reconcile(true); }
      catch { this.status = 'Session state could not be confirmed. Playback remains stopped.'; this.detachMedia(); this.blocked = true; this.notify(); }
    });
  }
  async create(item_id: string, scope: SourceLibraryScope, caps: PlaybackClientCapabilities, beginning = false) {
    if (this.creation || this.closed) return;
    this.creation = { request_id: crypto.randomUUID(), item_id, scope: { ...scope }, client_capabilities: { ...caps }, start_policy: beginning ? 'beginning' : 'resume', position_ms: null };
    return this.retryCreate();
  }
  async retryCreate() {
    if (!this.creation || this.session || this.closed) return;
    return this.enqueue(async () => {
      if (this.session || this.closed) return;
      this.busy = true; this.status = 'Preparing playback…'; this.notify();
      try {
        const response = await observeOperation('playback.create', traceParent => this.client.create_playback_session(this.creation!, traceParent), this.traceContext);
        await this.accept(response.session, true);
        if (!this.closed && !this.blocked && response.session?.transport) this.heartbeat = setInterval(() => { void this.event('heartbeat'); }, 15_000);
      } catch (error) {
        const code = (error as { playback_code?: string; code?: string }).playback_code;
        this.status = code ? `Playback unavailable: ${code}.` : 'Playback preparation was not confirmed. Retry uses the same request; it does not start a new session.';
      } finally { this.busy = false; this.notify(); }
    });
  }
  event(kind: PlaybackEventKind) {
    if (this.closed || this.blocked || !this.session?.transport || (this.busy && kind !== 'started')) return Promise.resolve();
    if (kind === 'started') {
      if (this.started) return Promise.resolve();
    } else if (!this.started) return Promise.resolve();
    const revision = this.session.transport_revision;
    const position_ms = this.snapshotPosition();
    const paused = this.media.paused, muted = this.media.muted, volume = this.media.volume;
    return this.enqueue(async () => {
      if (this.closed || this.blocked || revision !== this.session?.transport_revision) return;
      if (this.sequence >= 0xffffffff) { this.exit('sequence_exhausted'); return; }
      if (kind === 'started') {
        if (this.started) return;
        this.started = true;
      }
      const request = { ...this.authority(), transport_revision: revision, sequence: ++this.sequence, kind, position_ms, paused, muted, volume };
      try {
        const response = await this.client.report_playback_event(request);
        await this.accept(response.session, false);
        if (response.report_outcome === 'confirmed') { this.client.invalidate_reads(); this.personalChanged(); }
      } catch {
        if (kind === 'started') this.uncertainStart = true;
        this.blocked = true; this.detachMedia();
        this.status = 'Playback report was not confirmed. Checking session state; the event will not be replayed.';
        try { await this.reconcile(); }
        catch { this.status = 'Personal playback outcome is unknown. Playback is stopped.'; }
        // A lost start response must never permit another start or later personal write.
        if (kind === 'started' || this.blocked) {
          this.closed = true; this.blocked = true; this.detachMedia();
          if (this.heartbeat) clearInterval(this.heartbeat);
          await this.stop('uncertain_outcome');
        }
      }
      this.notify();
    });
  }
  progress(now = Date.now()) {
    if (now - this.lastProgress < 10_000) return;
    this.lastProgress = now;
    void this.event('progress');
  }
  update(track_choices: PlaybackTrackChoices | null, fallback = false) {
    if (this.closed || this.blocked || !this.session?.transport || this.busy || (fallback && this.fallbackUsed)) return Promise.resolve();
    const position_ms = this.snapshotPosition();
    if (fallback) this.fallbackUsed = true;
    this.busy = true;
    this.detachMedia();
    return this.enqueue(async () => {
      try {
        if (this.closed) return;
        this.pendingUpdate = { ...this.authority(), request_id: crypto.randomUUID(), expected_transport_revision: this.session!.transport_revision, position_ms, track_choices, compatible_transcode_fallback: fallback ? true : null };
        const response = await this.client.update_playback_session(this.pendingUpdate);
        await this.accept(response.session, true);
        this.pendingUpdate = null;
      } catch {
        this.status = 'Transport change was not confirmed. Checking session state; no automatic retry.';
        this.blocked = true;
        try {
          const response = await this.client.get_playback_session(this.authority());
          await this.accept(response.session, true);
        } catch { this.detachMedia(); }
      } finally { this.busy = false; this.notify(); }
    });
  }
  async retryUpdate() {
    if (!this.pendingUpdate || this.closed || this.busy || this.blocked) return;
    const body = this.pendingUpdate;
    this.busy = true; this.detachMedia();
    return this.enqueue(async () => {
      try { await this.accept((await this.client.update_playback_session(body)).session, true); this.pendingUpdate = null; }
      catch { this.blocked = true; this.status = 'Transport retry was not confirmed. Check session state.'; }
      finally { this.busy = false; this.notify(); }
    });
  }
  private async stop(reason: string) {
    if (!this.session || this.stopBody) return;
    this.stopBody = { ...this.authority(), request_id: crypto.randomUUID(), final_sequence: Math.min(0xffffffff, ++this.sequence), transport_revision: this.session.transport_revision, position_ms: this.exitPosition ?? this.snapshotPosition(), stop_reason: reason };
    try {
      const response = await this.client.stop_playback_session(this.stopBody);
      this.status = `Playback stopped. Personal final report: ${response.session?.report_outcome ?? 'unknown'}.`;
      if (this.uncertainStart) this.status += ' Initial start outcome is unknown; it was not retried.';
      if (response.session?.report_outcome === 'confirmed') { this.client.invalidate_reads(); this.personalChanged(); }
    } catch {
      // Read-only reconciliation, never replay a final personal write automatically.
      try { await this.client.get_playback_session(this.authority()); } catch {}
      this.status = 'Playback stopped locally. Final personal outcome is unknown.';
    }
    this.notify();
  }
  exit(reason = 'player_exit') {
    if (this.closed) return;
    try { this.snapshotPosition(); } catch {}
    this.exitPosition = this.lastMediaState?.position_ms ?? '0';
    this.exitReason = reason;
    this.closed = true; this.blocked = true;
    this.status = 'Playback stopped locally; final personal report pending.';
    if (this.heartbeat) clearInterval(this.heartbeat);
    this.detachMedia();
    this.client.cancel_all();
    void this.enqueue(() => this.stop(reason));
    this.notify();
  }
}
