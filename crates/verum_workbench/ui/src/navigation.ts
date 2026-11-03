/** Locations share a revision digest, never a retained source copy per history entry. */
export type SourceRevision = { digest: () => Promise<string> };
export type SourceBookmark = { path: string; revision: SourceRevision; anchor: number; head: number };
export type NavigationStep = { readonly generation: number; readonly index: number; readonly origin: SourceBookmark; readonly target: SourceBookmark };
const copy = (location: SourceBookmark): SourceBookmark => ({ ...location });
const same = (left: SourceBookmark, right: SourceBookmark) => left.path === right.path && left.revision === right.revision && left.anchor === right.anchor && left.head === right.head;

/** Bounded session history. Preparing a traversal never consumes its entry. */
export class NavigationHistory {
  private entries: SourceBookmark[] = [];
  private index = -1;
  private generation = 0;
  private readonly capacity: number;
  constructor(capacity = 100) {
    this.capacity = capacity;
    if (!Number.isInteger(capacity) || capacity < 2) throw new Error('Navigation capacity must be at least two');
  }
  get back() { return this.index > 0; }
  get forward() { return this.index >= 0 && this.index + 1 < this.entries.length; }
  record(origin: SourceBookmark, destination: SourceBookmark) {
    if (same(origin, destination)) return;
    if (this.index < 0) { this.entries = [copy(origin)]; this.index = 0; }
    else this.entries[this.index] = copy(origin);
    this.entries.splice(this.index + 1);
    this.entries.push(copy(destination)); this.index++;
    if (this.entries.length > this.capacity) { this.entries.shift(); this.index--; }
    this.generation++;
  }
  /** Refine a successful file arrival after its semantic location is validated. */
  reviseCurrent(expected: SourceBookmark, destination: SourceBookmark): boolean {
    if (this.index < 0 || !same(this.entries[this.index], expected) || destination.path !== expected.path) return false;
    this.entries[this.index] = copy(destination); this.generation++; return true;
  }
  plan(direction: -1 | 1, origin: SourceBookmark): NavigationStep | null {
    const index = this.index + direction;
    if (index < 0 || index >= this.entries.length) return null;
    return { generation: this.generation, index, origin: copy(origin), target: copy(this.entries[index]) };
  }
  complete(step: NavigationStep, destination: SourceBookmark): boolean {
    if (step.generation !== this.generation || Math.abs(step.index - this.index) !== 1 || destination.path !== step.target.path) return false;
    this.entries[this.index] = copy(step.origin);
    this.index = step.index; this.entries[this.index] = copy(destination); this.generation++;
    return true;
  }
  /** Only edits to a matching revision can move an existing source location. */
  mapEdits(path: string, before: SourceRevision, revision: SourceRevision, map: (position: number, association: -1 | 1) => number) {
    let changed = false;
    for (const entry of this.entries) {
      if (entry.path !== path || entry.revision !== before) continue;
      const forward = entry.anchor < entry.head, cursor = entry.anchor === entry.head;
      entry.anchor = map(entry.anchor, forward ? -1 : 1);
      entry.head = map(entry.head, cursor || forward ? 1 : -1);
      entry.revision = revision; changed = true;
    }
    if (changed) this.generation++;
  }
  clear() { this.entries = []; this.index = -1; this.generation++; }
}
