import test from 'node:test';
import assert from 'node:assert/strict';
import { cbgrObservationState, runtimeSourceRange, type CbgrEvent } from '../src/runtime.ts';
import type { Receipt } from '../src/model.ts';

const event: CbgrEvent = {
  kind: 'GenerationMismatch', target_kind: 'register', target: '7', expected: '18446744073709551615', actual: '0',
  function_id: 0, function: 'main', pc: 18, opcode: 20,
  source: { path: '/tmp/entry.vr', line: 2, column: 3 },
};
const receipt: Receipt = {
  id: 'run-one', kind: 'run', path: 'experiment.vr', source_hash: 'hash-one', status: 'failed',
  stdout: '', stderr: '', elapsed_ms: 1, exit_code: 1, runtime_diagnostics: true,
  runtime: {
    schema: 'verum.runtime-report/1', run_id: 'run-one', source_sha256: 'hash-one',
    prepared_source_sha256: 'wrapped-hash', wrapper_lines: 1, entry_source_path: '/tmp/entry.vr',
    compiler_version: '0.1.0', observation: 'captured',
    cbgr: { schema: 'verum.cbgr.dispatch/1', capacity: 256, checks_enabled_at_start: true, truncated: false, events: [event] },
  },
};
const range = (record = receipt, observed = event, path = 'experiment.vr', hash: string | null = 'hash-one') =>
  runtimeSourceRange(record, observed, path, hash, 'let λ = 1;\r\nprint(λ);');

test('omitted failure records never display an observed clean capture', () => {
  const report = structuredClone(receipt.runtime!);
  assert.equal(cbgrObservationState(report), 'retained');
  report.cbgr!.events = [];
  assert.equal(cbgrObservationState(report), 'empty');
  report.cbgr!.truncated = true;
  assert.equal(cbgrObservationState(report), 'omitted');
  report.cbgr!.truncated = false;
  report.schema = 'verum.runtime-report/2';
  report.export_loss = { cbgr_events: 256, method_sites: 2, task_rows: 3 };
  assert.equal(cbgrObservationState(report), 'omitted');
  report.export_loss.cbgr_events = 0;
  assert.equal(cbgrObservationState(report), 'empty');
  report.observation = 'unavailable';
  assert.equal(cbgrObservationState(report), 'unavailable');
});

test('exact runtime source navigation accounts for the script wrapper without guessing column units', () => {
  assert.deepEqual(range(), { start: { line: 0, character: 0 }, end: { line: 0, character: 10 } });
  const unwrapped = structuredClone(receipt); unwrapped.runtime!.wrapper_lines = 0;
  assert.deepEqual(range(unwrapped), { start: { line: 1, character: 0 }, end: { line: 1, character: 9 } });
});

test('stale, mismatched, cached and unmapped runtime evidence cannot select current source', () => {
  for (const [path, hash] of [['other.vr', 'hash-one'], ['experiment.vr', 'hash-two'], ['experiment.vr', null]]) {
    assert.equal(range(receipt, event, path!, hash), null);
  }
  for (const change of [
    { run_id: 'earlier-run' }, { source_sha256: 'other-source' }, { entry_source_path: null },
    { wrapper_lines: 2 }, { wrapper_lines: -1 }, { observation: 'unavailable' as const },
  ]) {
    const record = structuredClone(receipt); Object.assign(record.runtime!, change);
    assert.equal(range(record), null);
  }
  for (const source of [null, { ...event.source!, path: 'dependency.vr' }, { ...event.source!, line: 0 },
    { ...event.source!, line: 1 }, { ...event.source!, line: 4 }, { ...event.source!, line: 1.5 }, { ...event.source!, column: 0 }]) {
    assert.equal(range(receipt, { ...event, source }), null);
  }
});

test('method site ranking preserves 64-bit nanosecond measurements and receipt order', async () => {
  const { rankMethodSites } = await import('../src/runtime.ts');
  const make = (pc: number, inclusive_ns: string) => ({
    module_instance: 0, module: 'study', function_id: 1, function: 'run', pc,
    selector_id: 4, selector: 'measure', outcome: { kind: 'completed_inline' as const },
    count: '1', inclusive_ns, max_ns: inclusive_ns,
  });
  const rows = [make(4, '9007199254740992'), make(9, '9007199254740993')];
  const profile = { schema: 'verum.method-dispatch/1' as const, capacity: 2, dropped_calls: '0', rows };
  assert.deepEqual(rankMethodSites(profile).map(row => row.pc), [9, 4]);
  assert.deepEqual(rows.map(row => row.pc), [4, 9]);
});

test('task summaries count only retained actual statuses and preserve full task IDs', async () => {
  const { taskSnapshotCounts } = await import('../src/runtime.ts');
  const rows = (['pending', 'running', 'suspended', 'completed', 'failed'] as const).map((status, index) => ({
    task_id: (9007199254740992n + BigInt(index)).toString(), function_id: 7, module: 'app',
    function: 'worker', entered: true, status, error: status === 'failed' ? 'original failure' : null,
    error_truncated: false,
  }));
  const report = { schema: 'verum.async-tasks/1' as const, capacity: 5, omitted_tasks: '18446744073709551615', rows };
  assert.deepEqual(taskSnapshotCounts(report), { pending: 1, running: 1, suspended: 1, completed: 1, failed: 1 });
  assert.equal(report.rows[4].task_id, '9007199254740996');
  assert.equal(report.omitted_tasks, '18446744073709551615');
});
