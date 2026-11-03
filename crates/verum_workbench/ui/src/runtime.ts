import type { Receipt } from './model';

export type CbgrEvent = {
  kind: string; target_kind: 'register' | 'address'; target: string;
  expected: string | null; actual: string | null;
  function_id: number | null; function: string | null; pc: number; opcode: number;
  source: { path: string; line: number; column: number } | null;
};
export type MethodOutcome = { kind: 'entered_frame'; function_id: number }
  | { kind: 'completed_inline' | 'retried' | 'returned' | 'yielded' | 'failed' };
export type MethodDispatchRow = {
  module_instance: number; module: string | null; function_id: number; function: string | null;
  pc: number; selector_id: number; selector: string | null; outcome: MethodOutcome;
  count: string; inclusive_ns: string; max_ns: string;
};
export type MethodDispatchReport = {
  schema: 'verum.method-dispatch/1'; capacity: number; dropped_calls: string; rows: MethodDispatchRow[];
};

export type AsyncTaskStatus = 'pending' | 'running' | 'suspended' | 'completed' | 'failed';
export type AsyncTaskReport = {
  schema: 'verum.async-tasks/1'; capacity: number; omitted_tasks: string;
  rows: { task_id: string; function_id: number | null; module: string | null; function: string | null;
    entered: boolean; status: AsyncTaskStatus; error: string | null; error_truncated: boolean }[];
};

export function taskSnapshotCounts(report: AsyncTaskReport) {
  const counts: Record<AsyncTaskStatus, number> = { pending: 0, running: 0, suspended: 0, completed: 0, failed: 0 };
  for (const row of report.rows) counts[row.status] += 1;
  return counts;
}

export function rankMethodSites(report: MethodDispatchReport) {
  return [...report.rows].sort((a, b) => {
    const left = BigInt(a.inclusive_ns), right = BigInt(b.inclusive_ns);
    return left > right ? -1 : left < right ? 1 : 0;
  });
}

export type RuntimeReport = {
  schema: 'verum.runtime-report/1'; run_id: string; source_sha256: string;
  prepared_source_sha256: string; wrapper_lines: number; entry_source_path: string | null;
  compiler_version: string; observation: 'captured' | 'unavailable';
  method_dispatch?: MethodDispatchReport | null;
  async_tasks?: AsyncTaskReport | null;
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
