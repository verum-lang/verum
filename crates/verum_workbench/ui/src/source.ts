import { positionAt, type Range } from './language.ts';

/** Project editor changes onto the original separators, including mixed-EOL files. */
export function editFileText(source: string, changes: { from: number; to: number; insert: string }[]): string {
  const newline = source.match(/\r\n|\r|\n/)?.[0] ?? '\n';
  let raw = 0, editor = 0, retained = 0;
  const segments: string[] = [];
  function advance(to: number) {
    while (editor < to) {
      if (source[raw] === '\r' && source[raw + 1] === '\n') raw++;
      raw++; editor++;
    }
    return raw;
  }
  for (const change of changes) {
    segments.push(source.slice(retained, advance(change.from)), change.insert.replaceAll('\n', newline));
    retained = advance(change.to);
  }
  segments.push(source.slice(retained));
  return segments.join('');
}

export async function sourceHash(source: string): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(source));
  return [...new Uint8Array(digest)].map(byte => byte.toString(16).padStart(2, '0')).join('');
}

/** Translate exact UTF-8 byte boundaries to editor/LSP UTF-16 positions. */
export function sourceByteRange(source: string, start: unknown, end: unknown): { range: Range; text: string } | null {
  if (typeof start !== 'number' || typeof end !== 'number' || !Number.isSafeInteger(start)
    || !Number.isSafeInteger(end) || start < 0 || end < start) return null;
  const bytes = new TextEncoder().encode(source);
  if (end > bytes.length) return null;
  try {
    const decode = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true });
    if (decode.decode(bytes) !== source) return null;
    const before = decode.decode(bytes.slice(0, start));
    const through = decode.decode(bytes.slice(0, end));
    // An LSP position cannot address the middle of a CRLF sequence.
    for (const prefix of [before, through]) if (prefix.endsWith('\r') && source[prefix.length] === '\n') return null;
    return { range: { start: positionAt(source, before.length), end: positionAt(source, through.length) },
      text: source.slice(before.length, through.length) };
  } catch { return null; }
}

/** Hash once per editor revision; release captured text as soon as hashing starts. */
export function sourceRevision(source: string): { digest: () => Promise<string> } {
  let text: string | null = source, pending: Promise<string> | null = null;
  return { digest() {
    if (!pending) { pending = sourceHash(text!); text = null; }
    return pending;
  } };
}
