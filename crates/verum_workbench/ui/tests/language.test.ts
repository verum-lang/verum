import test from 'node:test';
import assert from 'node:assert/strict';
import { analysisScopeLabel, positionAt, offsetAt, rangeOffsets, sameRevision, textEditChanges } from '../src/language.ts';

test('LSP locations round-trip UTF-16 including non-BMP characters and CRLF', () => {
  const source = '😀name\r\nsecond\n';
  for (const offset of [0, 2, 3, 6, 8, 10, source.length]) assert.equal(offsetAt(source, positionAt(source, offset)), offset);
  assert.equal(offsetAt(source, { line: 0, character: 1 }), null);
  assert.equal(offsetAt(source, { line: 0, character: 99 }), 6);
  assert.equal(offsetAt(source, { line: 3, character: 0 }), null);
  assert.equal(rangeOffsets(source, { start: { line: 1, character: 3 }, end: { line: 0, character: 0 } }), null);
});

test('an obsolete analysis cannot describe a new buffer or another file with identical text', () => {
  const snapshot = { path: 'one.vr', source: 'fn one() {}' };
  assert.equal(sameRevision(snapshot, { ...snapshot }), true);
  assert.equal(sameRevision(snapshot, { ...snapshot, path: 'two.vr' }), false);
  assert.equal(sameRevision(snapshot, { ...snapshot, source: snapshot.source + '\n' }), false);
  assert.equal(sameRevision(snapshot, { ...snapshot, source_mode: 'script' }), false);
  assert.equal(sameRevision(snapshot, { ...snapshot, source_mode: 'source' }), true);
});

test('format edits preserve untouched Unicode and CRLF text and use original coordinates', () => {
  const source = '// 😀\r\nfn a(){}\r\nfn b(){}';
  const edit = (line: number, character: number, newText: string) => ({ range: { start: { line, character }, end: { line, character } }, newText });
  const changes = textEditChanges(source, [edit(2, 6, ' '), edit(1, 6, ' ')]);
  assert.ok(changes);
  let result = source;
  for (const change of changes.toReversed()) result = result.slice(0, change.from) + change.insert + result.slice(change.to);
  assert.equal(result, '// 😀\r\nfn a() {}\r\nfn b() {}');
  assert.deepEqual(textEditChanges(source, null), []);
});

test('a malformed, overlapping or out-of-bounds formatting set is rejected atomically', () => {
  const source = '😀 abc\n';
  const edit = (start: number, end: number) => ({ range: { start: { line: 0, character: start }, end: { line: 0, character: end } }, newText: 'x' });
  for (const edits of [undefined, {}, [null], [edit(2, 3), edit(1, 2)], [edit(0, 4), edit(3, 6)], [edit(0, 0), edit(0, 0)], [edit(9, 10)], [edit(4, 2)], [{ ...edit(0, 2), newText: null }], [{ ...edit(0, 2), range: {} }]]) {
    assert.equal(textEditChanges(source, edits), null, JSON.stringify(edits));
  }
});

test('diagnostic scope distinguishes release-backed analysis from builtins and unknown modes', () => {
  assert.equal(analysisScopeLabel('project-sources-with-library-metadata').key, 'Project sources and compiled standard library');
  assert.equal(analysisScopeLabel('project-sources-with-builtins').key, 'Project sources and compiler builtins');
  assert.equal(analysisScopeLabel('single-module-with-builtins').key, 'Current module and compiler builtins');
  assert.equal(analysisScopeLabel('new-compiler-mode').key, 'Analysis scope unavailable');
});
