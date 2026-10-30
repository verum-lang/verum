import test from 'node:test';
import assert from 'node:assert/strict';
import { projectedTree, visibleTree, changedDirectories, type ProjectEntry } from '../src/project-tree.ts';

test('filtered and deleted paths retain ancestors, folders first and exact sibling positions', () => {
  const files: ProjectEntry[] = ['README.md', 'src/api/request.vr', 'src/main.vr', 'tests/api.vr'].map(path => ({ path, kind: 'file', size: 0 }));
  const tree = projectedTree(files);
  assert.deepEqual(visibleTree(tree, new Set()).map(row => row.path), ['src', 'tests', 'README.md']);
  const rows = visibleTree(tree, new Set(['src', 'src/api']));
  assert.deepEqual(rows.map(row => [row.path, row.depth, row.position, row.siblings]), [['src', 1, 1, 3], ['src/api', 2, 1, 2], ['src/api/request.vr', 3, 1, 1], ['src/main.vr', 2, 2, 2], ['tests', 1, 2, 3], ['README.md', 1, 3, 3]]);
  assert.equal(visibleTree(tree, new Set(), true).length, 7);
});
test('change counts aggregate by path components, not string prefixes', () => {
  const changes = ['src/api/a.vr', 'src/api/b.vr', 'src/api2/c.vr'].map(path => ({ path, index: ' ', worktree: 'D', original_path: null }));
  const counts = changedDirectories(changes);
  assert.equal(counts.get('src'), 3); assert.equal(counts.get('src/api'), 2); assert.equal(counts.get('src/api2'), 1);
});
