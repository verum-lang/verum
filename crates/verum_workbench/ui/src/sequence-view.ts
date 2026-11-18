import { m, setText, type Content } from './i18n.ts';

type Observation = { ordinal: number; review_id: string; turn: string; tool: string; step: string };
export type ActionSequence = {
  rule: string; status: 'empty' | 'incomplete' | 'checked' | 'violated';
  retained: number; omitted: number; awaiting_decision: number; unfinished: number;
  succeeded: number; failed: number; declined: number; refused_before_dispatch: number; unbound_receipts: number;
  violation: { previous: Observation | null; observed: Observation } | null;
};

const labels = {
  empty: 'Action order · no decisions yet', incomplete: 'Action order · incomplete',
  checked: 'Action order · checked', violated: 'Action order · violation',
} as const;

export function renderSequence(panel: HTMLDetailsElement, check: ActionSequence | null) {
  panel.hidden = !check || check.rule !== 'studio.reviewed-tool-order/1' || !Object.hasOwn(labels, check.status);
  const body = panel.querySelector('div')!;
  body.replaceChildren();
  if (panel.hidden || !check) return;
  panel.dataset.status = check.status;
  setText(panel.querySelector('summary')!, m(labels[check.status]));
  function line(content: Content) {
    const p = document.createElement('p');
    setText(p, content); body.append(p);
  }
  line(m('The host checks that approval precedes each tool dispatch and its outcome follows the same call. A declined review permits no dispatch.'));
  line(m('Retained host steps: {count}', { count: check.retained }));
  line(m('Succeeded: {ok} · failed: {failed} · declined: {declined} · refused before dispatch: {refused}',
    { ok: check.succeeded, failed: check.failed, declined: check.declined, refused: check.refused_before_dispatch }));
  if (check.awaiting_decision || check.unfinished) line(m('Awaiting a decision: {awaiting} · awaiting further evidence: {unfinished}',
    { awaiting: check.awaiting_decision, unfinished: check.unfinished }));
  if (check.omitted) line(m('Earlier host steps omitted: {count}. The beginning of this conversation’s action history is unavailable.', { count: check.omitted }));
  if (check.unbound_receipts) line(m('Execution outcomes without a bound receipt: {count}', { count: check.unbound_receipts }));
  if (check.violation) {
    const { observed } = check.violation;
    line(m('First violation at host step {number} for {tool}.', { number: observed.ordinal, tool: observed.tool }));
    const trace = document.createElement('pre');
    trace.textContent = JSON.stringify(check.violation, null, 2);
    body.append(trace);
  }
}
