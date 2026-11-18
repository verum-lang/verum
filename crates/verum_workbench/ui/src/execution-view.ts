import { m, setText, t, type Content } from './i18n.ts';
import type { ExecutionPlan, RunPermissions } from './model.ts';
import { sourceHash } from './source.ts';

const operations = { run: 'Execution', check: 'Module check', proof: 'Proof check' } as const;
const permissions = { source: 'Source / CLI defaults', 'deny-all': 'Deny all grants', 'allow-all': 'Allow all grants' } as const;
export const operationLabel = (kind: ExecutionPlan['kind']) => m(operations[kind]);
export const permissionLabel = (policy?: RunPermissions | null) => m(policy ? permissions[policy] : 'Not recorded');

export type ExecutionPreview = { plan: ExecutionPlan; source: string };

export async function executionPreviewView(preview: ExecutionPreview) {
  const plan = preview?.plan;
  const incomplete = () => new Error(t(m('The host returned an incomplete execution plan.')));
  if (!plan || !Object.hasOwn(operations, plan.kind) || typeof plan.path !== 'string' || !plan.path
    || typeof plan.executable !== 'string' || !plan.executable || !Number.isSafeInteger(plan.timeout_ms)
    || (plan.source_mode !== 'source' && plan.source_mode !== 'script')
    || typeof plan.source_hash !== 'string' || typeof preview.source !== 'string') throw incomplete();
  if (await sourceHash(preview.source) !== plan.source_hash.toLowerCase()) {
    throw new Error(t(m('The preview source does not match its SHA-256.')));
  }

  const fields = document.createElement('dl');
  fields.className = 'approval-plan';
  function field(label: Content, value: Content) {
    const term = document.createElement('dt'), definition = document.createElement('dd');
    setText(term, label); setText(definition, value);
    fields.append(term, definition);
  }
  field(m('Operation'), operationLabel(plan.kind));
  field(m('File path'), plan.path);
  field(m('Source mode'), m(plan.source_mode === 'script' ? 'Script entry' : 'Source declarations'));
  field(m('Time limit'), m('{milliseconds} ms', { milliseconds: plan.timeout_ms }));
  if (plan.kind === 'run') {
    if (!plan.permissions || !Object.hasOwn(permissions, plan.permissions) || typeof plan.runtime_diagnostics !== 'boolean') throw incomplete();
    field(m('Script permissions'), permissionLabel(plan.permissions));
    field(m('Runtime diagnostics'), m(plan.runtime_diagnostics ? 'On' : 'Off'));
  }
  if (plan.kind === 'proof') {
    if (typeof plan.declaration !== 'string' || !plan.declaration || typeof plan.proof_profile !== 'string' || !plan.proof_profile) throw incomplete();
    field(m('Proof declaration'), plan.declaration);
    field(m('Proof profile'), plan.proof_profile);
  }
  field(m('Executable'), plan.executable);
  field(m('Source SHA-256'), plan.source_hash);

  const details = document.createElement('details'), summary = document.createElement('summary'), source = document.createElement('pre');
  setText(summary, m('Inspect exact source'));
  source.textContent = preview.source;
  details.append(summary, source);
  const view = document.createDocumentFragment();
  view.append(fields, details);
  return view;
}
