import { CommandRegistry, ShortcutError, displayShortcut, shortcutAttribute, type CommandContext, type StudioCommand } from './commands';
import { icon } from './icons';
import { m, setText, setAttribute, type Content } from './i18n';

/** Palette, preferences, toolbar hints and keyboard dispatch share one command registry. */
export function installCommands(registry: CommandRegistry, failure: (error: unknown) => void, announce: (message: Content) => void) {
  const mac = /Mac|iPhone|iPad/.test(navigator.platform), storageKey = 'verum.studio.shortcuts.v1';
  const modal = document.createElement('dialog'); modal.id = 'commands-dialog'; modal.setAttribute('aria-labelledby', 'commands-title');
  document.body.append(modal);
  function element<K extends keyof HTMLElementTagNameMap>(tag: K, className: string, text: Content = '') {
    const result = document.createElement(tag); result.className = className; setText(result, text); return result;
  }
  function action(label: Content, run: () => void, name?: 'check' | 'refresh' | 'close') {
    const button = element('button', name ? 'icon-button' : 'quiet-button', name ? '' : label); button.type = 'button';
    if (name) { button.append(icon(name)); setAttribute(button, 'aria-label', label); setAttribute(button, 'data-tooltip', label); }
    button.addEventListener('click', run); return button;
  }
  function updateHints() {
    for (const command of registry.commands) {
      const shortcut = registry.shortcut(command);
      for (const control of document.querySelectorAll<HTMLElement>(command.controls || ':not(*)')) {
        control.dataset.shortcut = displayShortcut(shortcut, mac);
        if (shortcut) control.setAttribute('aria-keyshortcuts', shortcutAttribute(shortcut, mac)); else control.removeAttribute('aria-keyshortcuts');
      }
    }
    document.dispatchEvent(new Event('studio:shortcuts'));
  }
  try { const raw = localStorage.getItem(storageKey); if (raw) registry.restore(raw); }
  catch { announce(m('Saved shortcuts could not be loaded. Defaults are active.')); }
  updateHints();
  function persist() {
    updateHints();
    try { localStorage.setItem(storageKey, registry.serialize()); return m('Shortcut saved'); }
    catch { return m('Shortcut active for this session. Browser storage is unavailable.'); }
  }
  function execute(command: StudioCommand) { if (command.enabled()) Promise.resolve().then(command.run).catch(failure); }
  const context = (target: EventTarget | null): CommandContext => target instanceof Element && target.closest('.cm-content') ? 'editor'
    : target instanceof Element && target.closest('#agent-input') ? 'agent' : 'workspace';
  document.addEventListener('keydown', event => {
    if (document.querySelector('dialog[open]')) return;
    const activeContext = context(event.target), command = registry.match(event, activeContext, mac);
    if (!command && !registry.reserves(event, activeContext, mac)) return;
    // Consume disabled and repeating commands too: Save must never open the browser's save-page dialog.
    event.preventDefault(); event.stopPropagation();
    if (command && !event.repeat) execute(command);
  }, { capture: true });

  function open(title: Content, mode: string) {
    if (document.querySelector('dialog[open]')) return null;
    modal.className = mode; modal.replaceChildren();
    const heading = element('div', 'settings-heading'), name = element('h2', '', title); name.id = 'commands-title';
    heading.append(name, action(m('Close'), () => modal.close(), 'close')); modal.append(heading);
    const search = element('input', 'command-search'); search.type = 'search'; search.autocomplete = 'off'; search.spellcheck = false;
    setAttribute(search, 'aria-label', m('Search commands')); setAttribute(search, 'placeholder', m('Search commands'));
    modal.append(search); return search;
  }
  const matches = (command: StudioCommand, query: string) => query.trim().toLocaleLowerCase().split(/\s+/).every(word => `${command.label()} ${command.id}`.toLocaleLowerCase().includes(word));
  function palette() {
    const field = open(m('Command palette'), 'command-palette'); if (!field) return;
    const search = field;
    const list = element('div', 'command-list'); list.id = 'command-options'; list.setAttribute('role', 'listbox'); setAttribute(list, 'aria-label', m('Commands'));
    search.setAttribute('role', 'combobox'); search.setAttribute('aria-controls', list.id); search.setAttribute('aria-expanded', 'true'); search.setAttribute('aria-autocomplete', 'list');
    const hint = element('p', 'command-hint', m('↑ ↓ to navigate · Enter to run · Esc to close'));
    const status = element('div', 'sr-only'); status.setAttribute('role', 'status'); modal.append(list, hint, status);
    let filtered: StudioCommand[] = [], selected = -1;
    function select(index: number) {
      selected = index;
      for (const [position, row] of Array.from(list.children).entries()) row.setAttribute('aria-selected', String(position === index));
      const row = list.children[index];
      if (row) { search.setAttribute('aria-activedescendant', row.id); row.scrollIntoView({ block: 'nearest' }); }
      else search.removeAttribute('aria-activedescendant');
    }
    function choose(command: StudioCommand) {
      if (!command.enabled()) return;
      modal.close();
      // Let the native dialog restore focus before a command moves it or opens another dialog.
      queueMicrotask(() => execute(command));
    }
    function render() {
      filtered = registry.commands.filter(command => matches(command, search.value)); list.replaceChildren();
      filtered.forEach((command, index) => {
        const row = element('div', 'command-option'); row.id = `command-option-${index}`; row.setAttribute('role', 'option');
        row.setAttribute('aria-disabled', String(!command.enabled()));
        row.append(element('span', '', command.label()), element('kbd', '', displayShortcut(registry.shortcut(command), mac)));
        row.addEventListener('pointerdown', event => event.preventDefault());
        row.addEventListener('click', () => choose(command)); list.append(row);
      });
      setText(status, m('{count} commands', { count: filtered.length }));
      select(filtered.length ? 0 : -1);
      if (!filtered.length) list.append(element('p', 'empty-note', m('No matching commands')));
    }
    search.addEventListener('input', render);
    search.addEventListener('keydown', event => {
      if (event.isComposing || event.altKey || event.ctrlKey || event.metaKey) return;
      if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
        event.preventDefault(); if (filtered.length) select((selected + (event.key === 'ArrowDown' ? 1 : -1) + filtered.length) % filtered.length);
      } else if (event.key === 'Enter') { event.preventDefault(); if (filtered[selected]) choose(filtered[selected]); }
    });
    render(); modal.showModal(); search.focus();
  }
  function preferences() {
    const field = open(m('Keyboard shortcuts'), 'keyboard-preferences'); if (!field) return;
    const search = field;
    const hint = element('p', 'command-hint', m('Mod means ⌘ on macOS and Ctrl elsewhere. Use physical keys, for example Mod+Shift+K or F6. Leave empty to disable.'));
    const status = element('p', 'shortcut-status'); status.setAttribute('role', 'status');
    const list = element('div', 'shortcut-rows');
    const navigation = element('details', 'keyboard-navigation'), summary = element('summary', '', m('Navigation and editing')); navigation.append(summary);
    for (const [label, keys] of [[m('Move through controls'), 'Tab / Shift+Tab'], [m('Navigate files'), '↑ ↓ ← → / Home / End'], [m('Resize focused divider'), m('Arrow keys · Home / End')], [m('Collapse focused divider'), 'Enter'], [m('Leave editor with Tab'), m('Esc, then Tab')]] as const) {
      const row = element('div', 'shortcut-navigation-row'); row.append(element('span', '', label), element('kbd', '', keys)); navigation.append(row);
    }
    function change(run: () => void) {
      try { run(); setText(status, persist()); return true; }
      catch (error) {
        setText(status, error instanceof ShortcutError && error.kind === 'conflict' ? m('Already assigned to {command}. Clear that binding first.', { command: error.command!.label() })
          : error instanceof ShortcutError && error.kind === 'reserved' ? m('This shortcut is reserved for browser, system or text input.') : m('Invalid shortcut. Use Mod, Alt or Shift with a physical key, or a function key.'));
        return false;
      }
    }
    function render() {
      list.replaceChildren();
      for (const command of registry.commands.filter(command => matches(command, search.value))) {
        const row = element('div', 'shortcut-row'), label = element('label', 'shortcut-label', command.label()), field = element('input', 'shortcut-input');
        field.id = `binding-${command.id}`; field.value = registry.shortcut(command); field.autocomplete = 'off'; field.spellcheck = false; label.htmlFor = field.id;
        field.setAttribute('aria-describedby', 'shortcut-format');
        const scope = element('small', 'muted', command.contexts.length === 3 ? m('Workspace') : command.contexts.includes('editor') ? m('Editor') : m('Agent'));
        label.append(scope);
        const apply = () => { if (change(() => registry.set(command.id, field.value))) { field.value = registry.shortcut(command); field.removeAttribute('aria-invalid'); } else field.setAttribute('aria-invalid', 'true'); };
        field.addEventListener('keydown', event => { if (event.key === 'Enter' && !event.isComposing) { event.preventDefault(); apply(); } });
        row.append(label, field, action(m('Apply shortcut for {command}', { command: command.label() }), apply, 'check'), action(m('Reset shortcut for {command}', { command: command.label() }), () => {
          if (change(() => registry.set(command.id, null))) { field.value = registry.shortcut(command); field.removeAttribute('aria-invalid'); }
        }, 'refresh')); list.append(row);
      }
      if (!list.childElementCount) list.append(element('p', 'empty-note', m('No matching commands')));
    }
    hint.id = 'shortcut-format'; modal.append(hint, status, list, navigation, action(m('Reset all shortcuts'), () => { if (change(() => registry.reset())) render(); }));
    search.addEventListener('input', render); render(); modal.showModal(); search.focus();
  }
  return { palette, preferences };
}
