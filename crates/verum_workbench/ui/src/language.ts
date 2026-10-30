export type Position = { line: number; character: number };
export type Range = { start: Position; end: Position };
export type Diagnostic = { range: Range; severity?: number; message: string; source?: string; code?: string | number };
export type Declaration = { id: string; name: string; kind: string; type_display?: string | null; range: Range; selection_range: Range; requirements: Array<{ source: string; range: Range }> };
export type Binding = { name: string; type_display: string; definition: { uri: string; range: Range } };
export type Analysis = { project_revision?: string | null; bindings?: Binding[]; source_hash: string; version: number; parsed: boolean; analysis_scope: string; diagnostics: Diagnostic[]; declarations: Declaration[]; mounts: Array<{ source: string; range: Range }> };
export type DocumentRevision = { path: string; source: string };
export type LanguageReply<T> = { source_hash: string; version: number; result: T };
export const sameRevision = (left: DocumentRevision, right: DocumentRevision) => left.path === right.path && left.source === right.source;

// JavaScript and LSP both count UTF-16 code units. Never convert these columns to bytes.
export function positionAt(source: string, offset: number): Position {
  const prefix = source.slice(0, offset), lines = prefix.split(/\r\n|\r|\n/);
  return { line: lines.length - 1, character: lines[lines.length - 1].length };
}
export function offsetAt(source: string, position: Position): number | null {
  if (!Number.isInteger(position.line) || !Number.isInteger(position.character) || position.line < 0 || position.character < 0) return null;
  const lines = source.split(/\r\n|\r|\n/);
  if (position.line >= lines.length) return null;
  let offset = 0, line = 0;
  for (const match of source.matchAll(/\r\n|\r|\n/g)) { if (line++ === position.line) break; offset = match.index! + match[0].length; }
  const column = Math.min(position.character, lines[position.line].length);
  const before = lines[position.line].charCodeAt(column - 1), after = lines[position.line].charCodeAt(column);
  if (before >= 0xd800 && before <= 0xdbff && after >= 0xdc00 && after <= 0xdfff) return null;
  return offset + column;
}
export function rangeOffsets(source: string, range: Range): { from: number; to: number } | null {
  const from = offsetAt(source, range.start), to = offsetAt(source, range.end);
  return from !== null && to !== null && from <= to ? { from, to } : null;
}
export async function languageRequest<T>(revision: DocumentRevision, method = 'analyze', position?: Position) {
  const { api } = await import('./api');
  return api<LanguageReply<T>>('language.request', { ...revision, method, ...(position ? { position } : {}) });
}
