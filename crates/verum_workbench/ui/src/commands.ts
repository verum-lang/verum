export type CommandContext = 'workspace' | 'editor' | 'agent';
export type StudioCommand = {
  id: string; label: () => string; shortcut: string; contexts: readonly CommandContext[];
  enabled: () => boolean; run: () => unknown | Promise<unknown>;
  controls?: string;
};
export type KeyInput = {
  code: string; ctrlKey: boolean; metaKey: boolean; altKey: boolean; shiftKey: boolean;
  isComposing: boolean; repeat: boolean; defaultPrevented: boolean;
  getModifierState?: (key: string) => boolean;
};
type Shortcut = { code: string; mod: boolean; ctrl: boolean; meta: boolean; alt: boolean; shift: boolean; text: string };
export class ShortcutError extends Error {
  readonly kind: 'invalid' | 'reserved' | 'conflict' | 'settings';
  readonly command?: StudioCommand;
  constructor(kind: ShortcutError['kind'], command?: StudioCommand) { super(kind); this.kind = kind; this.command = command; }
}

export function parseShortcut(value: string): Shortcut | null {
  if (!value.trim()) return null;
  const parts = value.trim().split('+').map(part => part.trim());
  const key = parts.pop()!;
  if (new Set(parts).size !== parts.length || parts.some(part => !['Mod', 'Ctrl', 'Meta', 'Alt', 'Shift'].includes(part))) throw new ShortcutError('invalid');
  const mod = parts.includes('Mod'), ctrl = parts.includes('Ctrl'), meta = parts.includes('Meta'), alt = parts.includes('Alt'), shift = parts.includes('Shift');
  if ((mod && (ctrl || meta)) || (ctrl && meta)) throw new ShortcutError('invalid');
  const code = /^[a-z]$/i.test(key) ? `Key${key.toUpperCase()}` : /^\d$/.test(key) ? `Digit${key}` : key;
  if (!/^(Key[A-Z]|Digit\d|F([1-9]|1\d|2[0-4])|Enter|Space|Slash|Comma|Period|Backspace|Delete|Arrow(Up|Down|Left|Right))$/.test(code)) throw new ShortcutError('invalid');
  if (!mod && !ctrl && !meta && !alt && !/^F\d+$/.test(code)) throw new ShortcutError('invalid');
  // Preserve browser/OS navigation, window management, reload and AltGraph input.
  if (((mod || ctrl) && alt) || (alt && ['F4', 'Space', 'ArrowLeft', 'ArrowRight'].includes(code)) || ((mod || ctrl || meta) && ['KeyL', 'KeyW', 'KeyT', 'KeyN', 'KeyR', 'KeyQ'].includes(code))) throw new ShortcutError('reserved');
  const display = code.startsWith('Key') ? code.slice(3) : code.startsWith('Digit') ? code.slice(5) : code;
  return { code, mod, ctrl, meta, alt, shift, text: [...(mod ? ['Mod'] : []), ...(ctrl ? ['Ctrl'] : []), ...(meta ? ['Meta'] : []), ...(alt ? ['Alt'] : []), ...(shift ? ['Shift'] : []), display].join('+') };
}
const signature = (key: Shortcut, mac: boolean) => [key.code, key.ctrl || (key.mod && !mac), key.meta || (key.mod && mac), key.alt, key.shift].join(':');

export class CommandRegistry {
  private overrides = new Map<string, string>();
  readonly commands: readonly StudioCommand[];
  constructor(commands: readonly StudioCommand[]) {
    this.commands = commands;
    if (new Set(commands.map(command => command.id)).size !== commands.length) throw new Error('Duplicate command identity');
    this.validate(this.overrides);
  }
  shortcut(command: StudioCommand): string { return this.overrides.get(command.id) ?? command.shortcut; }
  private validate(overrides: ReadonlyMap<string, string>) {
    const seen: Array<{ command: StudioCommand; shortcut: Shortcut }> = [];
    for (const command of this.commands) {
      const shortcut = parseShortcut(overrides.get(command.id) ?? command.shortcut);
      if (!shortcut) continue;
      const conflict = seen.find(other => [true, false].some(mac => signature(other.shortcut, mac) === signature(shortcut, mac)) && other.command.contexts.some(context => command.contexts.includes(context)));
      if (conflict) throw new ShortcutError('conflict', conflict.command);
      seen.push({ command, shortcut });
    }
  }
  set(id: string, value: string | null) {
    if (!this.commands.some(command => command.id === id)) throw new Error('Unknown command');
    const next = new Map(this.overrides);
    if (value === null) next.delete(id); else next.set(id, parseShortcut(value)?.text ?? '');
    this.validate(next); this.overrides = next;
  }
  reset() { this.overrides.clear(); }
  serialize(): string { return JSON.stringify({ version: 1, bindings: Object.fromEntries(this.overrides) }); }
  restore(raw: string) {
    const value: unknown = JSON.parse(raw);
    if (!value || typeof value !== 'object' || Array.isArray(value)) throw new ShortcutError('settings');
    const data = value as { version?: unknown; bindings?: unknown };
    if (data.version !== 1 || !data.bindings || typeof data.bindings !== 'object' || Array.isArray(data.bindings)) throw new ShortcutError('settings');
    const next = new Map<string, string>();
    for (const [id, binding] of Object.entries(data.bindings)) {
      if (!this.commands.some(command => command.id === id) || typeof binding !== 'string') throw new ShortcutError('settings');
      next.set(id, parseShortcut(binding)?.text ?? '');
    }
    this.validate(next); this.overrides = next;
  }
  private find(event: KeyInput, context: CommandContext, mac: boolean, defaults: boolean): StudioCommand | undefined {
    if (event.defaultPrevented || event.isComposing || event.getModifierState?.('AltGraph')) return;
    return this.commands.find(command => {
      const shortcut = parseShortcut(defaults ? command.shortcut : this.shortcut(command));
      return shortcut && command.contexts.includes(context)
        && shortcut.code === event.code && shortcut.alt === event.altKey && shortcut.shift === event.shiftKey
        && event.ctrlKey === (shortcut.ctrl || (shortcut.mod && !mac)) && event.metaKey === (shortcut.meta || (shortcut.mod && mac));
    });
  }
  match(event: KeyInput, context: CommandContext, mac: boolean) { return this.find(event, context, mac, false); }
  reserves(event: KeyInput, context: CommandContext, mac: boolean) { return !!this.find(event, context, mac, true); }
}

export function displayShortcut(value: string, mac: boolean): string {
  return value.replace(/\bMod\b/g, mac ? '⌘' : 'Ctrl').replace(/\bAlt\b/g, mac ? '⌥' : 'Alt').replace(/\bShift\b/g, mac ? '⇧' : 'Shift').replace(/\bSlash\b/g, '/').replace(/\bComma\b/g, ',').replace(/\bPeriod\b/g, '.');
}

export function shortcutAttribute(value: string, mac: boolean): string {
  return value.replace(/\bMod\b/g, mac ? 'Meta' : 'Control').replace(/\bCtrl\b/g, 'Control').replace(/\bSlash\b/g, '/').replace(/\bComma\b/g, ',').replace(/\bPeriod\b/g, '.');
}
