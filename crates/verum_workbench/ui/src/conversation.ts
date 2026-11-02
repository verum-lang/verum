// Every provider operation captures this object before its first await. The
// selected conversation controls presentation only, never protocol routing.
export class Conversation {
  readonly log = document.createElement('div');
  readonly option = document.createElement('option');
  readonly messageBodies = new Map<string, HTMLElement>();
  readonly reviews = new Map<string, () => void>();
  readonly completed = new Set<string>();
  readonly events: string[] = [];
  eventLength = 0;
  truncated = false;
  cursor = 0;
  thread: string | null = null;
  turn: string | null = null;
  sending = false;
  role = 'author';
  model = '';
  draft = '';
  title = '';
  scroll = 0;

  get busy() { return this.sending || this.turn !== null; }

  start(id: string) {
    if (!this.completed.has(id)) this.turn = id;
  }

  finish(id: string) {
    this.completed.add(id);
    if (this.turn === id) this.turn = null;
  }

  record(payload: string) {
    const excerpt = payload.length > 4096 ? payload.slice(0, 4096) + '\n[Event payload excerpt]' : payload;
    this.truncated ||= payload.length > 4096;
    this.events.push(excerpt);
    this.eventLength += excerpt.length;
    while (this.events.length > 150 || this.eventLength > 65536) {
      this.eventLength -= this.events.shift()!.length;
      this.truncated = true;
    }
  }
}
