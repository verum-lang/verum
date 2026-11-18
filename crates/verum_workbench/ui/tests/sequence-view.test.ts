import test from 'node:test';
import assert from 'node:assert/strict';
import { parseHTML } from 'linkedom';
import { renderSequence, type ActionSequence } from '../src/sequence-view.ts';

const { document, Node } = parseHTML('<html><body><details><summary></summary><div></div></details></body></html>');
Object.assign(globalThis, { document, Node });
const panel = document.querySelector('details')!;
const check: ActionSequence = { rule: 'studio.reviewed-tool-order/1', status: 'checked', retained: 6, omitted: 0,
  awaiting_decision: 0, unfinished: 0, succeeded: 1, failed: 1, declined: 0, refused_before_dispatch: 0, unbound_receipts: 0, violation: null };

test('a failed action can have checked ordering, while gaps and unfinished calls remain visible', () => {
  renderSequence(panel, check);
  assert.equal(panel.querySelector('summary')?.textContent, 'Action order · checked');
  assert.match(panel.textContent ?? '', /failed: 1/);
  renderSequence(panel, { ...check, status: 'incomplete', omitted: 8, unfinished: 1, awaiting_decision: 2, unbound_receipts: 1 });
  assert.equal(panel.querySelector('summary')?.textContent, 'Action order · incomplete');
  assert.match(panel.textContent ?? '', /omitted: 8/);
  assert.match(panel.textContent ?? '', /decision: 2/);
  assert.match(panel.textContent ?? '', /further evidence: 1/);
  assert.match(panel.textContent ?? '', /without a bound receipt: 1/);
});

test('the host counterexample retains literal identities and an unavailable report yields no verdict', () => {
  const observed = { ordinal: 12, review_id: 'review', tool: '<read>', turn: 'turn', step: 'dispatched' };
  renderSequence(panel, { ...check, status: 'violated', violation: { previous: null, observed } });
  assert.equal(panel.querySelector('summary')?.textContent, 'Action order · violation');
  assert.deepEqual(JSON.parse(panel.querySelector('pre')!.textContent!), { previous: null, observed });
  assert.equal(panel.querySelector('read'), null);
  renderSequence(panel, { ...check, rule: 'unknown' });
  assert.equal(panel.hidden, true);
  renderSequence(panel, null);
  assert.equal(panel.hidden, true);
});
