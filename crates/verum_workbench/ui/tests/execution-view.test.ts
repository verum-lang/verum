import test from 'node:test';
import assert from 'node:assert/strict';
import { parseHTML } from 'linkedom';
import { executionPreviewView } from '../src/execution-view.ts';
import type { ExecutionPlan } from '../src/model.ts';
import { sourceHash } from '../src/source.ts';

const { document, Node } = parseHTML('<html><body></body></html>');
Object.assign(globalThis, { document, Node });
const source = '// λ\r\nprint("<b>Run</b>");\n';
const plan: ExecutionPlan = {
  kind: 'run', path: 'entry.vr', executable: '/bin/verum', source_mode: 'script', source_hash: await sourceHash(source),
  timeout_ms: 1, permissions: 'allow-all', runtime_diagnostics: true, declaration: null, proof_profile: null,
};
function fields(view: DocumentFragment) {
  return Object.fromEntries([...view.querySelectorAll('dt')].map(term => [term.textContent, term.nextElementSibling?.textContent]));
}

test('a run review shows the host plan and preserves the complete source as collapsed, literal text', async () => {
  const view = await executionPreviewView({ plan, source });
  assert.deepEqual(fields(view), {
    Operation: 'Execution', 'File path': 'entry.vr', 'Source mode': 'Script entry', 'Time limit': '1 ms',
    'Script permissions': 'Allow all grants', 'Runtime diagnostics': 'On', Executable: '/bin/verum', 'Source SHA-256': plan.source_hash,
  });
  assert.equal(view.querySelector('details')?.hasAttribute('open'), false);
  assert.equal(view.querySelector('pre')?.textContent, source);
  assert.equal(view.querySelector('b'), null);
});

test('a proof review identifies its declaration, exact profile and proof checker', async () => {
  const proof = { ...plan, kind: 'proof' as const, source_mode: 'source' as const, executable: '/bin/verum-source-replay',
    permissions: null, runtime_diagnostics: null, declaration: 'identity', proof_profile: 'verum-source-pi-id/1' as const };
  const data = fields(await executionPreviewView({ plan: proof, source }));
  assert.equal(data.Operation, 'Proof check');
  assert.equal(data['Proof declaration'], 'identity');
  assert.equal(data['Proof profile'], 'verum-source-pi-id/1');
  assert.equal(data.Executable, '/bin/verum-source-replay');
  assert.ok(!('Script permissions' in data));
});

test('an incomplete plan or a different source cannot produce an approvable preview', async () => {
  await assert.rejects(executionPreviewView({ plan, source: source.replace('\r\n', '\n') }), /SHA-256/);
  for (const difference of [{ source_mode: null }, { permissions: null }, { runtime_diagnostics: null },
    { executable: '' }, { kind: 'proof' as const, proof_profile: null }]) {
    await assert.rejects(executionPreviewView({ plan: { ...plan, ...difference }, source }), /incomplete execution plan/);
  }
});
