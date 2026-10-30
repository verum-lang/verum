import { clamp, layoutLimits, readLayout, resolveLayout, type Panel, type Pane } from './layout';
import { m, setAttribute } from './i18n';

export function installPanels(measureEditor: () => void) {
  const element = (id: string) => document.getElementById(id)!;
  const app = element('app'), workspace = element('workspace');
  const storageKey = 'verum.studio.layout';
  let stored: unknown;
  try { stored = JSON.parse(localStorage.getItem(storageKey) || 'null'); } catch { /* Use defaults if storage is unavailable or invalid. */ }
  const preferences = readLayout(stored);
  let resolved = resolveLayout(preferences, workspace.clientWidth, workspace.clientHeight);
  let focusResults = false;
  const save = () => { try { localStorage.setItem(storageKey, JSON.stringify(preferences)); } catch { /* Layout still works without storage. */ } };
  const focusedPane = (): Pane | undefined => {
    const active = document.activeElement;
    const id = active?.closest('.pane')?.id ?? (active?.matches('.divider') ? active.id : undefined);
    return id ? id.replace(/-(pane|divider)$/, '') as Pane : undefined;
  };
  function selectMobile(pane: Pane) {
    app.dataset.pane = pane;
    document.querySelectorAll<HTMLButtonElement>('.mobile-nav button').forEach(button => {
      if (button.dataset.pane === pane) button.setAttribute('aria-current', 'page'); else button.removeAttribute('aria-current');
    });
  }
  function render() {
    const focused = focusedPane();
    const next = resolveLayout(preferences, workspace.clientWidth, workspace.clientHeight);
    const enteringMobile = next.mobile && !resolved.mobile;
    if (enteringMobile && focused) selectMobile(focused);
    const resultsOnly = !next.mobile && focusResults && next.maximum.results < layoutLimits.results.min;
    if (next.mobile || next.maximum.results >= layoutLimits.results.min) focusResults = false;
    app.dataset.resultsOnly = String(resultsOnly);
    if (resultsOnly) { next.collapsed.results = false; next.sizes.results = workspace.clientHeight; }
    resolved = next;
    app.dataset.layout = next.mobile ? 'mobile' : 'desktop';
    for (const panel of ['files', 'agent', 'results'] as const) {
      const collapsed = !next.mobile && next.collapsed[panel];
      app.style.setProperty(`--${panel}-${panel === 'results' ? 'height' : 'width'}`, `${next.sizes[panel]}px`);
      element(`${panel}-pane`).dataset.collapsed = String(collapsed);
      element(`${panel}-body`).hidden = collapsed;
      if (panel !== 'results') element(`${panel}-rail`).hidden = !collapsed;
      for (const toggle of document.querySelectorAll<HTMLElement>(`[data-toggle-panel="${panel}"]`)) {
        const labels = { files: ['Expand explorer', 'Collapse explorer'], agent: ['Expand collaborator', 'Collapse collaborator'], results: ['Expand results', 'Collapse results'] } as const;
        const label = m(labels[panel][collapsed ? 0 : 1]);
        toggle.setAttribute('aria-expanded', String(!collapsed));
        setAttribute(toggle, 'aria-label', label); setAttribute(toggle, 'data-tooltip', label);
      }
      const separator = element(`${panel}-divider`);
      separator.setAttribute('aria-valuemin', String(collapsed ? layoutLimits.rail : layoutLimits[panel].min));
      separator.setAttribute('aria-valuemax', String(Math.max(next.sizes[panel], layoutLimits[panel].min, next.maximum[panel])));
      separator.setAttribute('aria-valuenow', String(next.sizes[panel]));
      const disabled = collapsed || next.mobile || (panel === 'results' && resultsOnly);
      separator.setAttribute('aria-disabled', String(disabled));
      separator.tabIndex = disabled ? -1 : 0;
      if (collapsed && focused === panel) {
        element(panel === 'results' ? 'toggle-results' : `${panel}-restore`).focus();
      } else if (!next.mobile && !collapsed && focused === panel && document.activeElement?.matches('.rail-restore')) {
        element(`toggle-${panel}`).focus();
      }
    }
    if (resultsOnly && focused === 'editor') element('results-pane').focus();
    if (enteringMobile && focused && document.activeElement?.matches('.divider,.panel-toggle,.rail-restore')) element(`${focused}-pane`).focus();
    measureEditor();
  }
  function show(pane: Pane) {
    const focused = focusedPane();
    selectMobile(pane);
    if (!resolved.mobile && pane !== 'editor') {
      preferences.collapsed[pane] = false;
      if (pane !== 'results') preferences.priority = pane;
      else focusResults = true;
      save(); render();
    } else if (pane === 'editor' && focusResults) {
      focusResults = false; render();
    }
    if (resolved.mobile && focused !== pane && !document.activeElement?.matches('.mobile-nav button')) element(`${pane}-pane`).focus();
    measureEditor();
  }
  function toggle(panel: Panel) {
    if (resolved.mobile) { show(app.dataset.pane === panel ? 'editor' : panel); return; }
    preferences.collapsed[panel] = !resolved.collapsed[panel];
    if (panel !== 'results' && !preferences.collapsed[panel]) preferences.priority = panel;
    if (panel === 'results') focusResults = !preferences.collapsed.results;
    save(); render();
    // Expanding replaces the rail that owned focus; hand focus to its matching control.
    if (!resolved.collapsed[panel]) element(`toggle-${panel}`).focus();
  }
  document.querySelectorAll<HTMLElement>('[data-toggle-panel]').forEach(button => button.addEventListener('click', () => toggle(button.dataset.togglePanel as Panel)));
  document.querySelectorAll<HTMLButtonElement>('.mobile-nav button').forEach(button => button.addEventListener('click', () => show(button.dataset.pane as Pane)));
  for (const panel of ['files', 'agent', 'results'] as const) {
    const separator = element(`${panel}-divider`), vertical = panel === 'results', sign = panel === 'files' ? 1 : -1;
    const actualSize = () => {
      const rect = element(`${panel}-pane`).getBoundingClientRect();
      return vertical ? rect.height : rect.width;
    };
    const resize = (size: number) => {
      if (resolved.mobile || resolved.collapsed[panel] || (panel === 'results' && focusResults)) return;
      preferences.sizes[panel] = clamp(size, layoutLimits[panel].min, resolved.maximum[panel]);
      // Freeze the other rendered side, so a stored oversized preference cannot absorb a drag.
      if (!vertical) {
        const other = panel === 'files' ? 'agent' : 'files';
        if (!resolved.collapsed[other]) preferences.sizes[other] = resolved.sizes[other];
      }
      save(); render();
    };
    separator.addEventListener('keydown', event => {
      const delta = event.key === (vertical ? 'ArrowUp' : 'ArrowLeft') ? -16 : event.key === (vertical ? 'ArrowDown' : 'ArrowRight') ? 16 : 0;
      if (delta) { event.preventDefault(); resize(actualSize() + delta * sign); }
      if (event.key === 'Home' || event.key === 'End') { event.preventDefault(); resize(event.key === 'Home' ? layoutLimits[panel].min : resolved.maximum[panel]); }
      if (event.key === 'Enter') { event.preventDefault(); toggle(panel); }
    });
    separator.addEventListener('pointerdown', event => {
      if (event.button !== 0 || resolved.collapsed[panel]) return;
      event.preventDefault();
      const start = actualSize(), origin = vertical ? event.clientY : event.clientX;
      separator.setPointerCapture(event.pointerId); separator.classList.add('dragging');
      const move = (next: PointerEvent) => resize(start + ((vertical ? next.clientY : next.clientX) - origin) * sign);
      const stop = () => {
        separator.classList.remove('dragging');
        separator.removeEventListener('pointermove', move);
        separator.removeEventListener('lostpointercapture', stop);
        separator.removeEventListener('pointerup', stop);
        separator.removeEventListener('pointercancel', stop);
      };
      separator.addEventListener('pointermove', move);
      separator.addEventListener('lostpointercapture', stop);
      separator.addEventListener('pointerup', stop);
      separator.addEventListener('pointercancel', stop);
    });
  }
  new ResizeObserver(render).observe(workspace);
  render();
  return { show, toggle };
}
