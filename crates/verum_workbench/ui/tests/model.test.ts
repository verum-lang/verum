import test from 'node:test';
import assert from 'node:assert/strict';
import { sameDocumentRevision, replaceDocumentRevision, isStale, mergeReceipts, knownReceiptVersions, safeAuthUrl, relativeVerumPath, approvalKind, approvalReply, parseArguments, type Receipt } from '../src/model.ts';

test('delayed document responses cannot replace intervening edits or a reopened document', async () => {
  for (const after of [{ path: 'a.vr', editVersion: 2 }, { path: 'b.vr', editVersion: 1 }, { path: 'a.vr', editVersion: 3 }]) {
    let current = { path: 'a.vr', editVersion: 1 }, draft = 'initial';
    let deliver!: (value: string) => void;
    const pending = replaceDocumentRevision(current, () => current,
      () => new Promise<string>(resolve => { deliver = resolve; }), source => { draft = source; });
    current = after; draft = 'new unsaved draft';
    deliver('old response');
    assert.equal(await pending, 'stale');
    assert.equal(draft, 'new unsaved draft');
  }
});

test('document replacement distinguishes consent, current results and failed reads', async () => {
  const revision = { path: 'a.vr', editVersion: 1 };
  const applied: string[] = [];
  assert.equal(await replaceDocumentRevision(revision, () => revision, async () => 'new', value => applied.push(value)), 'applied');
  assert.equal(await replaceDocumentRevision(revision, () => revision, async () => null, value => applied.push(value)), 'cancelled');
  await assert.rejects(replaceDocumentRevision(revision, () => revision, async () => { throw new Error('read failed'); }, value => applied.push(value)), /read failed/);
  assert.equal(await replaceDocumentRevision(revision, () => ({ ...revision, editVersion: 2 }),
    async () => { assert.fail('stale consent must not fetch'); }, value => applied.push(value)), 'stale');
  assert.deepEqual(applied, ['new']);
});

const receipt: Receipt = { id: 'one', kind: 'proof', path: 'identity.vr', source_mode: 'source', source_hash: 'revision-a', status: 'succeeded', stdout: '', stderr: '', elapsed_ms: 4, exit_code: 0 };

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
  assert.equal(isStale(receipt, 'identity.vr', 'revision-a', 'script'), true);
  assert.equal(isStale({ ...receipt, source_mode: undefined }, 'identity.vr', 'revision-a'), true);
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

test('the review UI uses only the host classification of a dynamic tool call', () => {
  for (const kind of ['source', 'execution', 'read']) assert.equal(approvalKind('item/tool/call', kind), kind);
  assert.equal(approvalKind('item/tool/call', undefined), 'unsupported');
  assert.equal(approvalKind('item/tool/call', { tool: 'verum_read', reviewKind: 'read' }), 'unsupported');
  assert.equal(approvalKind('item/commandExecution/requestApproval', 'execution'), 'unsupported');
  assert.equal(approvalKind('item/fileChange/requestApproval', 'source'), 'unsupported');
  assert.deepEqual(approvalReply('item/tool/call', true), { approved: true });
  assert.deepEqual(approvalReply('item/tool/call', false), { approved: false });
  assert.deepEqual(approvalReply('item/commandExecution/requestApproval', false), { decision: 'decline' });
});

test('tool arguments support both wire object and JSON string forms without evaluating content', () => {
  assert.deepEqual(parseArguments('{"source":"<script>alert(1)</script>"}'), { source: '<script>alert(1)</script>' });
  assert.deepEqual(parseArguments({ path: 'file.vr' }), { path: 'file.vr' });
  for (const value of [null, false, 'not JSON', '[1,2]', '"scalar"']) assert.deepEqual(parseArguments(value), {});
});
