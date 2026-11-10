import assert from 'node:assert/strict';
import test from 'node:test';
import { lintPage, type LintReport } from '../src/lint.ts';
import { editFileText } from '../src/source.ts';

test('one editor transaction preserves untouched mixed endings and UTF-16 positions', () => {
  const original = '😀a\r\nb\rc\nd';
  assert.equal(editFileText(original, [
    { from: 2, to: 3, insert: 'A' }, { from: 6, to: 7, insert: 'C\nX' },
  ]), '😀A\r\nb\rC\r\nX\nd');
  assert.equal(editFileText(original, [{ from: 3, to: 4, insert: '' }]), '😀ab\rc\nd');
});

test('project lint filters across files and preserves partial parser coverage under pagination', () => {
  const finding = { rule: 'unused-import', severity: 2, message: 'Unused mount', position: null, edits: [] };
  const report: LintReport = { schema_version: 1, files: [
    { path: 'pkg/a.vr', source_hash: 'a', parsed: false, findings: [] },
    { path: 'pkg/b.vr', source_hash: 'b', parsed: true, findings: [finding, finding] },
    { path: 'clean.vr', source_hash: 'c', parsed: true, findings: [] },
  ] };
  const first = lintPage(report, '', 1);
  assert.deepEqual([first.total, first.parsed, first.matched], [2, 2, 3]);
  assert.deepEqual(first.groups.map(group => group.findings), [[null]]);
  const selection = lintPage(report, 'B.VR mount', 1);
  assert.equal(selection.matched, 2);
  assert.deepEqual(selection.groups.map(group => group.file.path), ['pkg/b.vr']);
  assert.equal(lintPage(report, 'missing', 3).groups.length, 0);
});
