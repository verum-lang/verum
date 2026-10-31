import type { Receipt } from './model';
import type { CbgrEvent } from './runtime';
import { m, setText, type Content } from './i18n';

const element = <K extends keyof HTMLElementTagNameMap>(tag: K, content?: Content) => {
  const node = document.createElement(tag);
  if (content !== undefined) setText(node, content);
  return node;
};

export function runtimeView(receipt: Receipt, canNavigate: (event: CbgrEvent) => boolean, navigate: (event: CbgrEvent) => void) {
  if (!receipt.runtime_diagnostics) return null;
  const section = element('section'); section.className = 'runtime-diagnostics';
  section.setAttribute('aria-label', 'CBGR');
  section.append(element('h3', m('CBGR runtime observations')));
  const report = receipt.runtime;
  if (!report || report.observation !== 'captured' || !report.cbgr) {
    section.append(element('p', m(receipt.status === 'running' ? 'Waiting for runtime observations…' : 'Runtime observations unavailable.')));
    if (receipt.runtime_error) section.append(element('pre', receipt.runtime_error));
    return section;
  }
  const cbgr = report.cbgr;
  if (!cbgr.checks_enabled_at_start) section.append(element('p', m('CBGR checks were disabled when capture started.')));
  section.append(element('p', m(cbgr.events.length ? '{count} dispatch observations' : 'No CBGR failures observed in this capture.', { count: cbgr.events.length })));
  if (cbgr.truncated) { const warning = element('p', m('Capture limit reached. Additional observations were dropped.')); warning.className = 'runtime-truncated'; section.append(warning); }
  const coverage = element('details'); coverage.dataset.key = 'cbgr-coverage';
  const summary = element('summary', m('Capture scope')); summary.dataset.focusKey = 'cbgr-coverage';
  coverage.append(summary, element('p', m('CBGR errors returned by this interpreter’s table dispatcher only. Other interpreters, native/AOT code, allocation lifetimes and successful validation checks are not captured. One failure may appear at several call boundaries. This is not a memory-safety proof.')));
  section.append(coverage);
  for (const [index, event] of cbgr.events.entries()) {
    const item = element('details'); item.dataset.key = `cbgr-${index}`;
    const heading = element('summary', `${event.kind} · ${event.function ?? event.function_id ?? '?'} · PC ${event.pc}`);
    heading.dataset.focusKey = `cbgr-${index}`; item.append(heading);
    item.append(element('p', `${event.target_kind}: ${event.target}`));
    if (event.expected !== null || event.actual !== null) item.append(element('p', m('Expected {expected} · actual {actual}', { expected: event.expected ?? '—', actual: event.actual ?? '—' })));
    if (event.source) item.append(element('p', `${event.source.path}:${event.source.line}:${event.source.column}`));
    else item.append(element('p', m('No exact source-map entry. Bytecode location retained.')));
    if (canNavigate(event)) {
      const link = element('button', m('Reveal source line')); link.className = 'quiet-button';
      link.dataset.focusKey = `cbgr-source-${index}`; link.addEventListener('click', () => navigate(event)); item.append(link);
    }
    section.append(item);
  }
  return section;
}
