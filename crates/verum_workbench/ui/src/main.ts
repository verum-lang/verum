import { projectSourceIndex, workspaceFilePath } from './project-tree';
import './style.css';
import { runtimeSourceRange } from './runtime';
import { proofSourceRange } from './proof';
import { ProofTargets, declarationProfile, type ProofProfile } from './proof-targets';
import { sourceHash as sha256 } from './source';
import { runtimeView } from './runtime-view';
import { makeEditor } from './editor';
import { CommandRegistry, type CommandContext, type StudioCommand } from './commands';
import { installCommands } from './command-surface';
import { installPanels } from './panels';
import { installExplorer } from './explorer';
import { icon, installIcons, type IconName } from './icons';
import { installTooltips } from './tooltips';
import { parentPath, type GitChange } from './project-tree';
import type { Pane } from './layout';
import { analysisScopeLabel, languageRequest, sameRevision, type DocumentRevision, type Analysis, type Range } from './language';
import { initializeLanguage, setLocale, currentLocale, m, t, setText, setAttribute, type Content, type Locale } from './i18n';
import { api, initialToken, isConnected, connectSession, onSessionChange } from './api';
import { SessionAccessError, SessionChangedError } from './session';
import { STARTER, IDENTITY, sameDocumentRevision, replaceDocumentRevision, isStale, mergeReceipts, knownReceiptVersions, safeAuthUrl, relativeVerumPath, approvalKind, approvalReply, parseArguments, type Receipt, type RunPermissions, type ExecutionPlan } from './model';

// Register before command/editor handlers. The connection page keeps ordinary
// text input and browser defaults, but cannot dispatch workspace commands.
document.addEventListener('keydown', event => { if (!isConnected()) event.stopImmediatePropagation(); }, { capture: true });
installIcons(); installTooltips(); initializeLanguage();
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
function sessionFailure(error: unknown) { return error instanceof SessionAccessError || error instanceof SessionChangedError; }
function fail(error: unknown) { if (!sessionFailure(error)) notice(errorText(error)); }
const actions = new Map<string, () => unknown | Promise<unknown>>();
function bind(id: string, action: () => unknown | Promise<unknown>) {
  if (actions.has(id)) throw new Error(`Duplicate action: ${id}`);
  actions.set(id, action);
  $(id).addEventListener('click', () => { Promise.resolve().then(action).catch(fail); });
}
function button(label: Content, action: () => unknown | Promise<unknown>, className = 'button') {
  const element = node('button', className, label); element.type = 'button';
  element.addEventListener('click', () => { Promise.resolve().then(action).catch(fail); }); return element;
}
function iconButton(name: IconName, label: Content, action: () => unknown | Promise<unknown>) {
  const element = button('', action, 'icon-button'); element.append(icon(name));
  setAttribute(element, 'aria-label', label); setAttribute(element, 'data-tooltip', label); return element;
}
function setPane(pane: Pane) { panels.show(pane); }
bind('notice-close', () => { $('notice').hidden = true; });

const themeChoice = localStorage.getItem('verum.studio.theme') || 'dark';
function applyTheme(choice: string) {
  if (!['dark', 'light', 'system'].includes(choice)) choice = 'dark';
  const dark = choice === 'dark' || (choice === 'system' && matchMedia('(prefers-color-scheme: dark)').matches);
  document.documentElement.dataset.theme = dark ? 'dark' : 'light';
  localStorage.setItem('verum.studio.theme', choice);
  $('theme').replaceChildren(icon(choice === 'system' ? 'system' : choice === 'light' ? 'sun' : 'theme'));
  setAttribute($('theme'), 'data-tooltip', m('Theme: {theme}. Change color theme', { theme: m(choice as 'dark' | 'light' | 'system') }));
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
let explorer: ReturnType<typeof installExplorer> | null = null;
let currentPath = 'experiment.vr';
let savedSource: string | null = null;
let savedHash: string | null = null;
let currentHash: string | null = null;
let hashGeneration = 0;
let hashTimer = 0;
let receipts: Receipt[] = [];
let selectedReceipt: string | null = null;
let isSaving = false;
let starting = false;
let switching = false;
let editVersion = 0;
const proofTargets = new ProofTargets();
const documentRevision = () => ({ path: currentPath, editVersion });
const editor = makeEditor($('editor'), STARTER, () => { editVersion++; updateDocumentState(); scheduleHash(); scheduleAnalysis(); }, (line, column) => {
  setText($('cursor-position'), m('Ln {line}, Col {column}', { line, column }));
});
const panels = installPanels(() => editor.view.requestMeasure());
explorer = installExplorer({ open: openFile, review: reviewChanges, failure: fail, listed: (root, entries, directory) => {
  files = [...files.filter(file => parentPath(file.path) !== directory), ...entries.filter(entry => entry.kind === 'file')];
  setText($('workspace-root'), root); $('workspace-root').dataset.root = root;
} });
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
  $('execution-plan').hidden = true;
  setText($('current-path'), currentPath);
  setText($('agent-context'), currentPath);
  $('dirty-marker').hidden = !dirty();
  $<HTMLButtonElement>('save').disabled = isSaving || !dirty();
  $<HTMLButtonElement>('reload-source').disabled = savedHash === null || isSaving;
  setText($('source-status'), currentHash ? m('{state} · {hash}', { state: m(dirty() ? 'Unsaved' : 'Saved'), hash: currentHash.slice(0, 10) }) : m('Calculating source revision…'));
  setAttribute($('source-status'), 'title', currentHash ? m('SHA-256 {hash}', { hash: currentHash }) : m('Source hashing in progress'));
  document.title = isConnected() ? `${dirty() ? '• ' : ''}${currentPath.split('/').pop()} · Verum Studio` : 'Verum Studio';
  explorer?.current(currentPath, dirty());
  const verum = currentPath.endsWith('.vr');
  for (const id of ['run', 'check', 'prove']) $<HTMLButtonElement>(id).disabled = !verum || starting;
  for (const id of ['run-settings', 'architecture-toggle']) $<HTMLButtonElement>(id).disabled = !verum;
  setText($('document-kind'), verum ? 'vr' : '·');
  setText($('document-language'), verum ? 'Verum' : m('Plain text'));
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
  editor.document(path); editor.replace(source); scheduleHash(); scheduleAnalysis(); updateDocumentState();
  explorer?.reveal(path).catch(fail); setPane('editor'); sourceView(); editor.view.focus();
}
async function refreshFiles() {
  await explorer!.refresh();
  connectionStatus(true);
}
function connectionStatus(connected: boolean) {
  $('connection').classList.toggle('online', connected);
  $('connection').replaceChildren(node('span', 'status-dot'), node('span', '', connected ? m('Connected') : m('Disconnected')));
  if (connected) delete $('settings').dataset.attention;
  else $('settings').dataset.attention = 'true';
}
async function reviewChanges(change: GitChange) {
  const content = node('div', 'change-review');
  content.append(node('p', 'muted', m('Git compares saved files. Unsaved editor changes are separate.')));
  if (change.original_path) content.append(node('p', 'result-meta', `${change.original_path} → ${change.path}`));
  const tabs = node('div', 'change-review-tabs'), patch = node('pre', 'git-patch'); content.append(tabs, patch);
  let generation = 0;
  async function show(side: 'index' | 'worktree') {
    const revision = ++generation; setText(patch, m('Loading changes…'));
    for (const tab of tabs.querySelectorAll('button')) tab.setAttribute('aria-pressed', String(tab.dataset.side === side));
    try {
      if (change.index === '?' && change.worktree === '?') {
        const file = await api<SourceFile>('document.read', { path: change.path });
        if (generation === revision) setText(patch, file.source);
      } else {
        const result = await api<{ patch: string }>('workspace.diff', { path: change.path, side });
        if (generation === revision) setText(patch, result.patch || m('No patch on this side. Refresh if the file changed.'));
      }
    } catch (error) { if (generation === revision) setText(patch, errorText(error)); }
  }
  for (const [side, label] of [['index', m('Staged')], ['worktree', m('Working tree')]] as const) {
    const tab = button(label, () => show(side), 'quiet-button'); tab.dataset.side = side;
    tab.disabled = change.index === '?' ? side === 'index' : change[side] === ' '; tabs.append(tab);
  }
  if (change.index === '?') content.prepend(node('p', 'muted', m('Untracked file · complete saved content')));
  void show(change.index !== ' ' && change.index !== '?' ? 'index' : 'worktree');
  await dialog(m('Changes · {path}', { path: change.path }), content, [{ value: 'close', label: m('Done'), primary: true }]);
  ++generation;
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
  try { await replaceDocument(async () => await mayReplace() ? api<SourceFile>('document.read', { path }) : null); }
  finally { switching = false; }
}
async function replaceDocument(prepare: () => Promise<{ path: string; source: string; hash: string | null } | null>) {
  const result = await replaceDocumentRevision(documentRevision(), documentRevision, prepare,
    file => loadDocument(file.path, file.source, file.hash));
  if (result === 'stale') announce(m('Your draft changed while loading. It has been kept; try opening the file again.'));
  return result;
}
async function save() {
  if (isSaving || !dirty()) return;
  const source = editor.source(), path = currentPath, expected_hash = savedHash;
  isSaving = true; updateDocumentState();
  try {
    const result = await api<SourceFile>('document.write', { path, source, expected_hash });
    // Saving is versioned: typing while the request is in flight stays unsaved.
    if (path === currentPath) { savedSource = result.source; savedHash = result.hash; updateDocumentState(); }
    await refreshFiles(); announce(m('Saved {path}', { path }));
  } finally { isSaving = false; updateDocumentState(); }
}
async function reloadSource() {
  if (switching || savedHash === null) return; switching = true;
  try { await replaceDocument(async () => {
    const path = currentPath;
    const file = await api<SourceFile>('document.read', { path });
    if (dirty()) {
      const content = node('div', 'source-review');
      content.append(node('p', '', m('Compare your draft with the saved source. Replacing the editor discards this draft; copy it first if you want to preserve both versions.')), button(m('Copy editor draft'), async () => { await navigator.clipboard.writeText(editor.source()); announce(m('Editor draft copied')); }, 'quiet-button'));
      content.append(node('span', 'approval-source-label', m('Current editor draft')), node('pre', '', editor.source()), node('span', 'approval-source-label', m('Saved file')), node('pre', '', file.source));
      if (await dialog(m('Review saved source'), content, [{ value: 'cancel', label: m('Keep editor draft') }, { value: 'reload', label: m('Replace with saved source'), primary: true }]) !== 'reload') return null;
    }
    return file;
  });
  } finally { switching = false; }
}
async function newDocument(source = '', suggestion = 'experiment.vr') {
  if (switching) return; switching = true;
  try { const replaced = await replaceDocument(async () => {
    if (!(await mayReplace())) return null;
    const content = node('div'); content.append(node('p', '', m('Create a Verum source file in this workspace. Existing directories are supported.')));
    const label = node('label', '', m('File path')); const field = node('input'); field.value = suggestion; field.autocomplete = 'off'; field.spellcheck = false;
    label.append(field); content.append(label);
    if (await dialog(m('New experiment'), content, [{ value: 'cancel', label: m('Cancel') }, { value: 'create', label: m('Create draft'), primary: true }]) !== 'create') return null;
    const path = relativeVerumPath(field.value);
    if (!path) throw new Error(t(m('Use a relative .vr path without hidden traversal, for example experiment.vr.')));
    if (files.some(file => file.path === path)) throw new Error(t(m('{path} already exists. Open it from the explorer or choose a different name.', { path })));
    return { path, source, hash: null };
  });
    if (replaced === 'applied' && source === IDENTITY) {
      proofTargets.select(languageRevision(), { profile: 'verum-source-pi-id/1', declaration: '' });
      renderProofTargets(); openExecutionSettings();
    }
  } finally { switching = false; }
}
bind('save', save); bind('refresh-files', refreshFiles); bind('new-file', () => newDocument());
let analysisGeneration = 0;
let analysisTimer = 0, analysisBusy = false, analysisPending = false;
const languageRevision = (): DocumentRevision => ({ path: currentPath, source: editor.source() });
function sourceView() {
  $('architecture').hidden = true; $('editor').hidden = false;
  setAttribute($('architecture-toggle'), 'aria-label', m('Source architecture')); $('architecture-toggle').setAttribute('aria-expanded', 'false');
  setAttribute($('architecture-toggle'), 'data-tooltip', m('Source architecture'));
  editor.view.requestMeasure();
}
function showRange(range: Range) { sourceView(); setPane('editor'); editor.select(range); }
async function navigateDefinition(location: { uri: string; range: Range }) {
  const relative = workspaceFilePath(location.uri, $('workspace-root').dataset.root || '');
  if (relative === null) throw new Error(t(m('Definition is outside this workspace.')));
  if (relative !== currentPath) await openFile(relative);
  if (relative === currentPath) showRange(location.range);
}
editor.connectLanguage({ revision: languageRevision, failure: fail, navigate: navigateDefinition });
bind('architecture-toggle', async () => {
  if (!$('architecture').hidden) { sourceView(); return; }
  $('architecture').hidden = false; $('editor').hidden = true;
  setAttribute($('architecture-toggle'), 'aria-label', m('Return to source')); $('architecture-toggle').setAttribute('aria-expanded', 'true');
  setAttribute($('architecture-toggle'), 'data-tooltip', m('Return to source'));
  await inspectProject();
});
let projectRequest = 0;
async function inspectProject() {
  const request = ++projectRequest;
  const list = $('project-content');
  list.setAttribute('aria-busy', 'true');
  try {
    const project = await api<{ workspace: { root: string; files: FileRecord[] }; analysis: { sources?: Array<{ uri: string }>; issues?: string[]; documents: Array<Analysis & { uri: string }> } }>('project.inspect');
    if (request !== projectRequest) return;
    const index = projectSourceIndex(project.workspace.root, project.analysis.sources ?? [], project.analysis.documents);
    const rows = document.createDocumentFragment();
    for (const issue of project.analysis.issues ?? []) rows.append(node('p', 'muted', issue));
    for (const file of project.workspace.files) {
      const digest = index.get(file.path);
      const status = digest === undefined ? m('Not analyzed') : digest === null ? m('Indexed source') : digest.slice(0, 10);
      const entry = button('', () => openFile(file.path), 'project-file');
      entry.append(node('span', '', file.path), node('span', 'muted', status));
      rows.append(entry);
    }
    list.replaceChildren(rows);
  } finally {
    if (request === projectRequest) list.removeAttribute('aria-busy');
  }
}
bind('project-refresh', async () => { scheduleAnalysis(); await inspectProject(); });
window.addEventListener('focus', scheduleAnalysis);
function resultView(diagnostics: boolean) {
  $('operations-panel').hidden = diagnostics; $('diagnostics-panel').hidden = !diagnostics;
  $('operations-tab').setAttribute('aria-selected', String(!diagnostics)); $('diagnostics-tab').setAttribute('aria-selected', String(diagnostics));
  $('operations-tab').tabIndex = diagnostics ? -1 : 0; $('diagnostics-tab').tabIndex = diagnostics ? 0 : -1;
}
bind('operations-tab', () => resultView(false)); bind('diagnostics-tab', () => resultView(true));
for (const id of ['operations-tab', 'diagnostics-tab']) $(id).addEventListener('keydown', event => {
  if (event.isComposing || event.altKey || event.ctrlKey || event.metaKey || !['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
  event.preventDefault();
  const diagnostics = event.key === 'End' || (event.key !== 'Home' && id === 'operations-tab');
  resultView(diagnostics); $(diagnostics ? 'diagnostics-tab' : 'operations-tab').focus();
});
bind('language-status', () => { resultView(true); setPane('results'); });
function scheduleAnalysis() {
  ++analysisGeneration;
  proofTargets.begin(languageRevision()); renderProofTargets();
  window.clearTimeout(analysisTimer);
  editor.diagnostics(editor.source(), []);
  $('analysis-scope').replaceChildren();
  if (!currentPath.endsWith('.vr')) {
    analysisPending = false;
    setText($('language-status'), m('Plain text · no Verum analysis'));
    setText($('diagnostics-count'), '0');
    $('diagnostics-list').replaceChildren(node('p', 'muted', m('Plain text · no Verum analysis')));
    $('architecture-content').replaceChildren(); closeSettings(); return;
  }
  setText($('language-status'), m('Analyzing…'));
  setText($('diagnostics-count'), '…');
  $('diagnostics-list').replaceChildren(node('p', 'muted', m('Waiting for current source analysis…')));
  $('architecture-content').replaceChildren(node('p', 'muted', m('Waiting for current source analysis…')));
  analysisPending = true; analysisTimer = window.setTimeout(analyzeSource, 350);
}
async function analyzeSource() {
  if (analysisBusy || !isConnected()) return;
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
      setText($('proof-target-status'), message);
    }
  } finally { analysisBusy = false; if (analysisPending) analysisTimer = window.setTimeout(analyzeSource, 100); }
}
function renderAnalysis(snapshot: Analysis) {
  const revision = languageRevision();
  proofTargets.accept(revision, snapshot); renderProofTargets();
  editor.diagnostics(editor.source(), snapshot.diagnostics);
  const count = snapshot.diagnostics.length;
  setText($('language-status'), count ? m('{count} diagnostics', { count }) : m('No module diagnostics'));
  setText($('diagnostics-count'), String(count));
  const scope = $('analysis-scope'), label = analysisScopeLabel(snapshot.analysis_scope);
  if (snapshot.library_metadata && snapshot.analysis_scope === 'project-sources-with-library-metadata') {
    const { sha256, format_version } = snapshot.library_metadata;
    const details = node('details');
    details.append(node('summary', '', label), node('code', '', m('Metadata v{version} · SHA-256 {digest}', { version: format_version, digest: sha256 })));
    scope.replaceChildren(details);
  } else {
    scope.replaceChildren(node('span', '', label));
  }
  const diagnostics = $('diagnostics-list'); diagnostics.replaceChildren();
  for (const diagnostic of snapshot.diagnostics) {
    const entry = button('', () => showRange(diagnostic.range), 'diagnostic-item');
    entry.append(node('span', `diagnostic-marker severity-${diagnostic.severity}`, diagnostic.severity === 1 ? '●' : '◇'), node('span', 'diagnostic-message', diagnostic.message), node('span', 'muted', `${diagnostic.range.start.line + 1}:${diagnostic.range.start.character + 1}`));
    diagnostics.append(entry);
    for (const related of diagnostic.relatedInformation ?? []) {
      const link = button(related.message, () => navigateDefinition(related.location), 'diagnostic-related');
      const position = related.location.range.start;
      link.append(node('span', 'muted', `${position.line + 1}:${position.character + 1}`));
      diagnostics.append(link);
    }
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
    const fields = declaration.record_fields;
    if (fields?.length) {
      const group = node('details', 'record-members');
      group.append(node('summary', '', m('Fields ({count})', { count: fields.length })));
      for (const field of fields) {
        const row = node('div', 'record-member');
        row.append(button(field.name, () => showRange(field.selection_range), 'declaration-name'),
          button(field.annotation.source, () => showRange(field.annotation.range), 'record-annotation'));
        for (const index of field.depends_on ?? []) {
          const carrier = fields[index];
          if (carrier) row.append(button(m('Depends on {field}', { field: carrier.name }),
            () => showRange(carrier.selection_range), 'record-dependency'));
        }
        group.append(row);
      }
      card.append(group);
    }
    if (declaration.requirements.length) {
      card.append(node('p', 'muted', m('Context requirements')));
      for (const requirement of declaration.requirements) card.append(button(requirement.source, () => showRange(requirement.range), 'context-requirement'));
    }
    for (const [label, clauses] of [[m('Preconditions'), declaration.preconditions], [m('Postconditions'), declaration.postconditions]] as const) {
      if (!clauses?.length) continue;
      card.append(node('p', 'muted', label));
      for (const clause of clauses) card.append(button(clause.source, () => showRange(clause.range), 'context-requirement'));
    }
    const profile = declarationProfile(declaration);
    if (profile) card.append(iconButton('proof', m(profile === 'verum-source-finite/1' ? 'Check finite function contract' : 'Check logical theorem'), () => {
      if (!sameRevision(revision, languageRevision())) throw new Error(t(m('Source changed. Select a declaration from the current analysis.')));
      proofTargets.select(revision, { profile, declaration: declaration.name }); renderProofTargets();
      return startOperation('proof');
    }));
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
function openExecutionSettings() { $('execution-settings').hidden = false; $('run-settings').setAttribute('aria-expanded', 'true'); }
function renderProofTargets() {
  const revision = languageRevision(), selection = proofTargets.selection(revision.path);
  const options = proofTargets.options(revision, selection.profile), field = $<HTMLSelectElement>('declaration');
  $<HTMLSelectElement>('proof-profile').value = selection.profile;
  const placeholder = node('option', '', m('Select a declaration')); placeholder.value = '';
  field.replaceChildren(placeholder);
  for (const target of options ?? []) {
    const option = node('option', '', target.name); option.value = target.name; field.append(option);
  }
  const valid = options?.some(target => target.name === selection.declaration);
  field.value = valid ? selection.declaration : ''; field.disabled = !options?.length;
  setText($('proof-target-status'), options === null ? m('Waiting for current source analysis…')
    : !options.length ? m('No declarations for this proof scope in the current source.')
    : selection.declaration && !valid ? m('The selected declaration is no longer available. Choose a current declaration.')
    : m('Targets come from current source analysis. Proof support is checked by the selected verifier.'));
}
for (const id of ['proof-profile', 'declaration']) $(id).addEventListener('change', () => {
  proofTargets.select(languageRevision(), { profile: $<HTMLSelectElement>('proof-profile').value as ProofProfile,
    declaration: id === 'proof-profile' ? '' : $<HTMLSelectElement>('declaration').value });
  renderProofTargets();
});
bind('run-settings', () => { if ($('execution-settings').hidden) openExecutionSettings(); else closeSettings(); });
function closeSettings() { $('execution-settings').hidden = true; $('run-settings').setAttribute('aria-expanded', 'false'); }
document.addEventListener('pointerdown', event => {
  if (event.target instanceof Node && !$('execution-settings').contains(event.target) && !$('run-settings').contains(event.target)) closeSettings();
});
document.addEventListener('keydown', event => {
  if (event.key === 'Escape' && !$('execution-settings').hidden) { event.preventDefault(); closeSettings(); $('run-settings').focus(); }
});
window.addEventListener('beforeunload', event => { if (dirty()) { event.preventDefault(); event.returnValue = ''; } });

function operationRequest(kind: Receipt['kind'], diagnostics = input('runtime-diagnostics').checked) {
  if (!currentPath.endsWith('.vr')) throw new Error(t(m('Select a Verum source file to run, check or prove.')));
  const source = editor.source(), path = currentPath;
  const seconds = Number(input('timeout').value);
  if (!Number.isFinite(seconds) || seconds < 1 || seconds > 300) throw new Error(t(m('Choose a time budget between 1 and 300 seconds.')));
  const target = proofTargets.request(languageRevision());
  if (kind === 'proof' && !target) {
    openExecutionSettings();
    ($<HTMLSelectElement>('declaration').disabled ? $('proof-profile') : $('declaration')).focus();
    throw new Error(t(m('Select a declaration from the current source analysis.')));
  }
  const permissions = $<HTMLSelectElement>('run-permissions').value as RunPermissions;
  return { kind, source, path, ...(kind === 'proof' && target ? { declaration: target.declaration, proof_profile: target.profile } : {}), ...(kind === 'run' ? { permissions, runtime_diagnostics: diagnostics } : {}), timeout_ms: Math.round(seconds * 1000) };
}
for (const id of ['run-permissions', 'runtime-diagnostics', 'timeout', 'declaration', 'proof-profile']) $(id).addEventListener('input', () => { $('execution-plan').hidden = true; });
bind('preview-run', async () => {
  const request = operationRequest('run'), revision = documentRevision();
  const plan = await api<ExecutionPlan>('operation.plan', request);
  if (!sameDocumentRevision(revision, documentRevision()) || JSON.stringify(request) !== JSON.stringify(operationRequest('run'))) {
    throw new Error(t(m('Source or execution settings changed. Preview again.')));
  }
  $('execution-plan-content').textContent = pretty(plan); $('execution-plan').hidden = false;
  announce(m('Run plan ready; no process started.'));
});
async function startOperation(kind: Receipt['kind'], diagnostics?: boolean) {
  if (starting) return;
  const request = operationRequest(kind, diagnostics);
  starting = true; ['run', 'check', 'prove'].forEach(id => { $<HTMLButtonElement>(id).disabled = true; });
  try {
    if (kind === 'run' && request.runtime_diagnostics) {
      const plan = await api<ExecutionPlan>('operation.plan', request);
      if (plan.runtime_diagnostics !== true) throw new Error(t(m('This Studio host does not support runtime diagnostics. Restart with the matching Studio build.')));
    }
    const result = await api<{ id: string }>('operation.start', request);
    selectedReceipt = result.id; await refreshOperations(); resultView(false); setPane('results'); announce(m('{kind} started for {path}', { kind: resultLabel(kind), path: request.path }));
  } finally { starting = false; updateDocumentState(); }
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
function appendProofLocation(parent: HTMLElement, receipt: Receipt, span: unknown, key: string) {
  if (typeof parseArguments(span).start !== 'number') return;
  const reveal = iconButton('source', m('Reveal proof location'), async () => {
    const revision = documentRevision();
    const range = await proofSourceRange(receipt, span, currentPath, currentHash, editor.source());
    if (range && sameDocumentRevision(revision, documentRevision())) {
      setPane('editor'); sourceView(); editor.select(range);
    } else announce(m('Proof location is unavailable for the current source.'));
  });
  reveal.dataset.focusKey = key;
  reveal.disabled = isStale(receipt, currentPath, currentHash) || !['succeeded', 'failed', 'unsupported'].includes(receipt.status);
  parent.classList.add('proof-location'); parent.append(reveal);
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
    const row = node('span', 'history-row'); row.append(icon(receipt.kind === 'proof' ? 'proof' : receipt.kind === 'check' ? 'check' : 'run'), node('span', '', resultLabel(receipt.kind)), node('span', `badge ${receipt.status}`, m(receipt.status)));
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
    const cancel = iconButton('close', m('Cancel'), async () => { await api('operation.cancel', { id: receipt.id }); await refreshOperations(); });
    cancel.dataset.focusKey = 'cancel'; heading.append(cancel);
  }
  const copy = iconButton('copy', m('Copy receipt'), async () => { await navigator.clipboard.writeText(pretty(receipt)); announce(m('Receipt copied')); });
  copy.dataset.focusKey = 'copy'; heading.append(copy);
  detail.append(heading, node('p', 'result-meta', receipt.exit_code == null ? m('{path} · {seconds}s · {hash}', { path: receipt.path, seconds: elapsed(receipt) / 1000, hash: receipt.source_hash.slice(0, 12) }) : m('{path} · {seconds}s · exit {code} · {hash}', { path: receipt.path, seconds: elapsed(receipt) / 1000, code: receipt.exit_code, hash: receipt.source_hash.slice(0, 12) })));
  const context = node('details'); context.dataset.key = 'context';
  const contextHeading = node('summary', '', m('Execution context')); contextHeading.dataset.focusKey = 'context'; context.append(contextHeading);
  if (receipt.timeout_ms) context.append(node('p', 'muted', m(receipt.evidence_path ? 'Wall budget {seconds}s · source and receipt retained locally' : 'Wall budget {seconds}s', { seconds: receipt.timeout_ms / 1000 })));
  if (receipt.kind === 'run') {
    const label = receipt.permissions === 'source' ? 'Source / CLI defaults' : receipt.permissions === 'deny-all' ? 'Deny all grants' : receipt.permissions === 'allow-all' ? 'Allow all grants' : 'Not recorded';
    context.append(node('p', 'result-meta', m('Script permissions: {policy}', { policy: m(label) })));
  }
  if (receipt.termination) context.append(node('p', 'termination', receipt.termination));
  if (receipt.kind === 'proof') {
    const proof = node('div', 'proof-outcome');
    const report = parseArguments(receipt.proof), diagnostic = parseArguments(report.diagnostic);
    proof.append(node('h3', '', m('Proof evidence')));
    if (receipt.declaration) {
      const declaration = node('p', '', m('Declaration: {declaration}', { declaration: receipt.declaration }));
      appendProofLocation(declaration, receipt, parseArguments(report.source).span, 'proof-declaration'); proof.append(declaration);
    }
    if (typeof diagnostic.message === 'string') {
      const message = node('p', '', diagnostic.message);
      appendProofLocation(message, receipt, report.diagnostic_span, 'proof-diagnostic'); proof.append(message);
    }
    proof.append(node('p', '', m(receipt.proof_profile === 'verum-source-finite/1' ? 'Finite function contract' : receipt.proof_profile === 'verum-source-pi-id/1' ? 'Logical theorem' : 'Not recorded')));
    const implementation = parseArguments(report.implementation);
    if (receipt.status === 'succeeded' && Array.isArray(implementation.obligations)) {
      const obligations = node('details'); obligations.dataset.key = 'obligations';
      const summary = node('summary', '', m('Checked obligations: {count}', { count: implementation.obligations.length })); summary.dataset.focusKey = 'obligations'; obligations.append(summary);
      const labels: Record<string, Content> = { function_contract: m('Function contract'), body_call: m('Call in function body'), precondition_call: m('Call in precondition'), postcondition_call: m('Call in postcondition') };
      for (const [index, raw] of implementation.obligations.entries()) {
        const obligation = parseArguments(raw), owner = parseArguments(obligation.owner), callee = parseArguments(obligation.callee);
        const label = labels[String(obligation.kind)] ?? m('Proof obligation');
        const row = node('p', 'muted', `${String(owner.declaration ?? '')} · `);
        row.append(node('span', '', label), document.createTextNode(typeof callee.declaration === 'string' ? ` → ${callee.declaration}` : ''));
        appendProofLocation(row, receipt, obligation.call ?? owner.span, `proof-obligation-${index}`); obligations.append(row);
      }
      proof.append(obligations);
    }
    proof.append(node('p', '', m('This judgment applies to the selected declaration, proof scope and source revision. Native implementation correspondence is separate.'))); detail.append(proof);
    if (receipt.proof !== undefined && receipt.proof !== null) appendDetails(detail, m('Full checker report'), receipt.proof, 'proof');
  } else if (receipt.kind === 'run') context.append(node('p', 'muted', m('Observed execution for this source revision; not a proof of all behaviors.')));
  if (receipt.stdout) {
    if (receipt.kind === 'proof' && receipt.proof) {
      const stream = node('details'); stream.dataset.key = 'raw-proof'; const summary = node('summary', '', m('Raw checker output')); summary.dataset.focusKey = 'raw-proof'; stream.append(summary, node('pre', '', receipt.stdout)); detail.append(stream);
    } else appendStream(detail, m('Standard output'), receipt.stdout, 'stdout');
  }
  if (receipt.stderr) appendStream(detail, m('Diagnostics / standard error'), receipt.stderr, 'stderr');
  if (!receipt.stdout && !receipt.stderr) detail.append(node('p', 'muted', receipt.status === 'running' ? m('Waiting for process output…') : m('The process produced no text output.')));
  const runtime = runtimeView(receipt,
    event => runtimeSourceRange(receipt, event, currentPath, currentHash, editor.source()) !== null,
    event => {
      const range = runtimeSourceRange(receipt, event, currentPath, currentHash, editor.source());
      if (range) { setPane('editor'); sourceView(); editor.select(range); }
    });
  if (runtime) detail.append(runtime);
  detail.append(context); appendDetails(detail, m('Full operation receipt'), receipt, 'receipt');
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
        if (!switching && args.path === currentPath && !dirty()) {
          await replaceDocument(() => api<SourceFile>('workspace.read', { path: args.path }));
        }
      }
      await refreshOperations();
    } catch (error) { status.append(node('p', '', m('Refresh unavailable: {error}', { error: errorText(error) }))); }
  }
  actions.append(button(m('Decline'), () => respond(false)));
  if (kind !== 'unsupported' && reviewAvailable) actions.append(button(kind === 'source' ? m('Accept edit') : kind === 'read' ? params.tool === 'verum_analyze' ? m('Allow analysis') : m('Allow read') : m('Approve operation'), () => respond(true), 'button primary'));
  card.append(content, actions, status); $('agent-welcome').hidden = true; $('agent-messages').append(card); pendingApprovals++; agentState();
  announce(m('Agent request awaiting your review'));
}

bind('theme', () => {
  const modes = ['system', 'light', 'dark'];
  const next = modes[(modes.indexOf(localStorage.getItem('verum.studio.theme') || 'dark') + 1) % modes.length];
  editor.theme(applyTheme(next));
});
bind('settings', () => { setText($('connection-endpoint'), location.origin); $<HTMLDialogElement>('settings-dialog').showModal(); });
bind('settings-close', () => $<HTMLDialogElement>('settings-dialog').close());
function findProjectFile() { setPane('files'); input('file-filter').focus(); input('file-filter').select(); }
bind('command-palette', () => commands.palette());
bind('help', () => {
  $<HTMLDialogElement>('settings-dialog').close();
  commands.preferences();
});
const everywhere: CommandContext[] = ['workspace', 'editor', 'agent'];
function command(id: string, label: Content, shortcut: string, run: StudioCommand['run'], contexts = everywhere, enabled = () => true, controls?: string): StudioCommand {
  return { id, label: () => t(label), shortcut, run, contexts, enabled, controls };
}
function control(id: string, label: Content, shortcut = '', contexts = everywhere, available = () => true): StudioCommand {
  const run = actions.get(id); if (!run) throw new Error(`Missing command action: ${id}`);
  return command(id, label, shortcut, run, contexts, () => available() && !$<HTMLButtonElement>(id).disabled, `#${id}`);
}
function editorAction(run: StudioCommand['run']): StudioCommand['run'] {
  return () => { setPane('editor'); sourceView(); return run(); };
}
const commandRegistry = new CommandRegistry([
  control('command-palette', m('Command palette'), 'Mod+Shift+P'),
  control('help', m('Keyboard shortcuts'), 'F1'), control('settings', m('Settings'), 'Mod+Comma'),
  command('project-search', m('Find a project file'), 'Mod+P', findProjectFile),
  control('save', m('Save source'), 'Mod+S'), control('new-file', m('New Verum file')),
  control('reload-source', m('Review the saved file before reloading')), control('refresh-files', m('Refresh files')),
  control('run', m('Run source'), 'Mod+Enter', ['editor']), control('check', m('Check source'), 'Mod+Shift+Enter', ['editor']),
  control('prove', m('Check a declaration'), 'Alt+Shift+Enter', ['editor']),
  command('run.cbgr', m('Run with CBGR diagnostics'), '', () => startOperation('run', true), ['editor'], () => currentPath.endsWith('.vr') && !starting),
  control('run-settings', m('Execution options')), control('preview-run', m('Inspect run plan')),
  control('architecture-toggle', m('Source architecture')), control('project-refresh', m('Refresh project')),
  control('language-status', m('Show module diagnostics')), control('theme', m('Change color theme')),
  control('agent-connect', m('Connect agent')), control('agent-login', m('Sign in'), '', everywhere, () => connected),
  control('new-thread', m('New conversation')), control('agent-stop', m('Stop agent turn'), '', everywhere, () => turnId !== null),
  command('agent.send', m('Send agent message'), 'Mod+Enter', sendMessage, ['agent'], () => !$<HTMLButtonElement>('agent-send').disabled, '#agent-send'),
  command('view.files', m('Toggle explorer'), 'Mod+B', () => panels.toggle('files'), everywhere, undefined, '[data-toggle-panel="files"]'),
  command('view.agent', m('Toggle collaborator'), 'Mod+Shift+B', () => panels.toggle('agent'), everywhere, undefined, '[data-toggle-panel="agent"]'),
  command('view.results', m('Toggle results'), 'Mod+J', () => panels.toggle('results'), everywhere, undefined, '[data-toggle-panel="results"]'),
  command('focus.editor', m('Focus editor'), 'Alt+1', () => { setPane('editor'); sourceView(); editor.view.focus(); }),
  command('focus.agent', m('Focus collaborator'), 'Alt+2', () => { setPane('agent'); $(input('agent-input').disabled ? 'agent-connect' : 'agent-input').focus(); }),
  command('focus.results', m('Focus results'), 'Alt+3', () => { setPane('results'); $( $('diagnostics-panel').hidden ? 'operations-tab' : 'diagnostics-tab').focus(); }),
  command('editor.nextDiagnostic', m('Next diagnostic'), 'F8', editorAction(editor.actions.nextDiagnostic), ['editor'], editor.hasDiagnostics),
  command('editor.previousDiagnostic', m('Previous diagnostic'), 'Shift+F8', editorAction(editor.actions.previousDiagnostic), ['editor'], editor.hasDiagnostics),
  command('editor.definition', m('Go to definition'), 'F12', editorAction(editor.definition), ['editor'], () => currentPath.endsWith('.vr')),
  command('editor.format', m('Format document'), 'Alt+Shift+F', editorAction(editor.format), ['editor'], () => currentPath.endsWith('.vr')),
  command('editor.find', m('Find in source'), 'Mod+F', editorAction(editor.actions.find), ['editor']),
  command('editor.line', m('Go to line'), 'Alt+G', editorAction(editor.actions.gotoLine), ['editor']),
  command('editor.undo', m('Undo edit'), 'Mod+Z', editorAction(editor.actions.undo), ['editor']),
  command('editor.redo', m('Redo edit'), /Mac|iPhone|iPad/.test(navigator.platform) ? 'Mod+Shift+Z' : 'Mod+Y', editorAction(editor.actions.redo), ['editor']),
  command('editor.comment', m('Toggle comment'), 'Mod+Slash', editorAction(editor.actions.comment), ['editor']),
  command('editor.complete', m('Show completions'), 'Ctrl+Space', editorAction(editor.actions.complete), ['editor'], () => currentPath.endsWith('.vr')),
]);
const commands = installCommands(commandRegistry, fail, announce);
let workspaceInitialized = false;
async function initialize() {
  const initialRevision = documentRevision();
  const restoreInitial = !workspaceInitialized;
  workspaceInitialized = true;
  await refreshFiles();
  // A returning workspace opens its existing default file, never a colliding new draft.
  // Both async boundaries must preserve the initial document revision: typing, opening
  // another file or entering a document-switch dialog revokes this automatic restore.
  const initialPath = files.find(file => file.path === initialRevision.path)?.path
    ?? files.find(file => file.path.toLowerCase() === 'readme.md')?.path
    ?? files.find(file => file.path.endsWith('.vr'))?.path;
  if (restoreInitial && !switching && sameDocumentRevision(initialRevision, documentRevision()) && initialPath) {
    const file = await api<SourceFile>('document.read', { path: initialPath });
    if (!switching && sameDocumentRevision(initialRevision, documentRevision())) loadDocument(file.path, file.source, file.hash);
  }
  await refreshOperations();
  scheduleAnalysis();
}
updateDocumentState(); scheduleHash();
let sessionWasConnected = false;
let lastPollError = '';
onSessionChange(state => {
  const unlocked = state === 'connected';
  $('app').hidden = !unlocked; $('app').inert = !unlocked;
  $('session-gate').hidden = unlocked;
  input('session-token').disabled = state === 'checking';
  $<HTMLButtonElement>('session-connect').disabled = state === 'checking';
  setText($('session-connect'), state === 'checking' ? m('Connecting…') : m('Connect'));
  if (unlocked) {
    if (lastPollError) { $('notice').hidden = true; lastPollError = ''; }
    updateDocumentState();
  }
  if (!unlocked) {
    proofTargets.clear(); renderProofTargets();
    document.querySelectorAll<HTMLDialogElement>('dialog[open]').forEach(dialog => dialog.close('session-locked'));
    document.title = 'Verum Studio';
    if (sessionWasConnected) {
      setText($('session-error'), m('Session access expired. Reconnect using the current launch URL.'));
      // A replacement host has its own operation and agent identities. Keep
      // the editor draft and readable conversation, but retire old actions.
      receipts = []; selectedReceipt = null; renderResults();
      analysisGeneration++; analysisPending = false;
      connected = false; authenticated = false; threadId = null; turnId = null;
      cursor = 0; pendingApprovals = 0; approvalIds.clear(); completedTurns.clear(); messageBodies.clear();
      document.querySelectorAll<HTMLButtonElement>('#agent-messages button').forEach(button => { button.disabled = true; });
      agentState(); connectionStatus(false);
    }
    if (state === 'locked') input('session-token').focus();
  }
  sessionWasConnected = unlocked;
});
setText($('session-endpoint'), location.origin);
async function connectWorkspace() {
  setText($('session-error'), '');
  try {
    await connectSession(input('session-token').value);
    input('session-token').value = '';
    await initialize();
    setPane('editor');
    if (!$('editor').hidden) editor.view.focus(); else $('editor-pane').focus();
  } catch (error) {
    if (isConnected()) fail(error);
    else { setText($('session-error'), errorText(error)); input('session-token').focus(); }
  }
}
$('session-form').addEventListener('submit', event => { event.preventDefault(); void connectWorkspace(); });
if (initialToken) { input('session-token').value = initialToken; void connectWorkspace(); }
window.setInterval(() => {
  if (!isConnected()) return;
  refreshOperations().then(() => { if (lastPollError) { lastPollError = ''; connectionStatus(true); } }).catch(error => {
    if (sessionFailure(error)) return;
    connectionStatus(false);
    const text = errorText(error); if (text !== lastPollError) { lastPollError = text; notice(m('Connection: {error}', { error: text })); }
  });
}, 1200);
window.setInterval(() => { if (isConnected()) pollEvents().catch(error => { if (sessionFailure(error)) return; const text = errorText(error); if (text !== lastPollError) { lastPollError = text; notice(m('Agent event stream: {error}', { error: text })); } }); }, 700);
window.setInterval(() => { if (isConnected() && document.visibilityState === 'visible') explorer?.refresh().catch(fail); }, 5000);
