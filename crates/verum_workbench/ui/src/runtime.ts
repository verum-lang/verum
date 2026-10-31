import type { Receipt } from './model';

export type CbgrEvent = {
  kind: string; target_kind: 'register' | 'address'; target: string;
  expected: string | null; actual: string | null;
  function_id: number | null; function: string | null; pc: number; opcode: number;
  source: { path: string; line: number; column: number } | null;
};
export type RuntimeReport = {
  schema: 'verum.runtime-report/1'; run_id: string; source_sha256: string;
  prepared_source_sha256: string; wrapper_lines: number; entry_source_path: string | null;
  compiler_version: string; observation: 'captured' | 'unavailable';
  cbgr: { schema: 'verum.cbgr.dispatch/1'; capacity: number; checks_enabled_at_start: boolean; truncated: boolean; events: CbgrEvent[] } | null;
};

// Navigate only an exact entry-source mapping for the current editor revision.
// Dependency paths and cached bytecode without source identity remain inspectable
// as evidence, but must never be guessed into the current editor buffer.
export function runtimeSourceRange(receipt: Receipt, event: CbgrEvent, path: string, hash: string | null, source: string) {
  const report = receipt.runtime, location = event.source;
  if (!report || !location || report.observation !== 'captured'
    || report.run_id !== receipt.id || report.source_sha256 !== receipt.source_hash
    || path !== receipt.path || hash !== receipt.source_hash
    || !report.entry_source_path || location.path !== report.entry_source_path
    || !Number.isInteger(report.wrapper_lines) || report.wrapper_lines < 0 || report.wrapper_lines > 1
    || !Number.isInteger(location.line) || !Number.isInteger(location.column) || location.column < 1) return null;
  const line = location.line - report.wrapper_lines - 1;
  const lines = source.split('\n');
  if (line < 0 || line >= lines.length) return null;
  // Source-map column units are not specified as LSP UTF-16. Select the exact
  // line instead of silently treating byte/scalar columns as editor offsets.
  return { start: { line, character: 0 }, end: { line, character: lines[line].replace(/\r$/, '').length } };
}
