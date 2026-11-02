import test from 'node:test';
import assert from 'node:assert/strict';
import { projectSourceIndex, workspaceFilePath, projectedTree, visibleTree, changedDirectories, retainedTreeFocus, type ProjectEntry } from '../src/project-tree.ts';

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

test('refresh keeps keyboard position, or selects a surviving parent or neighbor', () => {
  const tree = projectedTree(['src/api/request.vr', 'src/main.vr', 'tests/a.vr', 'README.md'].map(path => ({ path, kind: 'file', size: 0 })));
  const before = visibleTree(tree, new Set(['src', 'src/api', 'tests']));
  const collapsed = visibleTree(tree, new Set());
  assert.equal(retainedTreeFocus(before, [...before], 'src/api/request.vr', 'README.md'), 'src/api/request.vr');
  assert.equal(retainedTreeFocus(before, collapsed, 'src/api/request.vr', 'README.md'), 'src');
  assert.equal(retainedTreeFocus(before, before.filter(row => row.path !== 'README.md'), 'README.md', 'src/main.vr'), 'tests/a.vr');
  assert.equal(retainedTreeFocus([], before, '', 'README.md'), 'README.md');
  assert.equal(retainedTreeFocus(before, [], 'src', 'README.md'), '');
});

test('analysis snapshots and definition links use the same workspace-relative paths', () => {
  const root = '/srv/registry';
  const uri = 'file:///srv/registry/src/publish%20cog.vr';
  assert.equal(workspaceFilePath(uri, root), 'src/publish cog.vr');
  assert.equal(workspaceFilePath('file:///srv/registry-copy/src/main.vr', root), null);
  assert.equal(workspaceFilePath('file://other/srv/registry/src/main.vr', root), null);
  assert.equal(workspaceFilePath('file:///C:/registry/src/main.vr', 'C:\\registry'), 'src/main.vr');
  assert.equal(workspaceFilePath('file://build/share/registry/src/main.vr', '\\\\build\\share\\registry'), 'src/main.vr');
  const index = projectSourceIndex(root, [{ uri }, { uri: 'file:///srv/registry/src/server.vr' }], [{ uri, source_hash: 'verified' }]);
  assert.equal(index.get('src/publish cog.vr'), 'verified');
  assert.equal(index.get('src/server.vr'), null);
  assert.equal(index.has('src/unseen.vr'), false);
});
