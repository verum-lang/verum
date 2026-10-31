import test from 'node:test';
import assert from 'node:assert/strict';
import { CommandRegistry, parseShortcut, type KeyInput, type StudioCommand } from '../src/commands.ts';
const commands: StudioCommand[] = [
  { id: 'run', label: () => 'Run', shortcut: 'Mod+Enter', contexts: ['editor'], enabled: () => true, run() {} },
  { id: 'send', label: () => 'Send', shortcut: 'Mod+Enter', contexts: ['agent'], enabled: () => true, run() {} },
  { id: 'save', label: () => 'Save', shortcut: 'Mod+S', contexts: ['workspace', 'editor', 'agent'], enabled: () => true, run() {} },
];
const key: KeyInput = { code: 'Enter', ctrlKey: true, metaKey: false, altKey: false, shiftKey: false, isComposing: false, repeat: false, defaultPrevented: false };
test('dispatch uses exact modifiers, physical keys, platform and active context', () => {
  const registry = new CommandRegistry(commands);
  assert.equal(registry.match(key, 'editor', false)?.id, 'run');
  assert.equal(registry.match(key, 'agent', false)?.id, 'send');
  assert.equal(registry.match(key, 'workspace', false), undefined);
  assert.equal(registry.match(key, 'editor', true), undefined);
  assert.equal(registry.match({ ...key, ctrlKey: false, metaKey: true }, 'editor', true)?.id, 'run');
  assert.equal(registry.match({ ...key, code: 'KeyS' }, 'editor', false)?.id, 'save');
  for (const change of [{ altKey: true }, { shiftKey: true }, { metaKey: true }, { isComposing: true }, { defaultPrevented: true }, { getModifierState: () => true }]) {
    assert.equal(registry.match({ ...key, ...change }, 'editor', false), undefined);
  }
});
test('remapping persists, disable/reset restores defaults, conflicts fail atomically', () => {
  const registry = new CommandRegistry(commands);
  registry.set('run', 'Alt+Enter');
  assert.equal(registry.match(key, 'editor', false), undefined);
  assert.equal(registry.reserves(key, 'editor', false), true);
  assert.equal(registry.reserves({ ...key, isComposing: true }, 'editor', false), false);
  const persisted = registry.serialize();
  assert.throws(() => registry.set('run', 'Mod+S'), /conflict/);
  assert.equal(registry.serialize(), persisted);
  const restored = new CommandRegistry(commands); restored.restore(persisted);
  assert.equal(restored.shortcut(commands[0]), 'Alt+Enter');
  restored.set('run', ''); assert.equal(restored.match({ ...key, ctrlKey: false, altKey: true }, 'editor', false), undefined);
  assert.equal(restored.reserves(key, 'editor', false), true);
  restored.set('run', null); assert.equal(restored.match(key, 'editor', false)?.id, 'run');
  restored.reset(); assert.equal(restored.serialize(), new CommandRegistry(commands).serialize());
});
test('invalid preferences and reserved shortcuts cannot replace usable bindings', () => {
  const registry = new CommandRegistry(commands), before = registry.serialize();
  for (const binding of ['S', 'Shift+Enter', 'Mod+Mod+S', 'Mod+Ctrl+S', 'Mod+Alt+S', 'Ctrl+Alt+S', 'Mod+L', 'Mod+W', 'Mod+Shift+R', 'Alt+F4', 'Alt+Space']) assert.throws(() => parseShortcut(binding));
  for (const raw of ['null', '[]', '{}', '{"version":1,"bindings":{"unknown":"F1"}}', '{"version":1,"bindings":{"run":false}}', '{"version":1,"bindings":{"run":"Mod+S"}}']) {
    assert.throws(() => registry.restore(raw)); assert.equal(registry.serialize(), before);
  }
  const disabled = new CommandRegistry([{ ...commands[0], enabled: () => false }]);
  assert.equal(disabled.match(key, 'editor', false)?.id, 'run');
  assert.equal(disabled.match({ ...key, repeat: true }, 'editor', false)?.id, 'run');
});
test('explicit Ctrl bindings remain portable without colliding with platform Mod', () => {
  const registry = new CommandRegistry(commands); registry.set('run', 'Ctrl+Space');
  for (const mac of [true, false]) assert.equal(registry.match({ ...key, code: 'Space' }, 'editor', mac)?.id, 'run');
  assert.throws(() => registry.set('run', 'Ctrl+S'), /conflict/);
  assert.throws(() => registry.set('run', 'Meta+S'), /conflict/);
});
