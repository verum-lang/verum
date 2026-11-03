import test from 'node:test';
import assert from 'node:assert/strict';
import { ChangeSet } from '@codemirror/state';
import { NavigationHistory, NavigationRequests, type SourceBookmark } from '../src/navigation.ts';
import { sourceRevision } from '../src/source.ts';
import { replaceDocumentRevision } from '../src/model.ts';
const bookmark = (path: string, anchor = 0, head = anchor): SourceBookmark => ({ path, anchor, head, revision: sourceRevision('first\nsecond\nthird\n') });

test('back/forward restore distinct file locations and the cursor last left there', () => {
  const history = new NavigationHistory(), a = bookmark('a.vr', 2, 4), b = bookmark('b.vr', 8);
  history.record(a, b);
  const back = history.plan(-1, { ...b, anchor: 12, head: 12 })!;
  assert.deepEqual(back.target, a); assert.equal(history.back, true); assert.equal(history.forward, false);
  assert.equal(history.complete(back, a), true);
  const forward = history.plan(1, a)!;
  assert.equal(forward.target.path, 'b.vr'); assert.equal(forward.target.head, 12);
  assert.equal(history.complete(forward, forward.target), true);
  assert.equal(history.forward, false);
});

test('cancelled or failed file loading does not consume a history destination', async () => {
  const history = new NavigationHistory(), a = bookmark('a.vr'), b = bookmark('b.vr'); history.record(a, b);
  const step = history.plan(-1, b)!;
  const revision = { path: 'b.vr', editVersion: 1 };
  assert.equal(await replaceDocumentRevision(revision, () => revision, async () => null, () => assert.fail('cancelled load applied')), 'cancelled');
  await assert.rejects(replaceDocumentRevision(revision, () => revision, async () => { throw Error('deleted'); }, () => assert.fail('failed load applied')), /deleted/);
  assert.deepEqual(history.plan(-1, b), step);
  assert.equal(history.complete(step, bookmark('other.vr')), false);
  assert.equal(history.complete(step, a), true);
});

test('typing while a destination loads preserves the draft and the traversal', async () => {
  const history = new NavigationHistory(), a = bookmark('a.vr'), b = bookmark('b.vr'); history.record(a, b);
  const revision = { path: 'b.vr', editVersion: 1 }; let current = revision;
  let resolve!: (value: string) => void;
  const pending = replaceDocumentRevision(revision, () => current, () => new Promise<string>(done => { resolve = done; }), () => assert.fail('newer draft was replaced'));
  current = { ...revision, editVersion: 2 }; resolve('saved a.vr');
  assert.equal(await pending, 'stale');
  assert.equal(history.plan(-1, b)!.target.path, 'a.vr');
});

test('new navigation truncates forward history and invalidates an older pending traversal', () => {
  const history = new NavigationHistory(), a = bookmark('a.vr'), b = bookmark('b.vr'), c = bookmark('c.vr');
  history.record(a, b); const back = history.plan(-1, b)!;
  history.complete(back, a); const forward = history.plan(1, a)!;
  history.record(a, c);
  assert.equal(history.forward, false); assert.equal(history.complete(forward, b), false);
  assert.equal(history.plan(-1, c)!.target.path, 'a.vr');
});

test('edits remap matching selections, including cursors, without adopting another revision', () => {
  const history = new NavigationHistory(), a = bookmark('a.vr', 6), b = { ...a, anchor: 7, head: 9 };
  history.record(a, b); const stale = history.plan(-1, b)!;
  const edits = ChangeSet.of({ from: 0, insert: '🧠\n' }, 19), revision = sourceRevision('🧠\nfirst\nsecond\nthird\n');
  history.mapEdits('a.vr', a.revision, revision, (position, association) => edits.mapPos(position, association));
  assert.equal(history.complete(stale, a), false);
  const back = history.plan(-1, { ...b, revision, anchor: 10, head: 12 })!;
  assert.equal(back.target.anchor, 9); assert.equal(back.target.head, 9); assert.equal(back.target.revision, revision);
  const unrelated = sourceRevision('different');
  history.mapEdits('a.vr', unrelated, unrelated, () => assert.fail('unrelated text must not move offsets'));
  assert.deepEqual(history.plan(-1, b)!.target, back.target);
});

test('history shares revision handles, hashes once and retains bounded positions', async () => {
  const history = new NavigationHistory(3), revision = sourceRevision('large file contents');
  const digest = revision.digest(); assert.equal(revision.digest(), digest);
  assert.equal(await digest, await sourceRevision('large file contents').digest());
  assert.notEqual(await digest, await sourceRevision('changed file contents').digest());
  for (let i = 0; i < 10; i++) history.record({ path: 'a.vr', revision, anchor: i, head: i }, { path: 'a.vr', revision, anchor: i + 1, head: i + 1 });
  let current = { path: 'a.vr', revision, anchor: 10, head: 10 }, count = 0;
  while (history.back) { const step = history.plan(-1, current)!; assert.equal(step.target.revision, revision); assert.equal('source' in step.target, false); history.complete(step, step.target); current = step.target; count++; }
  assert.equal(count, 2);
});

test('identical jumps are ignored and session reset invalidates pending destinations', () => {
  const history = new NavigationHistory(), a = bookmark('a.vr'), b = bookmark('b.vr');
  history.record(a, a); assert.equal(history.back, false); assert.equal(history.forward, false);
  history.record(a, b); const step = history.plan(-1, b)!; history.clear();
  assert.equal(history.back, false); assert.equal(history.forward, false); assert.equal(history.complete(step, a), false);
});

test('a semantic location refines one file arrival and cannot overwrite a newer jump', () => {
  const history = new NavigationHistory(), a = bookmark('a.vr'), b = bookmark('b.vr');
  history.record(a, b);
  const refined = { ...b, anchor: 7, head: 9 };
  assert.equal(history.reviseCurrent(b, refined), true);
  const back = history.plan(-1, refined)!; assert.deepEqual(back.target, a);
  history.complete(back, a);
  assert.equal(history.reviseCurrent(b, refined), false);
  assert.equal(history.plan(1, a)!.target.head, 9);
});

test('a rejected busy click preserves the accepted request and its explicit continuation ticket', () => {
  const requests = new NavigationRequests();
  const accepted = requests.begin(false)!;
  assert.equal(requests.begin(true), null);
  assert.equal(requests.current(accepted), true);
  assert.equal(requests.begin(true), null);
  assert.equal(requests.current(accepted), true, 'history continuation still belongs to the accepted request');
  const replacement = requests.begin(false)!;
  assert.equal(requests.current(accepted), false);
  assert.equal(requests.current(replacement), true);
  requests.invalidate();
  assert.equal(requests.current(replacement), false, 'a real synchronous jump or session reset revokes pending selection');
});
