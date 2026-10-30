import test from 'node:test';
import assert from 'node:assert/strict';
import { positionAt, offsetAt, rangeOffsets, sameRevision } from '../src/language.ts';

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
});
