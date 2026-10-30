import test from 'node:test';
import assert from 'node:assert/strict';
import { layoutLimits, readLayout, resolveLayout } from '../src/layout.ts';

test('viewport pressure preserves a usable editor and never overwrites restoration preferences', () => {
  for (const files of [180, 224, 380]) for (const agent of [300, 344, 600]) {
    const preferences = readLayout({ sizes: { files, agent, results: 600 } });
    const saved = structuredClone(preferences);
    for (let width = layoutLimits.mobile + 1; width <= 1800; width += 17) for (const height of [300, 372, 600, 900]) {
      const layout = resolveLayout(preferences, width, height);
      assert.ok(width - layout.sizes.files - layout.sizes.agent - 10 >= layoutLimits.editorWidth);
      assert.ok(height - layout.sizes.results - 5 >= layoutLimits.editorHeight);
      for (const panel of ['files', 'agent', 'results'] as const) {
        if (layout.collapsed[panel]) assert.equal(layout.sizes[panel], 40);
        else {
          assert.ok(layout.sizes[panel] >= layoutLimits[panel].min);
          assert.ok(layout.sizes[panel] <= layout.maximum[panel]);
        }
      }
    }
    assert.deepEqual(preferences, saved);
    assert.deepEqual(resolveLayout(preferences, 1800, 1000).sizes, saved.sizes);
  }
});

test('explicit expansion wins scarce space; returning to a wide viewport restores both panes', () => {
  const preferences = readLayout(null);
  assert.deepEqual(resolveLayout(preferences, 900, 800).collapsed, { files: true, agent: false, results: false });
  preferences.priority = 'files';
  assert.deepEqual(resolveLayout(preferences, 900, 800).collapsed, { files: false, agent: true, results: false });
  assert.deepEqual(resolveLayout(preferences, 1440, 800).collapsed, { files: false, agent: false, results: false });
  preferences.collapsed.files = true;
  assert.equal(resolveLayout(preferences, 1440, 800).collapsed.files, true);
  assert.equal(resolveLayout(preferences, 1440, 350).collapsed.results, true);
});

test('storage corruption and invalid sizes recover without poisoning CSS dimensions', () => {
  for (const value of [null, false, [], 'bad', { sizes: { files: NaN, agent: Infinity, results: '800' }, collapsed: { files: 'false' } }]) {
    assert.deepEqual(readLayout(value), readLayout(null));
  }
  assert.deepEqual(readLayout({ sizes: { files: -100, agent: 9999, results: 0 } }).sizes, { files: 180, agent: 600, results: 140 });
});
