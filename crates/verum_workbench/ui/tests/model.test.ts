import test from 'node:test';
import assert from 'node:assert/strict';
import { sameDocumentRevision, isStale, mergeReceipts, knownReceiptVersions, safeAuthUrl, relativeVerumPath, approvalKind, approvalReply, parseArguments, type Receipt } from '../src/model.ts';

const receipt: Receipt = { id: 'one', kind: 'proof', path: 'identity.vr', source_hash: 'revision-a', status: 'succeeded', stdout: '', stderr: '', elapsed_ms: 4, exit_code: 0 };

test('startup restore is revoked by typing or opening another file during an async read', async () => {
  const requested = { path: 'experiment.vr', editVersion: 0 };
  assert.equal(sameDocumentRevision(requested, { ...requested }), true);
  assert.equal(sameDocumentRevision(requested, { ...requested, editVersion: 1 }), false);
  assert.equal(sameDocumentRevision(requested, { path: 'identity.vr', editVersion: 0 }), false);
  // Even typing and undoing to the same source advances the edit version.
  assert.equal(sameDocumentRevision(requested, { ...requested, editVersion: 2 }), false);
});

test('results never describe another file, edited source, or a pending hash', () => {
  assert.equal(isStale(receipt, 'identity.vr', 'revision-a'), false);
  assert.equal(isStale(receipt, 'identity.vr', 'revision-b'), true);
  assert.equal(isStale(receipt, 'other.vr', 'revision-a'), true);
  assert.equal(isStale(receipt, 'identity.vr', null), true);
});

test('sign-in links permit HTTPS, never executable URLs or embedded credentials', () => {
  assert.equal(safeAuthUrl('https://auth.openai.com/authorize?state=example'), 'https://auth.openai.com/authorize?state=example');
  for (const value of ['javascript:alert(1)', 'data:text/html,test', 'http://example.com', 'https://user:password@example.com', '/relative', null, {}]) assert.equal(safeAuthUrl(value), null);
});

test('source creation only accepts relative Verum paths without traversal', () => {
  assert.equal(relativeVerumPath(' experiment.vr '), 'experiment.vr');
  assert.equal(relativeVerumPath('examples/identity.vr'), 'examples/identity.vr');
  for (const value of ['/tmp/file.vr', '../file.vr', 'a/../file.vr', 'a//file.vr', 'a/./file.vr', '.private/file.vr', '.secret.vr', 'a\\file.vr', 'file.py', 'file.vr\u0000']) assert.equal(relativeVerumPath(value), null);
});

test('incremental receipts retain unchanged history and reject older revisions', () => {
  const previous = [{ ...receipt, version: 3, started_unix_ms: 100 }, { ...receipt, id: 'two', version: 1, started_unix_ms: 200, status: 'running' as const }];
  const next = mergeReceipts(previous, [{ ...previous[1], version: 2, status: 'succeeded', stdout: '42\n' }]);
  assert.equal(next.length, 2);
  assert.deepEqual(next[0], previous[0]);
  assert.equal(next[1].stdout, '42\n');
  assert.deepEqual(mergeReceipts(next, [previous[1]]), next);
  assert.deepEqual(mergeReceipts(next, []), next);
  assert.deepEqual(knownReceiptVersions(next), { one: 3, two: 2 });
  assert.equal(knownReceiptVersions([receipt]), null);
});

test('history cap retains the newest 128 runs by their actual start times', () => {
  const items = Array.from({ length: 130 }, (_, index) => ({ ...receipt, id: String(index), started_unix_ms: index }));
  const merged = mergeReceipts([], items.reverse());
  assert.equal(merged.length, 128);
  assert.equal(merged[0].id, '2');
  assert.equal(merged[127].id, '129');
});

test('only declared Verum tools offer approval; shell/file bypasses refuse', () => {
  assert.equal(approvalKind('item/tool/call', { tool: 'verum_propose' }), 'source');
  assert.equal(approvalKind('item/tool/call', { tool: 'verum_run' }), 'execution');
  assert.equal(approvalKind('item/tool/call', { tool: 'verum_read' }), 'read');
  assert.equal(approvalKind('item/tool/call', { tool: 'run_shell' }), 'unsupported');
  assert.equal(approvalKind('item/commandExecution/requestApproval', {}), 'unsupported');
  assert.equal(approvalKind('item/fileChange/requestApproval', {}), 'unsupported');
  assert.deepEqual(approvalReply('item/tool/call', true), { approved: true });
  assert.deepEqual(approvalReply('item/tool/call', false), { approved: false });
  assert.deepEqual(approvalReply('item/commandExecution/requestApproval', false), { decision: 'decline' });
});

test('tool arguments support both wire object and JSON string forms without evaluating content', () => {
  assert.deepEqual(parseArguments('{"source":"<script>alert(1)</script>"}'), { source: '<script>alert(1)</script>' });
  assert.deepEqual(parseArguments({ path: 'file.vr' }), { path: 'file.vr' });
  for (const value of [null, false, 'not JSON', '[1,2]', '"scalar"']) assert.deepEqual(parseArguments(value), {});
});
