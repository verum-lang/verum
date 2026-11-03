import assert from 'node:assert/strict';
import test from 'node:test';
import { moduleNeighborhood, importDependencyPath, type ProjectAnalysis } from '../src/topology.ts';
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
  assert.equal(view.dependents, null); // Open documents cannot stand in for the project graph.
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

test('reverse reachability preserves a shortest source witness through diamonds and cycles', () => {
  const selected = doc('base', 'unrelated');
  const edge = (importer: string, provider: string) => ({ importer, provider,
    source: `mount ${provider}.entry;`, location: { uri: `file:///src/${importer}`, range }, names: ['entry'] });
  const project: ProjectAnalysis = { project_revision: 'r1', documents: [selected],
    sources: [{ uri: selected.uri, module: 'base' }],
    import_topology: { inspected_modules: 6, unresolved_mounts: 1, failures: [
      { importer: 'hidden', source: 'mount missing;', location: { uri: 'file:///src/hidden', range }, reason: 'missing' },
    ], edges: [edge('left', 'base'), edge('right', 'base'), edge('tip', 'left'), edge('tip', 'right'),
      edge('base', 'tip'), edge('tip', 'tip'), edge('leaf', 'tip'), edge('far', 'unrelated'), edge('left', 'base')] },
  };
  const view = moduleNeighborhood(project, selected.uri, selected.source_hash)!;
  assert.deepEqual([...view.dependents!].map(([name, item]) => [name, item.distance]),
    [['left', 1], ['right', 1], ['tip', 2], ['leaf', 3]]);
  const path = importDependencyPath(view.dependents!, 'leaf');
  assert.deepEqual(path.map(e => [e.importer, e.provider]), [['leaf', 'tip'], ['tip', 'left'], ['left', 'base']]);
  for (const step of path) assert.ok(project.import_topology!.edges.includes(step));
  assert.deepEqual(importDependencyPath(view.dependents!, 'base'), []);
  assert.deepEqual(importDependencyPath(view.dependents!, 'hidden'), []);
  assert.equal(view.coverage!.unresolved_mounts, 1);
  assert.equal(moduleNeighborhood({ ...project, project_revision: 'r2' }, selected.uri, selected.source_hash), null);
});

test('long import chains use an iterative predecessor tree, not recursive copied paths', () => {
  const selected = doc('m0', 'none');
  const edges = Array.from({ length: 12000 }, (_, i) => ({ importer: `m${i + 1}`, provider: `m${i}`,
    source: `mount m${i}.entry;`, location: { uri: `file:///src/m${i + 1}`, range }, names: ['entry'] }));
  const project: ProjectAnalysis = { project_revision: 'r1', documents: [selected],
    sources: [{ uri: selected.uri, module: 'm0' }],
    import_topology: { inspected_modules: 12001, unresolved_mounts: 0, edges } };
  const view = moduleNeighborhood(project, selected.uri, selected.source_hash)!;
  assert.equal(view.dependents!.size, 12000);
  assert.equal(view.dependents!.get('m12000')!.distance, 12000);
  assert.equal(importDependencyPath(view.dependents!, 'm12000').length, 12000);
});
