import assert from 'node:assert/strict';
import test from 'node:test';
import { moduleNeighborhood, importDependencyPath, type ProjectAnalysis, type ProjectSource } from '../src/topology.ts';
import type { Mount } from '../src/language.ts';

const range = { start: { line: 4, character: 0 }, end: { line: 4, character: 15 } };
function source(module: string): ProjectSource {
  return { uri: 'file:///src/' + module, module, source_hash: 'hash-' + module,
    source_mode: 'source', version: 1, parsed: true };
}
function topology(edges: NonNullable<ProjectAnalysis['import_topology']>['edges'], inspected_modules: number) {
  return { scope: 'indexed-source-mounts' as const, project_revision: 'r1', uninspected_sources: [],
    inspected_modules, unresolved_mounts: 0, edges };
}
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
    sources: ['a', 'b', 'old'].map(source) };
  const view = moduleNeighborhood(project, a.uri, a.source_hash)!;
  assert.deepEqual([...view.outgoing.keys()], ['lib.x']);
  assert.deepEqual([...view.incoming.keys()], ['b']);
  assert.deepEqual(view.incoming.get('b')![0].location, { uri: b.uri, range });
  assert.deepEqual(view.incoming.get('b')![0].origin, source('b'));
  assert.deepEqual(view.outgoing.get('lib.x')![0].origin, source('a'));
  assert.deepEqual(view.gaps.map(g => g.reason), ['missing', undefined]);
  assert.equal(view.analyzed, 2);
  assert.equal(view.indexed, 3);
  assert.equal(view.dependents, null); // Open documents cannot stand in for the project graph.
  const full = moduleNeighborhood({ ...project, import_topology: {
    ...topology([
      { origin: source('closed'), importer: 'closed', provider: 'a', location: { uri: 'file:///src/closed', range }, source: 'mount a.entry;', names: ['entry'] },
    ], 3), unresolved_mounts: 1,
  } }, a.uri, a.source_hash)!;
  assert.deepEqual([...full.incoming.keys()], ['closed']);
  assert.equal(full.coverage?.inspected_modules, 3);
  assert.equal(full.coverage?.unresolved_mounts, 1);
  assert.deepEqual(full.incoming.get('closed')![0].origin, source('closed'));
  assert.equal(moduleNeighborhood(project, a.uri, 'old hash'), null);
  assert.equal(moduleNeighborhood({ ...project, project_revision: '' }, a.uri, a.source_hash), null);
});

test('reverse reachability preserves a shortest source witness through diamonds and cycles', () => {
  const selected = doc('base', 'unrelated');
  const edge = (importer: string, provider: string) => ({ origin: source(importer), importer, provider,
    source: `mount ${provider}.entry;`, location: { uri: `file:///src/${importer}`, range }, names: ['entry'] });
  const project: ProjectAnalysis = { project_revision: 'r1', documents: [selected],
    sources: [source('base')],
    import_topology: { ...topology([edge('left', 'base'), edge('right', 'base'), edge('tip', 'left'), edge('tip', 'right'),
      edge('base', 'tip'), edge('tip', 'tip'), edge('leaf', 'tip'), edge('far', 'unrelated'), edge('left', 'base')], 6),
      unresolved_mounts: 1, failures: [
      { origin: source('hidden'), importer: 'hidden', source: 'mount missing;', location: { uri: 'file:///src/hidden', range }, reason: 'missing' },
    ] },
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
  const edges = Array.from({ length: 12000 }, (_, i) => ({ origin: source(`m${i + 1}`), importer: `m${i + 1}`, provider: `m${i}`,
    source: `mount m${i}.entry;`, location: { uri: `file:///src/m${i + 1}`, range }, names: ['entry'] }));
  const project: ProjectAnalysis = { project_revision: 'r1', documents: [selected],
    sources: [source('m0')],
    import_topology: topology(edges, 12001) };
  const view = moduleNeighborhood(project, selected.uri, selected.source_hash)!;
  assert.equal(view.dependents!.size, 12000);
  assert.equal(view.dependents!.get('m12000')!.distance, 12000);
  assert.equal(importDependencyPath(view.dependents!, 'm12000').length, 12000);
});

test('unknown source coverage survives projection and cannot become an absent dependency claim', () => {
  const selected = doc('base', 'provider');
  const unknown = { ...source('broken'), parsed: false, version: null };
  const uninspected = [{ origin: unknown, reason: 'parse_unavailable' as const }];
  const project: ProjectAnalysis = { project_revision: 'r1', documents: [selected],
    sources: [source('base'), unknown],
    import_topology: { ...topology([], 1), uninspected_sources: uninspected } };
  const view = moduleNeighborhood(project, selected.uri, selected.source_hash)!;
  assert.equal(view.incoming.size, 0);
  assert.equal(view.dependents!.size, 0);
  assert.deepEqual(view.coverage!.uninspected_sources, uninspected);
  assert.equal(view.coverage!.inspected_modules, 1);
  assert.equal(view.indexed, 2);

  assert.equal(moduleNeighborhood({ ...project, import_topology: {
    ...project.import_topology!, project_revision: 'stale',
  } }, selected.uri, selected.source_hash), null);
  assert.equal(moduleNeighborhood({ ...project, sources: [
    { ...source('base'), source_hash: 'different overlay' }, unknown,
  ] }, selected.uri, selected.source_hash), null);
});
