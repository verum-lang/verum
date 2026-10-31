export type SessionState = 'locked' | 'checking' | 'connected';
type Send = (token: string, method: string, params: Record<string, unknown>, signal?: AbortSignal) => Promise<unknown>;

export class SessionAccessError extends Error {}
export class SessionChangedError extends Error {}

/** Client connection state. Every server operation still authenticates independently. */
export class StudioSession {
  private token = '';
  private generation = 0;
  private current: SessionState = 'locked';
  private readonly send: Send;
  private readonly changed: (state: SessionState) => void;
  constructor(send: Send, changed: (state: SessionState) => void) { this.send = send; this.changed = changed; }
  get state() { return this.current; }

  private transition(state: SessionState) { this.current = state; this.changed(state); }
  private lock() { this.generation++; this.token = ''; this.transition('locked'); }

  async connect(candidate: string) {
    const token = candidate.trim();
    if (!token) { this.lock(); throw new SessionAccessError(); }
    const generation = ++this.generation;
    this.token = ''; this.transition('checking');
    try {
      // A cheap, read-only request to the same authenticated endpoint used by
      // all workspace tools. Merely possessing a string never grants access.
      await this.send(token, 'operation.list', { known: {} }, AbortSignal.timeout(10_000));
    } catch (error) {
      if (generation !== this.generation) throw new SessionChangedError();
      this.lock(); throw error;
    }
    if (generation !== this.generation) throw new SessionChangedError();
    this.token = token; this.transition('connected');
  }

  async call<T>(method: string, params: Record<string, unknown>): Promise<T> {
    if (this.state !== 'connected') throw new SessionAccessError();
    const generation = this.generation;
    try {
      const result = await this.send(this.token, method, params);
      if (generation !== this.generation) throw new SessionChangedError();
      return result as T;
    } catch (error) {
      if (generation !== this.generation) throw new SessionChangedError();
      if (error instanceof SessionAccessError) this.lock();
      throw error;
    }
  }
}
