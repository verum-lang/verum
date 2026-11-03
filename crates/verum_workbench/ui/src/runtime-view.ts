import type { Receipt } from './model';
import { cbgrObservationState, rankMethodSites, taskSnapshotCounts, type AsyncTaskReport, type AsyncTaskStatus, type CbgrEvent, type MethodDispatchReport, type MethodOutcome } from './runtime';
import { m, setText, setAttribute, type Content } from './i18n';

const element = <K extends keyof HTMLElementTagNameMap>(tag: K, content?: Content) => {
  const node = document.createElement(tag);
  if (content !== undefined) setText(node, content);
  return node;
};

function methodOutcome(outcome: MethodOutcome): Content {
  switch (outcome.kind) {
    case 'entered_frame': return m('Entered function #{id}', { id: outcome.function_id });
    case 'completed_inline': return m('Completed inline');
    case 'retried': return m('Retried');
    case 'returned': return m('Returned');
    case 'yielded': return m('Yielded');
    case 'failed': return m('Failed');
  }
}

function methodView(report: MethodDispatchReport) {
  const details = element('details'); details.dataset.key = 'method-profile';
  const summary = element('summary', m('Method dispatch measurements'));
  summary.dataset.focusKey = 'method-profile';
  details.append(summary);
  if (report.rows.length) {
    details.append(element('p', m('Sorted by inclusive handler time. Synchronous nested calls can overlap.')));
    for (const row of rankMethodSites(report)) {
      const item = element('article');
      const caller = row.function ?? `#${row.function_id}`, selector = row.selector ?? `#${row.selector_id}`;
      item.append(element('code', `${row.module ?? `#${row.module_instance}`} · ${caller}@${row.pc} → ${selector}`),
        element('p', methodOutcome(row.outcome)),
        element('p', m('{calls} calls · {total} ns total · {peak} ns maximum',
          { calls: row.count, total: row.inclusive_ns, peak: row.max_ns })));
      details.append(item);
    }
  } else {
    details.append(element('p', m('No retained method dispatch measurements.')));
  }
  if (report.dropped_calls !== '0') {
    details.append(element('p', m('{count} calls exceeded the capture capacity.', { count: report.dropped_calls })));
  }
  return details;
}

function taskStatus(status: AsyncTaskStatus): Content {
  switch (status) {
    case 'pending': return m('Pending');
    case 'running': return m('Running');
    case 'suspended': return m('Suspended');
    case 'completed': return m('Completed');
    case 'failed': return m('Failed');
  }
}

function taskView(report: AsyncTaskReport) {
  const details = element('details'); details.dataset.key = 'async-tasks';
  const counts = taskSnapshotCounts(report);
  details.open = counts.failed > 0;
  const summary = element('summary', m('Task queue · {count} retained · {failed} failed',
    { count: report.rows.length, failed: counts.failed }));
  summary.dataset.focusKey = 'async-tasks'; details.append(summary);
  details.append(element('p', m('Snapshot of recent cooperative tasks, including detached tasks. Nursery jobs and native worker threads are outside this capture. A completed task may return an application error value; completion is not a correctness proof.')));
  if (report.omitted_tasks !== '0') details.append(element('p', m('{count} older tasks omitted. Their outcomes are not shown.', { count: report.omitted_tasks })));
  for (const row of report.rows) {
    const item = element('article');
    item.append(element('code', `#${row.task_id} · ${row.module ? `${row.module} · ` : ''}${row.function ?? (row.function_id === null ? '—' : `#${row.function_id}`)}`),
      element('p', taskStatus(row.status)));
    if (!row.entered) item.append(element('p', m('No task entry frame observed.')));
    if (row.error) item.append(element('pre', row.error));
    if (row.error_truncated) item.append(element('p', m('Error text truncated for this report. The task retains its original failure.')));
    details.append(item);
  }
  return details;
}

export function runtimeView(receipt: Receipt, canNavigate: (event: CbgrEvent) => boolean, navigate: (event: CbgrEvent) => void) {
  if (!receipt.runtime_diagnostics) return null;
  const section = element('section'); section.className = 'runtime-diagnostics';
  setAttribute(section, 'aria-label', m('Runtime observations'));
  section.append(element('h3', m('Runtime observations')));
  const report = receipt.runtime;
  if (!report || report.observation !== 'captured' || !report.cbgr) {
    section.append(element('p', m(receipt.status === 'running' ? 'Waiting for runtime observations…' : 'Runtime observations unavailable.')));
    if (receipt.runtime_error) section.append(element('pre', receipt.runtime_error));
    return section;
  }
  const loss = report.export_loss;
  if (loss && loss.cbgr_events + loss.method_sites + loss.task_rows > 0) {
    const warning = element('p', m('Report byte limit reached. Omitted: {cbgr} CBGR observations, {methods} method sites, {tasks} task outcomes. Missing observations cannot establish absence of failures.',
      { cbgr: loss.cbgr_events, methods: loss.method_sites, tasks: loss.task_rows }));
    warning.className = 'runtime-truncated'; section.append(warning);
  }
  if (report.method_dispatch) section.append(methodView(report.method_dispatch));
  if (report.async_tasks) section.append(taskView(report.async_tasks));
  const cbgr = report.cbgr;
  if (!cbgr.checks_enabled_at_start) section.append(element('p', m('CBGR checks were disabled when capture started.')));
  section.append(element('p', m(cbgrObservationState(report) === 'omitted'
    ? 'No CBGR failure records retained; capture or export omitted observations.'
    : cbgr.events.length ? '{count} dispatch observations' : 'No CBGR failures observed in this capture.', { count: cbgr.events.length })));
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
