import './style.css';
import { makeEditor } from './editor';
import { installPanels } from './panels';
import type { Pane } from './layout';
import { languageRequest, sameRevision, type DocumentRevision, type Analysis, type Range } from './language';
import { initializeLanguage, setLocale, currentLocale, m, t, setText, setAttribute, type Content, type Locale } from './i18n';
import { api, hasToken, installToken } from './api';
import { STARTER, IDENTITY, sameDocumentRevision, isStale, mergeReceipts, knownReceiptVersions, safeAuthUrl, relativeVerumPath, approvalKind, approvalReply, parseArguments, type Receipt } from './model';

initializeLanguage();
const $ = <T extends HTMLElement = HTMLElement>(id: string) => {
  const node = document.getElementById(id);
  if (!node) throw new Error(`Missing interface element: ${id}`);
  return node as T;
};
const input = (id: string) => $<HTMLInputElement>(id);
function node<K extends keyof HTMLElementTagNameMap>(tag: K, className = '', text: Content = ''): HTMLElementTagNameMap[K] {
  const element = document.createElement(tag); element.className = className; setText(element, text); return element;
}
const pretty = (value: unknown) => JSON.stringify(value, null, 2);
function errorText(error: unknown) { return error instanceof Error ? error.message : String(error); }
function notice(message: Content) { setText($('notice-text'), message); $('notice').hidden = false; }
function announce(message: Content) { setText($('announcer'), message); }
function fail(error: unknown) { notice(errorText(error)); }
function bind(id: string, action: () => unknown | Promise<unknown>) {
  $(id).addEventListener('click', () => { Promise.resolve().then(action).catch(fail); });
}
function button(label: Content, action: () => unknown | Promise<unknown>, className = 'button') {
  const element = node('button', className, label); element.type = 'button';
  element.addEventListener('click', () => { Promise.resolve().then(action).catch(fail); }); return element;
}
function setPane(pane: Pane) { panels.show(pane); }
bind('notice-close', () => { $('notice').hidden = true; });

const themeChoice = localStorage.getItem('verum.studio.theme') || 'dark';
function applyTheme(choice: string) {
  if (!['dark', 'light', 'system'].includes(choice)) choice = 'dark';
  const dark = choice === 'dark' || (choice === 'system' && matchMedia('(prefers-color-scheme: dark)').matches);
  document.documentElement.dataset.theme = dark ? 'dark' : 'light';
  localStorage.setItem('verum.studio.theme', choice);
  setAttribute($('theme'), 'title', m('Theme: {theme}. Change color theme', { theme: m(choice as 'dark' | 'light' | 'system') }));
  setAttribute($('theme'), 'aria-label', m('Color theme: {theme}. Change theme', { theme: m(choice as 'dark' | 'light' | 'system') }));
  return dark;
}
applyTheme(themeChoice);
matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => {
  if (localStorage.getItem('verum.studio.theme') === 'system') editor.theme(applyTheme('system'));
});

type FileRecord = { path: string; hash?: string };
type SourceFile = { path: string; source: string; hash: string };
let files: FileRecord[] = [];
let currentPath = 'experiment.vr';
let savedSource: string | null = null;
let savedHash: string | null = null;
let currentHash: string | null = null;
let hashGeneration = 0;
let hashTimer = 0;
let receipts: Receipt[] = [];
let selectedReceipt: string | null = null;
let isSaving = false;
let switching = false;
let editVersion = 0;
const documentRevision = () => ({ path: currentPath, editVersion });
const editor = makeEditor($('editor'), STARTER, () => { editVersion++; updateDocumentState(); scheduleHash(); scheduleAnalysis(); }, (line, column) => {
  setText($('cursor-position'), m('Ln {line}, Col {column}', { line, column }));
});
const panels = installPanels(() => editor.view.requestMeasure());
document.querySelector('.skip-link')?.addEventListener('click', event => { event.preventDefault(); setPane('editor'); editor.view.focus(); });
const scrolling = new WeakMap<HTMLElement, number>();
document.addEventListener('scroll', event => {
  if (!(event.target instanceof HTMLElement)) return;
  const target = event.target;
  window.clearTimeout(scrolling.get(target));
  target.dataset.scrolling = '';
  scrolling.set(target, window.setTimeout(() => { delete target.dataset.scrolling; scrolling.delete(target); }, 900));
}, { capture: true, passive: true });
function dirty() { return savedSource === null || editor.source() !== savedSource; }
function updateDocumentState() {
  setText($('current-path'), currentPath);
  setText($('agent-context'), currentPath);
  $('dirty-marker').hidden = !dirty();
  $<HTMLButtonElement>('save').disabled = isSaving || !dirty();
  $<HTMLButtonElement>('reload-source').disabled = savedHash === null || isSaving;
  setText($('source-status'), currentHash ? m('{state} · {hash}', { state: m(dirty() ? 'Unsaved' : 'Saved'), hash: currentHash.slice(0, 10) }) : m('Calculating source revision…'));
  setAttribute($('source-status'), 'title', currentHash ? m('SHA-256 {hash}', { hash: currentHash }) : m('Source hashing in progress'));
  document.title = `${dirty() ? '• ' : ''}${currentPath.split('/').pop()} · Verum Studio`;
}
async function sha256(source: string) {
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(source));
  return [...new Uint8Array(digest)].map(byte => byte.toString(16).padStart(2, '0')).join('');
}
function scheduleHash() {
  const generation = ++hashGeneration;
  currentHash = null; updateDocumentState(); renderResults(); window.clearTimeout(hashTimer);
  hashTimer = window.setTimeout(() => {
    const source = editor.source();
    sha256(source).then(hash => {
      if (generation !== hashGeneration) return;
      currentHash = hash; updateDocumentState(); renderResults();
    }).catch(fail);
  }, 120);
}
function loadDocument(path: string, source: string, hash: string | null) {
  editVersion++;
  currentPath = path; savedSource = hash === null ? null : source; savedHash = hash;
  editor.replace(source); scheduleHash(); scheduleAnalysis(); updateDocumentState(); renderFiles(); setPane('editor'); sourceView(); editor.view.focus();
}
async function refreshFiles() {
  const result = await api<{ root: string; files: FileRecord[] }>('workspace.list');
  files = result.files.filter(file => file.path.endsWith('.vr')).sort((a, b) => a.path.localeCompare(b.path));
  setText($('workspace-root'), result.root); setAttribute($('workspace-root'), 'title', result.root);
  $('connection').classList.add('online'); $('connection').replaceChildren(node('span', 'status-dot'), node('span', '', m('Local service')));
  renderFiles();
}
function renderFiles() {
  setText($('files-rail-status'), String(files.length));
  setAttribute($('files-restore'), 'title', m('Expand explorer · {count} Verum files', { count: files.length }));
  const filter = input('file-filter').value.toLowerCase();
  const list = $('file-list'); list.replaceChildren();
  const visible = files.filter(file => file.path.toLowerCase().includes(filter));
  if (!visible.length) { list.append(node('p', 'muted empty-note', filter ? m('No matching Verum files.') : m('No Verum files yet. Start an experiment below.'))); return; }
  for (const file of visible) {
    const entry = button('', () => openFile(file.path), 'file-entry');
    entry.setAttribute('aria-current', String(file.path === currentPath)); setAttribute(entry, 'title', file.path);
    entry.append(node('span', 'file-icon', 'vr'), node('span', '', file.path));
    const wrapper = node('div'); wrapper.setAttribute('role', 'listitem'); wrapper.append(entry); list.append(wrapper);
  }
}
type Choice = { value: string; label: Content; primary?: boolean };
function dialog(title: Content, content: HTMLElement, choices: Choice[]): Promise<string> {
  const modal = $<HTMLDialogElement>('dialog');
  if (modal.open) return Promise.resolve('cancel');
  setText($('dialog-title'), title); $('dialog-body').replaceChildren(content); $('dialog-actions').replaceChildren();
  for (const choice of choices) {
    const item = node('button', `button${choice.primary ? ' primary' : ''}`, choice.label);
    item.value = choice.value; item.type = 'submit'; $('dialog-actions').append(item);
  }
  modal.returnValue = 'cancel'; modal.showModal();
  return new Promise(resolve => modal.addEventListener('close', () => resolve(modal.returnValue || 'cancel'), { once: true }));
}
async function mayReplace() {
  if (!dirty()) return true;
  const choice = await dialog(m('Keep your current experiment?'), node('p', '', m('{path} has unsaved changes. Save them before opening another source, or discard them explicitly.', { path: currentPath })), [
    { value: 'cancel', label: m('Cancel') }, { value: 'discard', label: m('Discard changes') }, { value: 'save', label: m('Save and continue'), primary: true },
  ]);
  if (choice === 'save') { await save(); return !dirty(); }
  return choice === 'discard';
}
async function openFile(path: string) {
  if (path === currentPath && savedHash !== null) { setPane('editor'); return; }
  if (switching) return; switching = true;
  try { if (await mayReplace()) { const file = await api<SourceFile>('workspace.read', { path }); loadDocument(file.path, file.source, file.hash); } }
  finally { switching = false; }
}
async function save() {
  if (isSaving || !dirty()) return;
  const source = editor.source(), path = currentPath, expected_hash = savedHash;
  isSaving = true; updateDocumentState();
  try {
    const result = await api<SourceFile>('workspace.write', { path, source, expected_hash });
    // Saving is versioned: typing while the request is in flight stays unsaved.
    if (path === currentPath) { savedSource = result.source; savedHash = result.hash; updateDocumentState(); }
    await refreshFiles(); announce(m('Saved {path}', { path }));
  } finally { isSaving = false; updateDocumentState(); }
}
async function reloadSource() {
  if (switching || savedHash === null) return; switching = true;
  try {
    const path = currentPath;
    const file = await api<SourceFile>('workspace.read', { path });
    if (dirty()) {
      const content = node('div', 'source-review');
      content.append(node('p', '', m('Compare your draft with the saved source. Replacing the editor discards this draft; copy it first if you want to preserve both versions.')), button(m('Copy editor draft'), async () => { await navigator.clipboard.writeText(editor.source()); announce(m('Editor draft copied')); }, 'quiet-button'));
      content.append(node('span', 'approval-source-label', m('Current editor draft')), node('pre', '', editor.source()), node('span', 'approval-source-label', m('Saved file')), node('pre', '', file.source));
      if (await dialog(m('Review saved source'), content, [{ value: 'cancel', label: m('Keep editor draft') }, { value: 'reload', label: m('Replace with saved source'), primary: true }]) !== 'reload') return;
    }
    loadDocument(path, file.source, file.hash);
  } finally { switching = false; }
}
async function newDocument(source = '', suggestion = 'experiment.vr') {
  if (switching) return; switching = true;
  try {
    if (!(await mayReplace())) return;
    const content = node('div'); content.append(node('p', '', m('Create a Verum source file in this workspace. Existing directories are supported.')));
    const label = node('label', '', m('File path')); const field = node('input'); field.value = suggestion; field.autocomplete = 'off'; field.spellcheck = false;
    label.append(field); content.append(label);
    if (await dialog(m('New experiment'), content, [{ value: 'cancel', label: m('Cancel') }, { value: 'create', label: m('Create draft'), primary: true }]) !== 'create') return;
    const path = relativeVerumPath(field.value);
    if (!path) throw new Error(t(m('Use a relative .vr path without hidden traversal, for example experiment.vr.')));
    if (files.some(file => file.path === path)) throw new Error(t(m('{path} already exists. Open it from the explorer or choose a different name.', { path })));
    loadDocument(path, source, null);
    if (source === IDENTITY) { input('declaration').value = 'identity'; $('execution-settings').hidden = false; $('run-settings').setAttribute('aria-expanded', 'true'); }
  } finally { switching = false; }
}
bind('save', save); bind('refresh-files', refreshFiles); bind('new-file', () => newDocument());
let analysisGeneration = 0;
let analysisTimer = 0, analysisBusy = false, analysisPending = false;
const languageRevision = (): DocumentRevision => ({ path: currentPath, source: editor.source() });
function sourceView() {
  $('architecture').hidden = true; $('editor').hidden = false;
  setText($('architecture-toggle'), m('Structure')); $('architecture-toggle').setAttribute('aria-expanded', 'false');
  editor.view.requestMeasure();
}
function showRange(range: Range) { sourceView(); setPane('editor'); editor.select(range); }
async function navigateDefinition(location: { uri: string; range: Range }) {
  const url = new URL(location.uri), root = $('workspace-root').textContent || '';
  const path = decodeURIComponent(url.pathname);
  if (url.protocol !== 'file:' || !path.startsWith(root + '/')) throw new Error(t(m('Definition is outside this workspace.')));
  const relative = path.slice(root.length + 1);
  if (relative !== currentPath) await openFile(relative);
  if (relative === currentPath) showRange(location.range);
}
editor.connectLanguage({ revision: languageRevision, failure: fail, navigate: navigateDefinition });
bind('architecture-toggle', async () => {
  if (!$('architecture').hidden) { sourceView(); return; }
  $('architecture').hidden = false; $('editor').hidden = true;
  setText($('architecture-toggle'), m('Source')); $('architecture-toggle').setAttribute('aria-expanded', 'true');
  await inspectProject();
});
async function inspectProject() {
  const project = await api<{ workspace: { root: string; files: FileRecord[] }; analysis: { sources?: Array<{ uri: string; source_hash: string }>; issues?: string[]; documents: Array<Analysis & { uri: string }> } }>('project.inspect');
  const list = $('project-content'); list.replaceChildren();
  for (const issue of project.analysis.issues ?? []) list.append(node('p', 'muted', issue));
  for (const file of project.workspace.files) {
    const snapshot = project.analysis.documents.find(document => decodeURIComponent(new URL(document.uri).pathname) === `${project.workspace.root}/${file.path}`);
    const indexed = project.analysis.sources?.find(source => decodeURIComponent(new URL(source.uri).pathname) === `${project.workspace.root}/${file.path}`);
    const entry = button('', () => openFile(file.path), 'project-file');
    entry.append(node('span', '', file.path), node('span', 'muted', snapshot ? snapshot.source_hash.slice(0, 10) : indexed ? m('Indexed source') : m('Not analyzed')));
    list.append(entry);
  }
}
bind('project-refresh', async () => { scheduleAnalysis(); await inspectProject(); });
window.addEventListener('focus', scheduleAnalysis);
function resultView(diagnostics: boolean) {
  $('operations-panel').hidden = diagnostics; $('diagnostics-panel').hidden = !diagnostics;
  $('operations-tab').setAttribute('aria-selected', String(!diagnostics)); $('diagnostics-tab').setAttribute('aria-selected', String(diagnostics));
}
bind('operations-tab', () => resultView(false)); bind('diagnostics-tab', () => resultView(true));
bind('language-status', () => { resultView(true); setPane('results'); });
function scheduleAnalysis() {
  ++analysisGeneration;
  window.clearTimeout(analysisTimer);
  editor.diagnostics(editor.source(), []);
  setText($('language-status'), m('Analyzing…'));
  setText($('diagnostics-count'), '…');
  $('diagnostics-list').replaceChildren(node('p', 'muted', m('Waiting for current source analysis…')));
  $('architecture-content').replaceChildren(node('p', 'muted', m('Waiting for current source analysis…')));
  analysisPending = true; analysisTimer = window.setTimeout(analyzeSource, 350);
}
async function analyzeSource() {
  if (analysisBusy || !hasToken()) return;
  analysisBusy = true; analysisPending = false;
  const revision = languageRevision(), generation = analysisGeneration;
  try {
    const reply = await languageRequest<Analysis>(revision);
    if (generation !== analysisGeneration || !sameRevision(revision, languageRevision())) return;
    renderAnalysis(reply.result);
  } catch (error) {
    if (generation === analysisGeneration && sameRevision(revision, languageRevision())) {
      setText($('language-status'), m('Analysis unavailable'));
      const message = m('Analysis failed: {error}', { error: errorText(error) });
      $('diagnostics-list').replaceChildren(node('p', 'muted', message));
      $('architecture-content').replaceChildren(node('p', 'muted', message));
    }
  } finally { analysisBusy = false; if (analysisPending) analysisTimer = window.setTimeout(analyzeSource, 100); }
}
function renderAnalysis(snapshot: Analysis) {
  editor.diagnostics(editor.source(), snapshot.diagnostics);
  const count = snapshot.diagnostics.length;
  setText($('language-status'), count ? m('{count} diagnostics', { count }) : m('No module diagnostics'));
  setText($('diagnostics-count'), String(count));
  setText($('analysis-scope'), snapshot.project_revision ? m('Project source analysis with resolved imports. Build and proof checks remain separate.') : m('Single-module analysis with builtins. Imported modules and whole-project contracts require a project check.'));
  const diagnostics = $('diagnostics-list'); diagnostics.replaceChildren();
  for (const diagnostic of snapshot.diagnostics) {
    const entry = button('', () => showRange(diagnostic.range), 'diagnostic-item');
    entry.append(node('span', `diagnostic-marker severity-${diagnostic.severity}`, diagnostic.severity === 1 ? '●' : '◇'), node('span', 'diagnostic-message', diagnostic.message), node('span', 'muted', `${diagnostic.range.start.line + 1}:${diagnostic.range.start.character + 1}`));
    diagnostics.append(entry);
  }
  if (!count) diagnostics.append(node('p', 'muted', m('No module diagnostics')));
  const structure = $('architecture-content'); structure.replaceChildren();
  structure.append(node('p', 'result-meta', `${currentPath} · ${snapshot.source_hash.slice(0, 12)}${snapshot.project_revision ? ` · ${snapshot.project_revision.slice(0, 12)}` : ''}`));
  if (!snapshot.declarations.length) structure.append(node('p', 'muted', m('No declarations available for this revision.')));
  for (const declaration of snapshot.declarations) {
    const card = node('article', 'declaration-card');
    const heading = button(declaration.name, () => showRange(declaration.selection_range), 'declaration-name');
    card.append(node('span', 'badge', declaration.kind), heading);
    if (declaration.type_display) card.append(node('code', 'muted', declaration.type_display));
    if (declaration.requirements.length) {
      card.append(node('p', 'muted', m('Context requirements')));
      for (const requirement of declaration.requirements) card.append(button(requirement.source, () => showRange(requirement.range), 'context-requirement'));
    }
    structure.append(card);
  }
  if (snapshot.bindings?.length) {
    structure.append(node('h3', '', m('Resolved imports')));
    for (const binding of snapshot.bindings) {
      const definition = binding.definition;
      const entry = definition
        ? button(binding.name, () => navigateDefinition(definition), 'context-requirement')
        : node('span', 'context-requirement', binding.name);
      entry.title = `${binding.type_display} · ${definition?.uri ?? binding.source_module ?? ''}`;
      structure.append(entry);
    }
  }
  if (snapshot.mounts.length) {
    structure.append(node('h3', '', m('Mounts')));
    for (const mount of snapshot.mounts) structure.append(button(mount.source, () => showRange(mount.range), 'context-requirement'));
  }
}
bind('reload-source', reloadSource);
bind('lesson-script', () => newDocument(STARTER, 'first-experiment.vr'));
bind('lesson-proof', () => newDocument(IDENTITY, 'identity.vr'));
$<HTMLSelectElement>('language').value = currentLocale();
$('language').addEventListener('change', () => { setLocale($<HTMLSelectElement>('language').value as Locale); editor.language(); editor.view.requestMeasure(); });
input('file-filter').addEventListener('input', renderFiles);
bind('run-settings', () => { $('execution-settings').hidden = !$('execution-settings').hidden; $('run-settings').setAttribute('aria-expanded', String(!$('execution-settings').hidden)); });
function closeSettings() { $('execution-settings').hidden = true; $('run-settings').setAttribute('aria-expanded', 'false'); }
document.addEventListener('pointerdown', event => {
  if (event.target instanceof Node && !$('execution-settings').contains(event.target) && !$('run-settings').contains(event.target)) closeSettings();
});
document.addEventListener('keydown', event => {
  if (event.key === 'Escape' && !$('execution-settings').hidden) { event.preventDefault(); closeSettings(); $('run-settings').focus(); }
});
window.addEventListener('beforeunload', event => { if (dirty()) { event.preventDefault(); event.returnValue = ''; } });

let starting = false;
async function startOperation(kind: Receipt['kind']) {
  if (starting) return;
  const source = editor.source(), path = currentPath;
  const seconds = Number(input('timeout').value);
  if (!Number.isFinite(seconds) || seconds < 1 || seconds > 300) throw new Error(t(m('Choose a time budget between 1 and 300 seconds.')));
  const declaration = input('declaration').value.trim();
  if (kind === 'proof' && !declaration) { $('execution-settings').hidden = false; input('declaration').focus(); throw new Error(t(m('Enter the logical declaration to check.'))); }
  starting = true; ['run', 'check', 'prove'].forEach(id => { $<HTMLButtonElement>(id).disabled = true; });
  try {
    const result = await api<{ id: string }>('operation.start', { kind, source, path, ...(kind === 'proof' ? { declaration } : {}), timeout_ms: Math.round(seconds * 1000) });
    selectedReceipt = result.id; await refreshOperations(); resultView(false); setPane('results'); announce(m('{kind} started for {path}', { kind: resultLabel(kind), path }));
  } finally { starting = false; ['run', 'check', 'prove'].forEach(id => { $<HTMLButtonElement>(id).disabled = false; }); }
}
bind('run', () => startOperation('run')); bind('check', () => startOperation('check')); bind('prove', () => startOperation('proof'));
let operationsBusy = false;
async function refreshOperations() {
  if (operationsBusy) return; operationsBusy = true;
  try {
    const previous = new Map(receipts.map(receipt => [receipt.id, receipt.status]));
    const known = knownReceiptVersions(receipts);
    const changes = await api<Receipt[]>('operation.list', known ? { known } : {});
    receipts = mergeReceipts(receipts, changes);
    for (const receipt of receipts) if (previous.get(receipt.id) === 'running' && receipt.status !== 'running') announce(m('{kind} {status}: {path}', { kind: resultLabel(receipt.kind), status: m(receipt.status), path: receipt.path }));
    if (!selectedReceipt && receipts.length) selectedReceipt = receipts[receipts.length - 1].id;
    renderResults();
  } finally { operationsBusy = false; }
}
const resultLabel = (kind: string) => m(kind === 'proof' ? 'Proof check' : kind === 'check' ? 'Module check' : 'Execution');
const elapsed = (receipt: Receipt) => receipt.status === 'running' && receipt.started_unix_ms ? Math.max(0, Date.now() - receipt.started_unix_ms) : receipt.elapsed_ms;
let detailSignature = '';
let historySignature = '';
function restoreFocusAfterRender(container: HTMLElement) {
  const active = document.activeElement;
  if (!(active instanceof HTMLElement) || !container.contains(active)) return () => {};
  const key = active.dataset.focusKey;
  return () => {
    const replacement = [...container.querySelectorAll<HTMLElement>('[data-focus-key]')].find(item => item.dataset.focusKey === key);
    if (replacement) replacement.focus({ preventScroll: true });
    else { container.tabIndex = -1; container.focus({ preventScroll: true }); }
  };
}
function renderResults() {
  setText($('result-count'), String(receipts.length)); setText($('mobile-result-count'), receipts.length ? String(receipts.length) : '');
  const running = receipts.filter(receipt => receipt.status === 'running').length;
  setText($('running-count'), running ? m('{count} running', { count: running }) : '');
  const nextHistory = JSON.stringify(receipts.map(receipt => ({ id: receipt.id, kind: receipt.kind, path: receipt.path, status: receipt.status, seconds: Math.floor(elapsed(receipt) / 1000) }))) + selectedReceipt;
  if (nextHistory !== historySignature) {
    const restoreFocus = restoreFocusAfterRender($('history'));
    historySignature = nextHistory; $('history').replaceChildren();
    for (const receipt of [...receipts].reverse()) {
    const entry = button('', () => { selectedReceipt = receipt.id; renderResults(); }, 'history-item');
    entry.dataset.focusKey = receipt.id;
    entry.setAttribute('aria-current', String(receipt.id === selectedReceipt));
    const row = node('span', 'history-row'); row.append(node('span', '', receipt.kind === 'proof' ? m('∀ Proof') : receipt.kind === 'check' ? m('✓ Check') : m('▶ Run')), node('span', `badge ${receipt.status}`, m(receipt.status)));
    entry.append(row, node('span', 'history-path', receipt.path), node('span', 'history-path', m('{seconds}s · {hash}', { seconds: Math.max(0, elapsed(receipt) / 1000), hash: receipt.source_hash.slice(0, 7) })));
    setAttribute(entry, 'title', m('{kind} · {path} · {status}', { kind: resultLabel(receipt.kind), path: receipt.path, status: m(receipt.status) }));
      $('history').append(entry);
    }
    restoreFocus();
  }
  const receipt = receipts.find(receipt => receipt.id === selectedReceipt);
  setText($('result-summary'), receipt ? m('{kind} · {status}', { kind: resultLabel(receipt.kind), status: m(receipt.status) }) : m('No operations yet'));
  if (!receipt) return;
  const nextDetail = JSON.stringify(receipt) + currentPath + currentHash + Math.floor(elapsed(receipt) / 1000);
  if (detailSignature === nextDetail) return;
  detailSignature = nextDetail;
  const detail = $('result-detail');
  // Preserve scroll and inspection expansion across receipt polls and edits.
  const oldId = detail.dataset.receipt;
  const expanded = new Set([...detail.querySelectorAll<HTMLDetailsElement>('details[open]')].map(item => item.dataset.key));
  const scroll = detail.scrollTop;
  const restoreFocus = restoreFocusAfterRender(detail);
  detail.replaceChildren(); detail.dataset.receipt = receipt.id;
  const heading = node('div', 'result-heading'); heading.append(node('strong', '', resultLabel(receipt.kind)), node('span', `badge ${receipt.status}`, m(receipt.status)));
  if (isStale(receipt, currentPath, currentHash)) {
    const badge = node('span', 'badge stale', currentHash === null && currentPath === receipt.path ? m('Checking revision') : m('Different revision'));
    setAttribute(badge, 'title', m('This result does not describe the current editor source.')); heading.append(badge);
  } else heading.append(node('span', 'badge', m('Current source')));
  if (receipt.status === 'running') {
    const cancel = button(m('Cancel'), async () => { await api('operation.cancel', { id: receipt.id }); await refreshOperations(); }, 'quiet-button');
    cancel.dataset.focusKey = 'cancel'; heading.append(cancel);
  }
  const copy = button(m('Copy receipt'), async () => { await navigator.clipboard.writeText(pretty(receipt)); announce(m('Receipt copied')); }, 'quiet-button');
  copy.dataset.focusKey = 'copy'; heading.append(copy);
  detail.append(heading, node('p', 'result-meta', receipt.exit_code == null ? m('{path} · {seconds}s · {hash}', { path: receipt.path, seconds: elapsed(receipt) / 1000, hash: receipt.source_hash.slice(0, 12) }) : m('{path} · {seconds}s · exit {code} · {hash}', { path: receipt.path, seconds: elapsed(receipt) / 1000, code: receipt.exit_code, hash: receipt.source_hash.slice(0, 12) })));
  if (receipt.timeout_ms) detail.append(node('p', 'muted', m(receipt.evidence_path ? 'Wall budget {seconds}s · source and receipt retained locally' : 'Wall budget {seconds}s', { seconds: receipt.timeout_ms / 1000 })));
  if (receipt.termination) detail.append(node('p', 'termination', receipt.termination));
  if (receipt.kind === 'proof') {
    const proof = node('div', 'proof-outcome');
    const report = parseArguments(receipt.proof), source = parseArguments(report.source), diagnostic = parseArguments(report.diagnostic);
    proof.append(node('h3', '', typeof report.status === 'string' ? m('Checker judgment: {status}', { status: report.status }) : m('Logical proof evidence')));
    if (typeof source.declaration === 'string') proof.append(node('p', '', m('Declaration: {declaration}', { declaration: source.declaration })));
    if (typeof diagnostic.message === 'string') proof.append(node('p', '', diagnostic.message));
    proof.append(node('p', '', m('This judgment applies to the selected logical declaration and source revision. Runtime implementation correspondence is a separate obligation.'))); detail.append(proof);
    if (receipt.proof !== undefined && receipt.proof !== null) appendDetails(detail, m('Full checker report'), receipt.proof, 'proof');
  } else if (receipt.kind === 'run') detail.append(node('p', 'muted', m('Observed execution for this source revision; not a proof of all behaviors.')));
  if (receipt.stdout) {
    if (receipt.kind === 'proof' && receipt.proof) {
      const stream = node('details'); stream.dataset.key = 'raw-proof'; const summary = node('summary', '', m('Raw checker output')); summary.dataset.focusKey = 'raw-proof'; stream.append(summary, node('pre', '', receipt.stdout)); detail.append(stream);
    } else appendStream(detail, m('Standard output'), receipt.stdout, 'stdout');
  }
  if (receipt.stderr) appendStream(detail, m('Diagnostics / standard error'), receipt.stderr, 'stderr');
  if (!receipt.stdout && !receipt.stderr) detail.append(node('p', 'muted', receipt.status === 'running' ? m('Waiting for process output…') : m('The process produced no text output.')));
  appendDetails(detail, m('Full operation receipt'), receipt, 'receipt');
  if (oldId === receipt.id) { detail.querySelectorAll<HTMLDetailsElement>('details').forEach(item => { item.open = expanded.has(item.dataset.key); }); detail.scrollTop = scroll; }
  restoreFocus();
}
function appendStream(parent: HTMLElement, label: Content, text: string, className: string) {
  const stream = node('div', `result-stream ${className}`); stream.append(node('span', 'stream-label', label), node('pre', '', text)); parent.append(stream);
}
function appendDetails(parent: HTMLElement, label: Content, value: unknown, key: string, open = false) {
  const details = node('details'); details.dataset.key = key; details.open = open;
  const summary = node('summary', '', label); summary.dataset.focusKey = key;
  details.append(summary, node('pre', '', pretty(value))); parent.append(details);
}

let connected = false;
let authenticated = false;
let threadId: string | null = null;
let turnId: string | null = null;
let sending = false;
let cursor = 0;
let eventBusy = false;
const messageBodies = new Map<string, HTMLElement>();
const approvalIds = new Set<string>();
let pendingApprovals = 0;
const eventLog: string[] = [];
let eventLogLength = 0;
let eventLogDropped = false;
const completedTurns = new Set<string>();
function addMessage(role: 'user' | 'agent' | 'system', text: Content, id?: string) {
  $('agent-welcome').hidden = true;
  const container = $('agent-messages');
  const nearBottom = container.scrollHeight - container.scrollTop - container.clientHeight < 90;
  const message = node('div', `message ${role}`); message.append(node('div', 'message-label', role === 'agent' ? m('Codex') : role === 'user' ? m('You') : m('Studio')));
  const body = node('div', 'message-body', text); message.append(body); container.append(message);
  if (id) messageBodies.set(id, body);
  if (nearBottom || role === 'user') container.scrollTop = container.scrollHeight;
  return body;
}
function agentState() {
  const busy = sending || turnId !== null;
  $<HTMLButtonElement>('agent-send').disabled = !connected || !authenticated || busy;
  $<HTMLTextAreaElement>('agent-input').disabled = !connected || !authenticated;
  $<HTMLButtonElement>('new-thread').disabled = !connected || !authenticated || busy || pendingApprovals > 0;
  $<HTMLSelectElement>('agent-model').disabled = threadId !== null;
  setAttribute($('agent-model'), 'title', threadId ? m('Model is fixed for this conversation. Start a new conversation to choose another model.') : m('Choose the model for a new conversation'));
  $('agent-stop').hidden = !busy;
  setText($('agent-progress'), pendingApprovals ? m('Awaiting review: {count}', { count: pendingApprovals }) : busy ? m('Codex is working…') : connected ? authenticated ? m('Connected · edits and tools require review') : m('Local provider ready · sign in to start a conversation') : '');
  setText($('approval-count'), pendingApprovals ? String(pendingApprovals) : '');
  setText($('agent-rail-status'), pendingApprovals ? m('{count} review', { count: pendingApprovals }) : busy ? m('Working') : authenticated ? m('Ready') : connected ? m('Sign in') : m('Offline'));
  $('agent-rail').dataset.attention = String(pendingApprovals > 0);
}
function accountLabel(value: unknown): Content {
  const record = parseArguments(value);
  const account = parseArguments(record.account ?? record);
  const email = typeof account.email === 'string' ? account.email : '';
  const plan = typeof account.planType === 'string' ? account.planType : '';
  if (email || plan) return [email, plan].filter(Boolean).join(' · ');
  return record.requiresOpenaiAuth === true || record.account === null ? m('Sign in to your Codex account') : m('Local Codex session connected');
}
async function connectAgent() {
  const control = $<HTMLButtonElement>('agent-connect'); control.disabled = true; setText(control, m('Connecting…'));
  try {
    const account = await api('agent.connect'); connected = true;
    const data = parseArguments(account); authenticated = data.account != null || data.requiresOpenaiAuth === false;
    setText($('agent-account'), accountLabel(account)); $('agent-controls').hidden = false;
    setText(control, m('Refresh')); agentState();
    await loadModels();
  } finally { control.disabled = false; if (!connected) setText(control, m('Connect')); }
}
async function loadModels() {
  try {
    const result = await api<{ data?: Array<{ id?: string; model?: string; displayName?: string; isDefault?: boolean }> }>('agent.request', { method: 'model/list', params: {} });
    const select = $<HTMLSelectElement>('agent-model'); const selected = select.value;
    const defaultOption = new Option('', ''); setText(defaultOption, m('Account default model')); select.replaceChildren(defaultOption);
    for (const model of result.data || []) { const id = model.id || model.model; if (id) select.add(new Option(model.displayName || id, id)); }
    select.value = selected;
  } catch (error) { addMessage('system', m('Model discovery unavailable: {error}', { error: errorText(error) })); }
}
async function loginAgent() {
  const result = await api<{ authUrl: string }>('agent.login');
  const url = safeAuthUrl(result.authUrl);
  if (!url) throw new Error(t(m('The agent returned an unsupported authentication URL.')));
  const content = node('div'); content.append(node('p', '', m('Continue in your provider’s secure sign-in page. Studio does not receive your password. Return here and refresh the connection after signing in.')));
  const link = node('a', 'button primary', m('Open Codex sign-in')); link.href = url; link.target = '_blank'; link.rel = 'noopener noreferrer'; content.append(link);
  await dialog(m('Connect your account'), content, [{ value: 'close', label: m('Done') }]);
}
async function newThread(internal = false) {
  if (turnId || (sending && !internal) || pendingApprovals) throw new Error(t(m('Finish or stop the current turn and resolve requests before starting another conversation.')));
  const model = $<HTMLSelectElement>('agent-model').value;
  const result = await api<{ thread?: { id: string }; id?: string }>('agent.request', { method: 'thread/start', params: model ? { model } : {} });
  const id = result.thread?.id || result.id; if (!id) throw new Error(t(m('Codex did not return a conversation ID.')));
  threadId = id; messageBodies.clear();
  $('agent-messages').querySelectorAll('.message,.approval').forEach(element => element.remove());
  $('agent-welcome').hidden = false; agentState(); announce(m('New conversation started'));
}
async function sendMessage() {
  const field = $<HTMLTextAreaElement>('agent-input'), text = field.value.trim();
  if (!text || sending || turnId) return;
  if (!authenticated) throw new Error(t(m('Sign in to your Codex account before starting a conversation.')));
  const source = editor.source(), path = currentPath, snapshotSavedHash = savedHash;
  sending = true; agentState(); field.value = '';
  try {
    if (!connected) await connectAgent();
    if (!threadId) await newThread(true);
    const hash = await sha256(source);
    addMessage('user', text);
    const model = $<HTMLSelectElement>('agent-model').value;
    const context = `${text}\n\nCurrent Verum editor source (may contain unsaved edits):\nPath: ${path}\nSource SHA-256: ${hash}\nSaved file SHA-256: ${snapshotSavedHash ?? '(new file; not saved)'}\n<verum_source>\n${source}\n</verum_source>`;
    const result = await api<{ turn?: { id: string; status?: string } }>('agent.request', { method: 'turn/start', params: { threadId, input: [{ type: 'text', text: context }], ...(model ? { model } : {}) } });
    if (result.turn?.id && !completedTurns.has(result.turn.id) && result.turn.status !== 'completed' && result.turn.status !== 'failed' && result.turn.status !== 'interrupted') turnId = result.turn.id;
  } catch (error) { addMessage('system', m('Message could not be sent: {error}', { error: errorText(error) })); if (!field.value) field.value = text; }
  finally { sending = false; agentState(); }
}
bind('agent-connect', connectAgent); bind('agent-login', loginAgent);
bind('new-thread', () => {
  if (turnId || sending || pendingApprovals) throw new Error(t(m('Finish the current turn and resolve requests before starting another conversation.')));
  threadId = null; messageBodies.clear();
  $('agent-messages').querySelectorAll('.message,.approval').forEach(element => element.remove());
  $('agent-welcome').hidden = false; agentState(); $('agent-input').focus();
});
bind('agent-suggestion', () => { $<HTMLTextAreaElement>('agent-input').value = t(m('Explain this Verum source and suggest one useful experiment. Use Verum tools to check your claims.')); if (connected) $('agent-input').focus(); else notice(m('Connect Codex to start this conversation.')); });
$('agent-form').addEventListener('submit', event => { event.preventDefault(); sendMessage().catch(fail); });
$('agent-input').addEventListener('keydown', event => { if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) { event.preventDefault(); sendMessage().catch(fail); } });
bind('agent-stop', async () => {
  if (!threadId || !turnId) throw new Error(t(m('The turn is still starting. Try Stop again when its identifier arrives.')));
  await api('agent.request', { method: 'turn/interrupt', params: { threadId, turnId } });
  setText($('agent-progress'), m('Stopping the current turn…'));
});

type RpcMessage = { id?: string | number; method?: string; params?: Record<string, unknown>; error?: unknown; result?: unknown };
async function pollEvents() {
  if (!connected || eventBusy) return; eventBusy = true;
  try {
    const result = await api<{ cursor: number; events: Array<{ seq: number; message: RpcMessage }>; gap: boolean }>('agent.events', { after: cursor });
    if (result.gap) addMessage('system', m('Some session events are no longer available. This conversation history is incomplete; inspect source and results before continuing.'));
    for (const event of result.events) { if (event.seq > cursor) await receiveEvent(event.message); }
    cursor = result.cursor;
    setText($('event-count'), String(eventLog.length));
    setText($('event-content'), `${eventLogDropped ? 'Recent event excerpts (older or large payloads omitted). Full pending requests appear in their review cards.\n\n' : ''}${eventLog.join('\n\n')}`);
  } finally { eventBusy = false; }
}
async function receiveEvent(message: RpcMessage) {
  const serialized = pretty(message);
  const excerpt = serialized.length > 4096 ? `${serialized.slice(0, 4096)}\n[Event payload excerpt]` : serialized;
  if (serialized.length > 4096) eventLogDropped = true;
  eventLog.push(excerpt); eventLogLength += excerpt.length;
  while (eventLog.length > 150 || eventLogLength > 65536) { eventLogLength -= eventLog.shift()!.length; eventLogDropped = true; }
  const method = message.method, params = message.params || {};
  if (message.id !== undefined && method) { await showApproval(message.id, method, params); return; }
  if (params.threadId && threadId && params.threadId !== threadId) return;
  if (method === 'item/agentMessage/delta') {
    const id = String(params.itemId || 'stream');
    const body = messageBodies.get(id) || addMessage('agent', '', id);
    const container = $('agent-messages'); const stick = container.scrollHeight - container.scrollTop - container.clientHeight < 90;
    body.textContent += typeof params.delta === 'string' ? params.delta : '';
    if (stick) container.scrollTop = container.scrollHeight;
  } else if (method === 'item/completed') {
    const item = parseArguments(params.item);
    if (item.type === 'agentMessage' && typeof item.text === 'string') {
      const id = String(item.id || 'stream'); const body = messageBodies.get(id) || addMessage('agent', '', id); setText(body, item.text);
    }
  } else if (method === 'turn/started') {
    const turn = parseArguments(params.turn); if (typeof turn.id === 'string') turnId = turn.id;
  } else if (method === 'turn/completed') {
    const turn = parseArguments(params.turn);
    if (typeof turn.id === 'string') completedTurns.add(turn.id);
    if (!turnId || !turn.id || turnId === turn.id) turnId = null;
    if (turn.status === 'failed' || turn.error) addMessage('system', m('Turn failed: {error}', { error: pretty(turn.error || turn.status) }));
    else if (turn.status === 'interrupted') addMessage('system', m('Turn interrupted. Completed evidence remains in Results.'));
  } else if (method === 'account/updated' || method === 'account/login/completed') {
    const account = await api('agent.request', { method: 'account/read', params: {} });
    const data = parseArguments(account); authenticated = data.account != null || data.requiresOpenaiAuth === false;
    setText($('agent-account'), accountLabel(account));
  } else if (method === 'error') addMessage('system', pretty(params));
  agentState();
}
async function showApproval(id: string | number, method: string, params: Record<string, unknown>) {
  const key = `${typeof id}:${id}`; if (approvalIds.has(key)) return; approvalIds.add(key);
  const kind = approvalKind(method, params), args = parseArguments(params.arguments);
  const card = node('section', 'approval'); card.setAttribute('aria-label', 'Agent request awaiting review');
  card.append(node('div', 'approval-heading', kind === 'source' ? m('Review a source proposal') : kind === 'execution' ? m('Review a Verum operation') : kind === 'read' ? params.tool === 'verum_analyze' ? m('Review language analysis') : m('Review a workspace read') : m('Unsupported agent request')));
  const content = node('div', 'approval-content');
  let reviewAvailable = true;
  content.append(node('p', '', kind === 'unsupported' ? m('This request is outside the Verum tool boundary. Decline it to let the agent continue.') : `${String(params.tool || method)}${typeof args.path === 'string' ? ` · ${args.path}` : ''}`));
  if (kind === 'source') {
    let before: Content = m('(A new file is proposed; the server will check that it does not already exist.)');
    if (typeof args.path === 'string' && args.expected_hash !== null) {
      try { const file = await api<SourceFile>('workspace.read', { path: args.path }); before = file.source; }
      catch (error) { before = m('Current source could not be read: {error}', { error: errorText(error) }); reviewAvailable = false; }
    }
    if (typeof args.source !== 'string' || typeof args.path !== 'string' || !('expected_hash' in args)) reviewAvailable = false;
    if (args.path === currentPath && dirty()) content.append(node('p', '', m('You have unsaved editor changes. Accepting writes the file on disk; your editor draft will be preserved.')));
    content.append(node('span', 'approval-source-label', m('Before · saved workspace source')), node('pre', '', before), node('span', 'approval-source-label', m('Proposed source')), node('pre', '', typeof args.source === 'string' ? args.source : m('(Missing source)')));
    content.append(node('p', '', m('Expected file revision: {revision}', { revision: args.expected_hash === null ? m('new file') : typeof args.expected_hash === 'string' ? args.expected_hash : m('not supplied') })));
  } else appendDetails(content, m('Request details'), params, 'request', kind === 'execution' || kind === 'read');
  const actions = node('div', 'approval-actions');
  const status = node('div', 'approval-status', m('Awaiting your decision. No requested action has been approved.'));
  let responding = false;
  async function respond(approved: boolean) {
    if (responding) return; responding = true;
    actions.querySelectorAll('button').forEach(button => { button.disabled = true; });
    setText(status, approved ? m('Approved · waiting for the tool result…') : m('Declining request…'));
    let result: unknown;
    try { result = await api('agent.respond', { id, result: approvalReply(method, approved) }); }
    catch (error) {
      responding = false; setText(status, m('Request response failed: {error}', { error: errorText(error) }));
      actions.querySelectorAll('button').forEach(button => { button.disabled = false; }); return;
    }
    try {
      const response = parseArguments(result);
      setText(status, !approved ? m('Request declined.') : response.success === true ? m('Tool completed. Its result is available below.') : response.success === false ? m('The tool reported a failure. Inspect the response below.') : m('Approval delivered. Inspect the tool result and evidence before treating the request as successful.'));
      if (result !== undefined && result !== null) appendDetails(content, m('Tool response'), result, 'response');
      pendingApprovals--; agentState();
      if (approved && kind === 'source' && typeof args.path === 'string') {
        await refreshFiles();
        if (args.path === currentPath && !dirty()) { const file = await api<SourceFile>('workspace.read', { path: args.path }); loadDocument(file.path, file.source, file.hash); }
      }
      await refreshOperations();
    } catch (error) { status.append(node('p', '', m('Refresh unavailable: {error}', { error: errorText(error) }))); }
  }
  actions.append(button(m('Decline'), () => respond(false)));
  if (kind !== 'unsupported' && reviewAvailable) actions.append(button(kind === 'source' ? m('Accept edit') : kind === 'read' ? params.tool === 'verum_analyze' ? m('Allow analysis') : m('Allow read') : m('Approve operation'), () => respond(true), 'button primary'));
  card.append(content, actions, status); $('agent-welcome').hidden = true; $('agent-messages').append(card); pendingApprovals++; agentState();
  announce(m('Agent request awaiting your review'));
}

bind('theme', async () => {
  const selected = await dialog(m('Appearance'), node('p', '', m('Choose a color scheme for your workspace.')), [{ value: 'system', label: m('Follow system') }, { value: 'light', label: m('Light') }, { value: 'dark', label: m('Dark'), primary: true }]);
  if (selected !== 'cancel') editor.theme(applyTheme(selected));
});
bind('help', async () => {
  const content = node('div', 'shortcut-list');
  for (const [label, shortcut] of [[m('Save source'), '⌘ / Ctrl S'], [m('Run source'), '⌘ / Ctrl Enter'], [m('Check source'), '⌘ / Ctrl Shift Enter'], [m('Find in source'), '⌘ / Ctrl F'], [m('Go to definition'), 'F12'], [m('Toggle explorer'), '⌘ / Ctrl B'], [m('Toggle collaborator'), '⌘ / Ctrl Shift B'], [m('Toggle results'), '⌘ / Ctrl J'], [m('Resize focused divider'), m('Arrow keys · Home / End')], [m('Collapse focused divider'), 'Enter'], [m('Send agent message'), '⌘ / Ctrl Enter'], [m('Leave editor with Tab'), m('Esc, then Tab')]]) content.append(node('span', '', label), node('kbd', '', shortcut));
  await dialog(m('Keyboard shortcuts'), content, [{ value: 'close', label: m('Done'), primary: true }]);
});
document.addEventListener('keydown', event => {
  if ($<HTMLDialogElement>('dialog').open || !(event.ctrlKey || event.metaKey)) return;
  if (event.key.toLowerCase() === 'b') { event.preventDefault(); panels.toggle(event.shiftKey ? 'agent' : 'files'); }
  else if (event.key.toLowerCase() === 'j') { event.preventDefault(); panels.toggle('results'); }
  else if (event.key.toLowerCase() === 's') { event.preventDefault(); save().catch(fail); }
  else if (event.key === 'Enter' && event.target !== $('agent-input')) { event.preventDefault(); startOperation(event.shiftKey ? 'check' : 'run').catch(fail); }
});
async function initialize() {
  const initialRevision = documentRevision();
  if (!hasToken()) {
    const content = node('div'); content.append(node('p', '', m('Open the launch URL printed by verum studio, or paste its local access token below. The token stays in this browser tab.')));
    const label = node('label', '', m('Local access token')); const field = node('input'); field.type = 'password'; field.autocomplete = 'off'; label.append(field); content.append(label);
    if (await dialog(m('Connect to Verum Studio'), content, [{ value: 'connect', label: m('Connect'), primary: true }]) !== 'connect') return;
    installToken(field.value);
  }
  await refreshFiles();
  // A returning workspace opens its existing default file, never a colliding new draft.
  // Both async boundaries must preserve the initial document revision: typing, opening
  // another file or entering a document-switch dialog revokes this automatic restore.
  if (!switching && sameDocumentRevision(initialRevision, documentRevision()) && files.some(file => file.path === initialRevision.path)) {
    const file = await api<SourceFile>('workspace.read', { path: initialRevision.path });
    if (!switching && sameDocumentRevision(initialRevision, documentRevision())) loadDocument(file.path, file.source, file.hash);
  }
  await refreshOperations();
  scheduleAnalysis();
}
updateDocumentState(); scheduleHash();
initialize().catch(fail);
let lastPollError = '';
window.setInterval(() => {
  if (!hasToken()) return;
  refreshOperations().then(() => { if (lastPollError) { lastPollError = ''; $('connection').classList.add('online'); } }).catch(error => {
    $('connection').classList.remove('online');
    const text = errorText(error); if (text !== lastPollError) { lastPollError = text; notice(m('Local service: {error}', { error: text })); }
  });
}, 1200);
window.setInterval(() => { pollEvents().catch(error => { const text = errorText(error); if (text !== lastPollError) { lastPollError = text; notice(m('Agent event stream: {error}', { error: text })); } }); }, 700);
