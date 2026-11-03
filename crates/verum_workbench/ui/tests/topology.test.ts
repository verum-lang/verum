import assert from 'node:assert/strict';
import test from 'node:test';
import { moduleNeighborhood, type ProjectAnalysis } from '../src/topology.ts';
import type { Mount } from '../src/language.ts';

const range = { start: { line: 4, character: 0 }, end: { line: 4, character: 15 } };
function doc(module: string, provider: string, revision = 'r1') {
  return {
    uri: 'file:///src/' + module, source_hash: 'hash-' + module, project_revision: revision,
    parsed: true, version: 1, analysis_scope: 'project-sources-with-builtins', declarations: [], diagnostics: [],
    bindings: [{ name: 'entry', source_module: provider, type_display: 'Int', definition: null }],
    mounts: [{ source: 'mount alias.entry;', range, resolution: { state: 'resolved', bindings: { start: 0, end: 1 } } }] as Mount[],
  };
}
test('uses declared owners, source sites and only same-generation importers', () => {
  const a = doc('a', 'lib.x'), b = doc('b', 'a'), old = doc('old', 'a', 'r0');
  a.mounts.push({ source: 'mount missing;', range, resolution: { state: 'unresolved', reason: 'missing' } });
  a.mounts.push({ source: 'mount unknown;', range, resolution: null });
  const project: ProjectAnalysis = { project_revision: 'r1', documents: [a, b, old],
    sources: [a, b, old].map(d => ({ uri: d.uri, module: d.uri.split('/').at(-1) })) };
  const view = moduleNeighborhood(project, a.uri, a.source_hash)!;
  assert.deepEqual([...view.outgoing.keys()], ['lib.x']);
  assert.deepEqual([...view.incoming.keys()], ['b']);
  assert.deepEqual(view.incoming.get('b')![0].location, { uri: b.uri, range });
  assert.deepEqual(view.gaps.map(g => g.reason), ['missing', undefined]);
  assert.equal(view.analyzed, 2);
  assert.equal(view.indexed, 3);
  const full = moduleNeighborhood({ ...project, import_topology: {
    inspected_modules: 3, unresolved_mounts: 1, edges: [
      { importer: 'closed', provider: 'a', location: { uri: 'file:///src/closed', range }, source: 'mount a.entry;', names: ['entry'] },
    ],
  } }, a.uri, a.source_hash)!;
  assert.deepEqual([...full.incoming.keys()], ['closed']);
  assert.equal(full.coverage?.inspected_modules, 3);
  assert.equal(full.coverage?.unresolved_mounts, 1);
  assert.equal(moduleNeighborhood(project, a.uri, 'old hash'), null);
  assert.equal(moduleNeighborhood({ ...project, project_revision: '' }, a.uri, a.source_hash), null);
});
