import test from 'node:test';
import assert from 'node:assert/strict';
import { parseHTML } from 'linkedom';
import { diagnosticContext, type Diagnostic } from '../src/language.ts';

const diagnostic: Diagnostic = {
  code: 'E312', message: 'reference outlives referent',
  range: { start: { line: 1, character: 3 }, end: { line: 1, character: 8 } },
};

test('compiler details retain their order and render source-derived messages as text', () => {
  const { document } = parseHTML('<html></html>');
  assert.equal(diagnosticContext(document, diagnostic), null);
  assert.equal(diagnosticContext(document, { ...diagnostic, data: { verum: { notes: [], help: [''] } } }), null);
  const context = diagnosticContext(document, { ...diagnostic, data: { verum: {
    notes: ['in collect<T>', '<a>source</a>'], help: ['return an owned value'],
  } } });
  assert.ok(context);
  assert.deepEqual([...context.children].map(node => [node.localName, node.textContent]), [
    ['dt', 'Context'], ['dd', 'in collect<T>'], ['dd', '<a>source</a>'],
    ['dt', 'Suggested action'], ['dd', 'return an owned value'],
  ]);
  assert.equal(context.querySelector('a'), null);
});
