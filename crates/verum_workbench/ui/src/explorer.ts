import { api } from './api';
import { icon } from './icons';
import { m, t, setText, setAttribute } from './i18n';
import { parentPath, fileName, projectedTree, visibleTree, changedDirectories, type ProjectEntry, type ProjectListing, type GitSnapshot, type GitChange, type TreeRow } from './project-tree';

export function installExplorer(hooks: { open(path: string): Promise<void>; review(change: GitChange): Promise<void>; failure(error: unknown): void; listed(root: string, entries: ProjectEntry[], directory: string): void }) {
  const tree = document.getElementById('file-list')!;
  const filter = document.getElementById('file-filter') as HTMLInputElement;
  const changesButton = document.getElementById('changes-filter') as HTMLButtonElement;
  const gitStatus = document.getElementById('git-status')!;
  const searchStatus = document.getElementById('file-search-status')!;
  let directories = new Map<string, ProjectEntry[]>(), expanded = new Set<string>();
  let root = '', current = '', unsaved = false, focusPath = '';
  let changes: GitSnapshot = { state: 'not_repository', branch: null, entries: [], error: null };
  let changesOnly = false, search: ProjectListing | null = null, searchPending = false, searchError = '';
  let searchGeneration = 0, searchTimer = 0, refreshing = false;
  const loading = new Map<string, Promise<void>>();
  let rows: TreeRow[] = [], typeahead = '', typeTimer = 0;
  const persist = () => { try { localStorage.setItem(`verum.studio.tree:${root}`, JSON.stringify([...expanded])); } catch { /* Session state remains usable. */ } };
  function element<K extends keyof HTMLElementTagNameMap>(tag: K, className = '', value = '') { const item = document.createElement(tag); item.className = className; item.textContent = value; return item; }
  function focused() { return tree.contains(document.activeElement) ? (document.activeElement as HTMLElement).closest<HTMLElement>('[data-path]')?.dataset.path : undefined; }
  function focus(path: string) {
    focusPath = path;
    for (const row of tree.querySelectorAll<HTMLElement>('[role="treeitem"]')) {
      row.tabIndex = row.dataset.path === path ? 0 : -1;
      if (row.tabIndex === 0) { row.focus({ preventScroll: true }); row.scrollIntoView({ block: 'nearest' }); }
    }
  }
  function render() {
    const oldFocus = focused();
    const changeMap = new Map(changes.entries.map(change => [change.path, change]));
    const counts = changedDirectories(changes.entries);
    const query = filter.value.trim().toLowerCase();
    const projection = changesOnly
      ? projectedTree(changes.entries.filter(change => change.path.toLowerCase().includes(query)).map(change => ({ path: change.path, kind: 'file', size: 0 })))
      : query ? projectedTree(search?.entries ?? []) : directories;
    rows = visibleTree(projection, expanded, changesOnly || !!query);
    if (!rows.some(row => row.path === focusPath)) focusPath = rows.find(row => row.path === current)?.path ?? rows[0]?.path ?? '';
    tree.replaceChildren();
    for (const row of rows) {
      const change = changeMap.get(row.path), directory = row.kind === 'directory';
      const item = element('div', 'tree-entry'); item.dataset.path = row.path;
      item.setAttribute('role', 'treeitem'); item.setAttribute('aria-level', String(row.depth)); item.setAttribute('aria-posinset', String(row.position)); item.setAttribute('aria-setsize', String(row.siblings));
      item.tabIndex = row.path === focusPath ? 0 : -1;
      item.style.setProperty('--depth', String(row.depth - 1)); item.title = row.path;
      item.setAttribute('aria-selected', String(row.path === current));
      const chevron = element('span', 'tree-chevron'); if (directory) chevron.append(icon('chevronRight')); chevron.setAttribute('aria-hidden', 'true');
      if (directory) { item.setAttribute('aria-expanded', String(changesOnly || !!query || expanded.has(row.path))); if (loading.has(row.path)) item.setAttribute('aria-busy', 'true'); }
      const glyph = icon(directory ? expanded.has(row.path) || changesOnly || !!query ? 'folderOpen' : 'folder' : row.path.endsWith('.vr') ? 'source' : 'file', directory ? 'folder-icon' : 'tree-file-icon');
      item.append(chevron, glyph, element('span', 'tree-name', fileName(row.path)));
      if (row.path === current && unsaved) { const dot = element('span', 'draft-dot', '●'); setAttribute(dot, 'aria-label', m('Unsaved changes')); item.append(dot); }
      if (change) {
        const badge = element('button', 'git-badge', `${change.index}${change.worktree}`.replaceAll(' ', '·')); badge.type = 'button'; badge.tabIndex = -1;
        badge.dataset.conflict = String(change.index === 'U' || change.worktree === 'U' || ['AA', 'DD'].includes(change.index + change.worktree));
        setAttribute(badge, 'title', m('Review changes for {path} · Alt+Enter', { path: row.path })); setAttribute(badge, 'aria-label', m('Review changes for {path} · Alt+Enter', { path: row.path }));
        badge.addEventListener('click', event => { event.stopPropagation(); hooks.review(change).catch(hooks.failure); }); item.append(badge);
      } else if (counts.has(row.path)) { const count = element('span', 'tree-change-count', String(counts.get(row.path))); setAttribute(count, 'title', m('{count} changed files', { count: counts.get(row.path)! })); item.append(count); }
      item.addEventListener('click', () => { focusPath = row.path; activate(row).catch(hooks.failure); });
      item.addEventListener('focus', () => { focusPath = row.path; });
      tree.append(item);
    }
    const status = searchError || (searchPending && !changesOnly ? t(m('Searching project…')) : search?.truncated && query && !changesOnly ? t(m('Search limit reached. Narrow the query.')) : !rows.length ? t(m(changesOnly ? 'No visible changed files.' : query ? 'No matching files.' : 'This folder is empty.')) : '');
    setText(searchStatus, status); searchStatus.hidden = !status;
    setText(gitStatus, changes.state === 'ready' ? changes.branch ?? m('Git repository') : m(changes.state === 'not_repository' ? 'No Git repository' : 'Git status unavailable'));
    gitStatus.title = changes.error ?? changes.branch ?? '';
    setText(document.getElementById('changes-count')!, String(changes.entries.length));
    changesButton.disabled = changes.state !== 'ready'; changesButton.setAttribute('aria-pressed', String(changesOnly));
    setText(document.getElementById('files-rail-status')!, changes.entries.length ? `${changes.entries.length} Δ` : '');
    if (oldFocus && rows.some(row => row.path === oldFocus)) focus(oldFocus);
  }
  async function load(path: string, force = false) {
    if (loading.has(path)) return loading.get(path);
    if (!force && directories.has(path)) return;
    const promise = api<ProjectListing>('workspace.browse', { path }).then(result => {
      directories.set(path, result.entries); hooks.listed(result.root, result.entries, path);
      if (!root) {
        root = result.root;
        try { const saved = JSON.parse(localStorage.getItem(`verum.studio.tree:${root}`) || '[]'); if (Array.isArray(saved)) expanded = new Set(saved.filter(path => typeof path === 'string').slice(0, 128)); } catch { /* Invalid preferences do not block navigation. */ }
      }
    }).finally(() => { loading.delete(path); render(); });
    loading.set(path, promise); return promise;
  }
  async function activate(row: TreeRow) {
    if (row.kind === 'directory') {
      if (changesOnly || filter.value.trim()) return;
      if (expanded.has(row.path)) expanded.delete(row.path);
      else { expanded.add(row.path); await load(row.path); }
      persist(); render(); focus(row.path);
    } else {
      const change = changes.entries.find(change => change.path === row.path);
      if (change?.worktree === 'D' || (change?.index === 'D' && change.worktree === ' ')) await hooks.review(change);
      else await hooks.open(row.path);
    }
  }
  async function searchFiles() {
    const query = filter.value.trim(), generation = ++searchGeneration;
    if (!query || changesOnly) search = null;
    searchError = ''; searchPending = !!query; render();
    if (!query || changesOnly) { searchPending = false; return; }
    try { const result = await api<ProjectListing>('workspace.search', { query }); if (generation === searchGeneration) search = result; }
    catch (error) { if (generation === searchGeneration) searchError = error instanceof Error ? error.message : String(error); }
    finally { if (generation === searchGeneration) { searchPending = false; render(); } }
  }
  async function refresh() {
    if (refreshing) return; refreshing = true;
    try {
      await load('', true);
      // Only expanded folders are read. Large closed subtrees cost no traversal.
      for (const path of [...expanded].sort()) {
        if (parentPath(path) && !expanded.has(parentPath(path))) continue;
        if (!directories.get(parentPath(path))?.some(entry => entry.path === path)) { expanded.delete(path); directories.delete(path); continue; }
        await load(path, true);
      }
      changes = await api<GitSnapshot>('workspace.changes');
      if (changes.state !== 'ready') changesOnly = false;
      await searchFiles(); render();
    } finally { refreshing = false; }
  }
  filter.addEventListener('input', () => { ++searchGeneration; window.clearTimeout(searchTimer); search = null; searchPending = !!filter.value.trim(); render(); searchTimer = window.setTimeout(() => { searchFiles().catch(hooks.failure); }, 180); });
  changesButton.addEventListener('click', () => { changesOnly = !changesOnly; searchFiles().catch(hooks.failure); });
  document.getElementById('collapse-folders')!.addEventListener('click', () => { expanded.clear(); changesOnly = false; filter.value = ''; persist(); searchFiles().catch(hooks.failure); });
  tree.addEventListener('keydown', event => {
    if (!(event.target instanceof HTMLElement) || event.target.getAttribute('role') !== 'treeitem') return;
    const path = event.target.getAttribute('data-path');
    const index = rows.findIndex(row => row.path === path), row = rows[index]; if (!row) return;
    let next = index;
    switch (event.key) {
      case 'ArrowDown': next = Math.min(rows.length - 1, index + 1); break;
      case 'ArrowUp': next = Math.max(0, index - 1); break;
      case 'Home': next = 0; break;
      case 'End': next = rows.length - 1; break;
      case 'ArrowRight': if (row.kind === 'directory' && !expanded.has(row.path) && !changesOnly && !filter.value.trim()) { activate(row).catch(hooks.failure); } else if (rows[index + 1]?.depth > row.depth) next++; break;
      case 'ArrowLeft': if (row.kind === 'directory' && expanded.has(row.path) && !changesOnly && !filter.value.trim()) { activate(row).catch(hooks.failure); } else { const parent = rows.findIndex(entry => entry.path === parentPath(row.path)); if (parent >= 0) next = parent; } break;
      case 'Enter': case ' ': {
        const change = changes.entries.find(change => change.path === row.path);
        if (event.altKey && change) hooks.review(change).catch(hooks.failure); else activate(row).catch(hooks.failure);
        break;
      }
      default:
        if (event.key.length !== 1 || event.ctrlKey || event.metaKey || event.altKey) return;
        typeahead += event.key.toLowerCase(); window.clearTimeout(typeTimer); typeTimer = window.setTimeout(() => { typeahead = ''; }, 700);
        for (let offset = 1; offset <= rows.length; offset++) { const candidate = (index + offset) % rows.length; if (fileName(rows[candidate].path).toLowerCase().startsWith(typeahead)) { next = candidate; break; } }
    }
    event.preventDefault(); focus(rows[next].path);
  });
  document.addEventListener('studio:language', render);
  return {
    refresh,
    current(path: string, dirty: boolean) { if (path === current && dirty === unsaved) return; current = path; unsaved = dirty; render(); },
    async reveal(path: string) { const ancestors: string[] = []; for (let parent = parentPath(path); parent; parent = parentPath(parent)) ancestors.unshift(parent); for (const parent of ancestors) { expanded.add(parent); await load(parent); } persist(); render(); },
  };
}
