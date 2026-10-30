import './style.css';
import { makeEditor } from './editor';
import { installPanels } from './panels';
import type { Pane } from './layout';
import { api, hasToken, installToken } from './api';
import { STARTER, IDENTITY, sameDocumentRevision, isStale, mergeReceipts, knownReceiptVersions, safeAuthUrl, relativeVerumPath, approvalKind, approvalReply, parseArguments, type Receipt } from './model';

const $ = <T extends HTMLElement = HTMLElement>(id: string) => {
  const node = document.getElementById(id);
  if (!node) throw new Error(`Missing interface element: ${id}`);
  return node as T;
};
const input = (id: string) => $<HTMLInputElement>(id);
function node<K extends keyof HTMLElementTagNameMap>(tag: K, className = '', text = ''): HTMLElementTagNameMap[K] {
  const element = document.createElement(tag); element.className = className; element.textContent = text; return element;
}
const pretty = (value: unknown) => JSON.stringify(value, null, 2);
function errorText(error: unknown) { return error instanceof Error ? error.message : String(error); }
function notice(message: string) { $('notice-text').textContent = message; $('notice').hidden = false; }
function announce(message: string) { $('announcer').textContent = message; }
function fail(error: unknown) { notice(errorText(error)); }
function bind(id: string, action: () => unknown | Promise<unknown>) {
  $(id).addEventListener('click', () => { Promise.resolve().then(action).catch(fail); });
}
function button(label: string, action: () => unknown | Promise<unknown>, className = 'button') {
  const element = node('button', className, label); element.type = 'button';
  element.addEventListener('click', () => { Promise.resolve().then(action).catch(fail); }); return element;
}
function setPane(pane: Pane) { panels.show(pane); }
bind('notice-close', () => { $('notice').hidden = true; });

const themeChoice = localStorage.getItem('verum.studio.theme') || 'dark';
function applyTheme(choice: string) {
  const dark = choice === 'dark' || (choice === 'system' && matchMedia('(prefers-color-scheme: dark)').matches);
  document.documentElement.dataset.theme = dark ? 'dark' : 'light';
  localStorage.setItem('verum.studio.theme', choice);
  $('theme').title = `Theme: ${choice}. Change color theme`;
  $('theme').setAttribute('aria-label', `Color theme: ${choice}. Change theme`);
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
const editor = makeEditor($('editor'), STARTER, () => { editVersion++; updateDocumentState(); scheduleHash(); }, (line, column) => {
  $('cursor-position').textContent = `Ln ${line}, Col ${column}`;
});
const panels = installPanels(() => editor.view.requestMeasure());
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
  $('current-path').textContent = currentPath;
  $('agent-context').textContent = currentPath;
  $('dirty-marker').hidden = !dirty();
  $<HTMLButtonElement>('save').disabled = isSaving || !dirty();
  $<HTMLButtonElement>('reload-source').disabled = savedHash === null || isSaving;
  $('source-status').textContent = currentHash ? `${dirty() ? 'Unsaved' : 'Saved'} · ${currentHash.slice(0, 10)}` : 'Calculating source revision…';
  $('source-status').title = currentHash ? `SHA-256 ${currentHash}` : 'Source hashing in progress';
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
  editor.replace(source); scheduleHash(); updateDocumentState(); renderFiles(); setPane('editor'); editor.view.focus();
}
async function refreshFiles() {
  const result = await api<{ root: string; files: FileRecord[] }>('workspace.list');
  files = result.files.filter(file => file.path.endsWith('.vr')).sort((a, b) => a.path.localeCompare(b.path));
  $('workspace-root').textContent = result.root; $('workspace-root').title = result.root;
  $('connection').classList.add('online'); $('connection').replaceChildren(node('span', 'status-dot'), document.createTextNode('Local service'));
  renderFiles();
}
function renderFiles() {
  $('files-rail-status').textContent = String(files.length);
  $('files-restore').title = `Expand explorer · ${files.length} Verum files`;
  const filter = input('file-filter').value.toLowerCase();
  const list = $('file-list'); list.replaceChildren();
  const visible = files.filter(file => file.path.toLowerCase().includes(filter));
  if (!visible.length) { list.append(node('p', 'muted empty-note', filter ? 'No matching Verum files.' : 'No Verum files yet. Start an experiment below.')); return; }
  for (const file of visible) {
    const entry = button('', () => openFile(file.path), 'file-entry');
    entry.setAttribute('aria-current', String(file.path === currentPath)); entry.title = file.path;
    entry.append(node('span', 'file-icon', 'vr'), node('span', '', file.path));
    const wrapper = node('div'); wrapper.setAttribute('role', 'listitem'); wrapper.append(entry); list.append(wrapper);
  }
}
type Choice = { value: string; label: string; primary?: boolean };
function dialog(title: string, content: HTMLElement, choices: Choice[]): Promise<string> {
  const modal = $<HTMLDialogElement>('dialog');
  if (modal.open) return Promise.resolve('cancel');
  $('dialog-title').textContent = title; $('dialog-body').replaceChildren(content); $('dialog-actions').replaceChildren();
  for (const choice of choices) {
    const item = node('button', `button${choice.primary ? ' primary' : ''}`, choice.label);
    item.value = choice.value; item.type = 'submit'; $('dialog-actions').append(item);
  }
  modal.returnValue = 'cancel'; modal.showModal();
  return new Promise(resolve => modal.addEventListener('close', () => resolve(modal.returnValue || 'cancel'), { once: true }));
}
async function mayReplace() {
  if (!dirty()) return true;
  const choice = await dialog('Keep your current experiment?', node('p', '', `${currentPath} has unsaved changes. Save them before opening another source, or discard them explicitly.`), [
    { value: 'cancel', label: 'Cancel' }, { value: 'discard', label: 'Discard changes' }, { value: 'save', label: 'Save and continue', primary: true },
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
    await refreshFiles(); announce(`Saved ${path}`);
  } finally { isSaving = false; updateDocumentState(); }
}
async function reloadSource() {
  if (switching || savedHash === null) return; switching = true;
  try {
    const path = currentPath;
    const file = await api<SourceFile>('workspace.read', { path });
    if (dirty()) {
      const content = node('div', 'source-review');
      content.append(node('p', '', 'Compare your draft with the saved source. Replacing the editor discards this draft; copy it first if you want to preserve both versions.'), button('Copy editor draft', async () => { await navigator.clipboard.writeText(editor.source()); announce('Editor draft copied'); }, 'quiet-button'));
      content.append(node('span', 'approval-source-label', 'Current editor draft'), node('pre', '', editor.source()), node('span', 'approval-source-label', 'Saved file'), node('pre', '', file.source));
      if (await dialog('Review saved source', content, [{ value: 'cancel', label: 'Keep editor draft' }, { value: 'reload', label: 'Replace with saved source', primary: true }]) !== 'reload') return;
    }
    loadDocument(path, file.source, file.hash);
  } finally { switching = false; }
}
async function newDocument(source = '', suggestion = 'experiment.vr') {
  if (switching) return; switching = true;
  try {
    if (!(await mayReplace())) return;
    const content = node('div'); content.append(node('p', '', 'Create a Verum source file in this workspace. Existing directories are supported.'));
    const label = node('label', '', 'File path'); const field = node('input'); field.value = suggestion; field.autocomplete = 'off'; field.spellcheck = false;
    label.append(field); content.append(label);
    if (await dialog('New experiment', content, [{ value: 'cancel', label: 'Cancel' }, { value: 'create', label: 'Create draft', primary: true }]) !== 'create') return;
    const path = relativeVerumPath(field.value);
    if (!path) throw new Error('Use a relative .vr path without hidden traversal, for example experiment.vr.');
    if (files.some(file => file.path === path)) throw new Error(`${path} already exists. Open it from the explorer or choose a different name.`);
    loadDocument(path, source, null);
    if (source === IDENTITY) { input('declaration').value = 'identity'; $('execution-settings').hidden = false; $('run-settings').setAttribute('aria-expanded', 'true'); }
  } finally { switching = false; }
}
bind('save', save); bind('refresh-files', refreshFiles); bind('new-file', () => newDocument());
bind('reload-source', reloadSource);
bind('lesson-script', () => newDocument(STARTER, 'first-experiment.vr'));
bind('lesson-proof', () => newDocument(IDENTITY, 'identity.vr'));
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
  if (!Number.isFinite(seconds) || seconds < 1 || seconds > 300) throw new Error('Choose a time budget between 1 and 300 seconds.');
  const declaration = input('declaration').value.trim();
  if (kind === 'proof' && !declaration) { $('execution-settings').hidden = false; input('declaration').focus(); throw new Error('Enter the logical declaration to check.'); }
  starting = true; ['run', 'check', 'prove'].forEach(id => { $<HTMLButtonElement>(id).disabled = true; });
  try {
    const result = await api<{ id: string }>('operation.start', { kind, source, path, ...(kind === 'proof' ? { declaration } : {}), timeout_ms: Math.round(seconds * 1000) });
    selectedReceipt = result.id; await refreshOperations(); setPane('results'); announce(`${kind} started for ${path}`);
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
    for (const receipt of receipts) if (previous.get(receipt.id) === 'running' && receipt.status !== 'running') announce(`${receipt.kind} ${receipt.status}: ${receipt.path}`);
    if (!selectedReceipt && receipts.length) selectedReceipt = receipts[receipts.length - 1].id;
    renderResults();
  } finally { operationsBusy = false; }
}
const resultLabel = (kind: string) => kind === 'proof' ? 'Proof check' : kind === 'check' ? 'Module check' : 'Execution';
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
  $('result-count').textContent = String(receipts.length); $('mobile-result-count').textContent = receipts.length ? String(receipts.length) : '';
  const running = receipts.filter(receipt => receipt.status === 'running').length;
  $('running-count').textContent = running ? `${running} running` : '';
  const nextHistory = JSON.stringify(receipts.map(receipt => ({ id: receipt.id, kind: receipt.kind, path: receipt.path, status: receipt.status, seconds: Math.floor(elapsed(receipt) / 1000) }))) + selectedReceipt;
  if (nextHistory !== historySignature) {
    const restoreFocus = restoreFocusAfterRender($('history'));
    historySignature = nextHistory; $('history').replaceChildren();
    for (const receipt of [...receipts].reverse()) {
    const entry = button('', () => { selectedReceipt = receipt.id; renderResults(); }, 'history-item');
    entry.dataset.focusKey = receipt.id;
    entry.setAttribute('aria-current', String(receipt.id === selectedReceipt));
    const row = node('span', 'history-row'); row.append(node('span', '', receipt.kind === 'proof' ? '∀ Proof' : receipt.kind === 'check' ? '✓ Check' : '▶ Run'), node('span', `badge ${receipt.status}`, receipt.status));
    entry.append(row, node('span', 'history-path', receipt.path), node('span', 'history-path', `${Math.max(0, elapsed(receipt) / 1000).toFixed(1)}s · ${receipt.source_hash.slice(0, 7)}`));
    entry.title = `${resultLabel(receipt.kind)} · ${receipt.path} · ${receipt.status}`;
      $('history').append(entry);
    }
    restoreFocus();
  }
  const receipt = receipts.find(receipt => receipt.id === selectedReceipt);
  $('result-summary').textContent = receipt ? `${resultLabel(receipt.kind)} · ${receipt.status}` : 'No operations yet';
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
  const heading = node('div', 'result-heading'); heading.append(node('strong', '', resultLabel(receipt.kind)), node('span', `badge ${receipt.status}`, receipt.status));
  if (isStale(receipt, currentPath, currentHash)) {
    const badge = node('span', 'badge stale', currentHash === null && currentPath === receipt.path ? 'Checking revision' : 'Different revision');
    badge.title = 'This result does not describe the current editor source.'; heading.append(badge);
  } else heading.append(node('span', 'badge', 'Current source'));
  if (receipt.status === 'running') {
    const cancel = button('Cancel', async () => { await api('operation.cancel', { id: receipt.id }); await refreshOperations(); }, 'quiet-button');
    cancel.dataset.focusKey = 'cancel'; heading.append(cancel);
  }
  const copy = button('Copy receipt', async () => { await navigator.clipboard.writeText(pretty(receipt)); announce('Receipt copied'); }, 'quiet-button');
  copy.dataset.focusKey = 'copy'; heading.append(copy);
  detail.append(heading, node('p', 'result-meta', `${receipt.path} · ${(elapsed(receipt) / 1000).toFixed(2)}s${receipt.exit_code == null ? '' : ` · exit ${receipt.exit_code}`} · ${receipt.source_hash.slice(0, 12)}`));
  if (receipt.timeout_ms) detail.append(node('p', 'muted', `Wall budget ${(receipt.timeout_ms / 1000).toFixed(0)}s${receipt.evidence_path ? ' · source and receipt retained locally' : ''}`));
  if (receipt.termination) detail.append(node('p', 'termination', receipt.termination));
  if (receipt.kind === 'proof') {
    const proof = node('div', 'proof-outcome');
    const report = parseArguments(receipt.proof), source = parseArguments(report.source), diagnostic = parseArguments(report.diagnostic);
    proof.append(node('h3', '', typeof report.status === 'string' ? `Checker judgment: ${report.status}` : 'Logical proof evidence'));
    if (typeof source.declaration === 'string') proof.append(node('p', '', `Declaration: ${source.declaration}`));
    if (typeof diagnostic.message === 'string') proof.append(node('p', '', diagnostic.message));
    proof.append(node('p', '', 'This judgment applies to the selected logical declaration and source revision. Runtime implementation correspondence is a separate obligation.')); detail.append(proof);
    if (receipt.proof !== undefined && receipt.proof !== null) appendDetails(detail, 'Full checker report', receipt.proof, 'proof');
  } else if (receipt.kind === 'run') detail.append(node('p', 'muted', 'Observed execution for this source revision; not a proof of all behaviors.'));
  if (receipt.stdout) {
    if (receipt.kind === 'proof' && receipt.proof) {
      const stream = node('details'); stream.dataset.key = 'raw-proof'; const summary = node('summary', '', 'Raw checker output'); summary.dataset.focusKey = 'raw-proof'; stream.append(summary, node('pre', '', receipt.stdout)); detail.append(stream);
    } else appendStream(detail, 'Standard output', receipt.stdout, 'stdout');
  }
  if (receipt.stderr) appendStream(detail, 'Diagnostics / standard error', receipt.stderr, 'stderr');
  if (!receipt.stdout && !receipt.stderr) detail.append(node('p', 'muted', receipt.status === 'running' ? 'Waiting for process output…' : 'The process produced no text output.'));
  appendDetails(detail, 'Full operation receipt', receipt, 'receipt');
  if (oldId === receipt.id) { detail.querySelectorAll<HTMLDetailsElement>('details').forEach(item => { item.open = expanded.has(item.dataset.key); }); detail.scrollTop = scroll; }
  restoreFocus();
}
function appendStream(parent: HTMLElement, label: string, text: string, className: string) {
  const stream = node('div', `result-stream ${className}`); stream.append(node('span', 'stream-label', label), node('pre', '', text)); parent.append(stream);
}
function appendDetails(parent: HTMLElement, label: string, value: unknown, key: string, open = false) {
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
function addMessage(role: 'user' | 'agent' | 'system', text: string, id?: string) {
  $('agent-welcome').hidden = true;
  const container = $('agent-messages');
  const nearBottom = container.scrollHeight - container.scrollTop - container.clientHeight < 90;
  const message = node('div', `message ${role}`); message.append(node('div', 'message-label', role === 'agent' ? 'Codex' : role === 'user' ? 'You' : 'Studio'));
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
  $('agent-model').title = threadId ? 'Model is fixed for this conversation. Start a new conversation to choose another model.' : 'Choose the model for a new conversation';
  $('agent-stop').hidden = !busy;
  $('agent-progress').textContent = pendingApprovals ? `${pendingApprovals} request${pendingApprovals === 1 ? '' : 's'} awaiting review` : busy ? 'Codex is working…' : connected ? authenticated ? 'Connected · edits and tools require review' : 'Local provider ready · sign in to start a conversation' : '';
  $('approval-count').textContent = pendingApprovals ? String(pendingApprovals) : '';
  $('agent-rail-status').textContent = pendingApprovals ? `${pendingApprovals} review` : busy ? 'Working' : authenticated ? 'Ready' : connected ? 'Sign in' : 'Offline';
  $('agent-rail').dataset.attention = String(pendingApprovals > 0);
}
function accountLabel(value: unknown): string {
  const record = parseArguments(value);
  const account = parseArguments(record.account ?? record);
  const email = typeof account.email === 'string' ? account.email : '';
  const plan = typeof account.planType === 'string' ? account.planType : '';
  if (email || plan) return [email, plan].filter(Boolean).join(' · ');
  return record.requiresOpenaiAuth === true || record.account === null ? 'Sign in to your Codex account' : 'Local Codex session connected';
}
async function connectAgent() {
  const control = $<HTMLButtonElement>('agent-connect'); control.disabled = true; control.textContent = 'Connecting…';
  try {
    const account = await api('agent.connect'); connected = true;
    const data = parseArguments(account); authenticated = data.account != null || data.requiresOpenaiAuth === false;
    $('agent-account').textContent = accountLabel(account); $('agent-controls').hidden = false;
    control.textContent = 'Refresh'; agentState();
    await loadModels();
  } finally { control.disabled = false; if (!connected) control.textContent = 'Connect'; }
}
async function loadModels() {
  try {
    const result = await api<{ data?: Array<{ id?: string; model?: string; displayName?: string; isDefault?: boolean }> }>('agent.request', { method: 'model/list', params: {} });
    const select = $<HTMLSelectElement>('agent-model'); const selected = select.value;
    select.replaceChildren(new Option('Account default model', ''));
    for (const model of result.data || []) { const id = model.id || model.model; if (id) select.add(new Option(model.displayName || id, id)); }
    select.value = selected;
  } catch (error) { addMessage('system', `Model discovery unavailable: ${errorText(error)}`); }
}
async function loginAgent() {
  const result = await api<{ authUrl: string }>('agent.login');
  const url = safeAuthUrl(result.authUrl);
  if (!url) throw new Error('The agent returned an unsupported authentication URL.');
  const content = node('div'); content.append(node('p', '', 'Continue in your provider’s secure sign-in page. Studio does not receive your password. Return here and refresh the connection after signing in.'));
  const link = node('a', 'button primary', 'Open Codex sign-in'); link.href = url; link.target = '_blank'; link.rel = 'noopener noreferrer'; content.append(link);
  await dialog('Connect your account', content, [{ value: 'close', label: 'Done' }]);
}
async function newThread(internal = false) {
  if (turnId || (sending && !internal) || pendingApprovals) throw new Error('Finish or stop the current turn and resolve requests before starting another conversation.');
  const model = $<HTMLSelectElement>('agent-model').value;
  const result = await api<{ thread?: { id: string }; id?: string }>('agent.request', { method: 'thread/start', params: model ? { model } : {} });
  const id = result.thread?.id || result.id; if (!id) throw new Error('Codex did not return a conversation ID.');
  threadId = id; messageBodies.clear();
  $('agent-messages').querySelectorAll('.message,.approval').forEach(element => element.remove());
  $('agent-welcome').hidden = false; agentState(); announce('New conversation started');
}
async function sendMessage() {
  const field = $<HTMLTextAreaElement>('agent-input'), text = field.value.trim();
  if (!text || sending || turnId) return;
  if (!authenticated) throw new Error('Sign in to your Codex account before starting a conversation.');
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
  } catch (error) { addMessage('system', `Message could not be sent: ${errorText(error)}`); if (!field.value) field.value = text; }
  finally { sending = false; agentState(); }
}
bind('agent-connect', connectAgent); bind('agent-login', loginAgent);
bind('new-thread', () => {
  if (turnId || sending || pendingApprovals) throw new Error('Finish the current turn and resolve requests before starting another conversation.');
  threadId = null; messageBodies.clear();
  $('agent-messages').querySelectorAll('.message,.approval').forEach(element => element.remove());
  $('agent-welcome').hidden = false; agentState(); $('agent-input').focus();
});
bind('agent-suggestion', () => { $<HTMLTextAreaElement>('agent-input').value = 'Explain this Verum source and suggest one useful experiment. Use Verum tools to check your claims.'; if (connected) $('agent-input').focus(); else notice('Connect Codex to start this conversation.'); });
$('agent-form').addEventListener('submit', event => { event.preventDefault(); sendMessage().catch(fail); });
$('agent-input').addEventListener('keydown', event => { if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) { event.preventDefault(); sendMessage().catch(fail); } });
bind('agent-stop', async () => {
  if (!threadId || !turnId) throw new Error('The turn is still starting. Try Stop again when its identifier arrives.');
  await api('agent.request', { method: 'turn/interrupt', params: { threadId, turnId } });
  $('agent-progress').textContent = 'Stopping the current turn…';
});

type RpcMessage = { id?: string | number; method?: string; params?: Record<string, unknown>; error?: unknown; result?: unknown };
async function pollEvents() {
  if (!connected || eventBusy) return; eventBusy = true;
  try {
    const result = await api<{ cursor: number; events: Array<{ seq: number; message: RpcMessage }>; gap: boolean }>('agent.events', { after: cursor });
    if (result.gap) addMessage('system', 'Some session events are no longer available. This conversation history is incomplete; inspect source and results before continuing.');
    for (const event of result.events) { if (event.seq > cursor) await receiveEvent(event.message); }
    cursor = result.cursor;
    $('event-count').textContent = String(eventLog.length);
    $('event-content').textContent = `${eventLogDropped ? 'Recent event excerpts (older or large payloads omitted). Full pending requests appear in their review cards.\n\n' : ''}${eventLog.join('\n\n')}`;
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
      const id = String(item.id || 'stream'); const body = messageBodies.get(id) || addMessage('agent', '', id); body.textContent = item.text;
    }
  } else if (method === 'turn/started') {
    const turn = parseArguments(params.turn); if (typeof turn.id === 'string') turnId = turn.id;
  } else if (method === 'turn/completed') {
    const turn = parseArguments(params.turn);
    if (typeof turn.id === 'string') completedTurns.add(turn.id);
    if (!turnId || !turn.id || turnId === turn.id) turnId = null;
    if (turn.status === 'failed' || turn.error) addMessage('system', `Turn failed: ${pretty(turn.error || turn.status)}`);
    else if (turn.status === 'interrupted') addMessage('system', 'Turn interrupted. Completed evidence remains in Results.');
  } else if (method === 'account/updated' || method === 'account/login/completed') {
    const account = await api('agent.request', { method: 'account/read', params: {} });
    const data = parseArguments(account); authenticated = data.account != null || data.requiresOpenaiAuth === false;
    $('agent-account').textContent = accountLabel(account);
  } else if (method === 'error') addMessage('system', pretty(params));
  agentState();
}
async function showApproval(id: string | number, method: string, params: Record<string, unknown>) {
  const key = `${typeof id}:${id}`; if (approvalIds.has(key)) return; approvalIds.add(key);
  const kind = approvalKind(method, params), args = parseArguments(params.arguments);
  const card = node('section', 'approval'); card.setAttribute('aria-label', 'Agent request awaiting review');
  card.append(node('div', 'approval-heading', kind === 'source' ? 'Review a source proposal' : kind === 'execution' ? 'Review a Verum operation' : kind === 'read' ? 'Review a workspace read' : 'Unsupported agent request'));
  const content = node('div', 'approval-content');
  let reviewAvailable = true;
  content.append(node('p', '', kind === 'unsupported' ? 'This request is outside the Verum tool boundary. Decline it to let the agent continue.' : `${String(params.tool || method)}${typeof args.path === 'string' ? ` · ${args.path}` : ''}`));
  if (kind === 'source') {
    let before = '(A new file is proposed; the server will check that it does not already exist.)';
    if (typeof args.path === 'string' && args.expected_hash !== null) {
      try { const file = await api<SourceFile>('workspace.read', { path: args.path }); before = file.source; }
      catch (error) { before = `Current source could not be read: ${errorText(error)}`; reviewAvailable = false; }
    }
    if (typeof args.source !== 'string' || typeof args.path !== 'string' || !('expected_hash' in args)) reviewAvailable = false;
    if (args.path === currentPath && dirty()) content.append(node('p', '', 'You have unsaved editor changes. Accepting writes the file on disk; your editor draft will be preserved.'));
    content.append(node('span', 'approval-source-label', 'Before · saved workspace source'), node('pre', '', before), node('span', 'approval-source-label', 'Proposed source'), node('pre', '', typeof args.source === 'string' ? args.source : '(Missing source)'));
    content.append(node('p', '', `Expected file revision: ${args.expected_hash === null ? 'new file' : String(args.expected_hash || 'not supplied')}`));
  } else appendDetails(content, 'Request details', params, 'request', kind === 'execution' || kind === 'read');
  const actions = node('div', 'approval-actions');
  const status = node('div', 'approval-status', 'Awaiting your decision. No requested action has been approved.');
  let responding = false;
  async function respond(approved: boolean) {
    if (responding) return; responding = true;
    actions.querySelectorAll('button').forEach(button => { button.disabled = true; });
    status.textContent = approved ? 'Approved · waiting for the tool result…' : 'Declining request…';
    let result: unknown;
    try { result = await api('agent.respond', { id, result: approvalReply(method, approved) }); }
    catch (error) {
      responding = false; status.textContent = `Request response failed: ${errorText(error)}`;
      actions.querySelectorAll('button').forEach(button => { button.disabled = false; }); return;
    }
    try {
      const response = parseArguments(result);
      status.textContent = !approved ? 'Request declined.' : response.success === true ? 'Tool completed. Its result is available below.' : response.success === false ? 'The tool reported a failure. Inspect the response below.' : 'Approval delivered. Inspect the tool result and evidence before treating the request as successful.';
      if (result !== undefined && result !== null) appendDetails(content, 'Tool response', result, 'response');
      pendingApprovals--; agentState();
      if (approved && kind === 'source' && typeof args.path === 'string') {
        await refreshFiles();
        if (args.path === currentPath && !dirty()) { const file = await api<SourceFile>('workspace.read', { path: args.path }); loadDocument(file.path, file.source, file.hash); }
      }
      await refreshOperations();
    } catch (error) { status.textContent += ` Refresh unavailable: ${errorText(error)}`; }
  }
  actions.append(button('Decline', () => respond(false)));
  if (kind !== 'unsupported' && reviewAvailable) actions.append(button(kind === 'source' ? 'Accept edit' : kind === 'read' ? 'Allow read' : 'Approve operation', () => respond(true), 'button primary'));
  card.append(content, actions, status); $('agent-welcome').hidden = true; $('agent-messages').append(card); pendingApprovals++; agentState();
  announce('Agent request awaiting your review');
}

bind('theme', async () => {
  const selected = await dialog('Appearance', node('p', '', 'Choose a color scheme for your workspace.'), [{ value: 'system', label: 'Follow system' }, { value: 'light', label: 'Light' }, { value: 'dark', label: 'Dark', primary: true }]);
  if (selected !== 'cancel') editor.theme(applyTheme(selected));
});
bind('help', async () => {
  const content = node('div', 'shortcut-list');
  for (const [label, shortcut] of [['Save source', '⌘ / Ctrl S'], ['Run source', '⌘ / Ctrl Enter'], ['Check source', '⌘ / Ctrl Shift Enter'], ['Find in source', '⌘ / Ctrl F'], ['Toggle explorer', '⌘ / Ctrl B'], ['Toggle collaborator', '⌘ / Ctrl Shift B'], ['Toggle results', '⌘ / Ctrl J'], ['Resize focused divider', 'Arrow keys · Home / End'], ['Collapse focused divider', 'Enter'], ['Send agent message', '⌘ / Ctrl Enter'], ['Leave editor with Tab', 'Esc, then Tab']]) content.append(node('span', '', label), node('kbd', '', shortcut));
  await dialog('Keyboard shortcuts', content, [{ value: 'close', label: 'Done', primary: true }]);
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
    const content = node('div'); content.append(node('p', '', 'Open the launch URL printed by verum studio, or paste its local access token below. The token stays in this browser tab.'));
    const label = node('label', '', 'Local access token'); const field = node('input'); field.type = 'password'; field.autocomplete = 'off'; label.append(field); content.append(label);
    if (await dialog('Connect to Verum Studio', content, [{ value: 'connect', label: 'Connect', primary: true }]) !== 'connect') return;
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
}
updateDocumentState(); scheduleHash();
initialize().catch(fail);
let lastPollError = '';
window.setInterval(() => {
  if (!hasToken()) return;
  refreshOperations().then(() => { if (lastPollError) { lastPollError = ''; $('connection').classList.add('online'); } }).catch(error => {
    $('connection').classList.remove('online');
    const text = errorText(error); if (text !== lastPollError) { lastPollError = text; notice(`Local service: ${text}`); }
  });
}, 1200);
window.setInterval(() => { pollEvents().catch(error => { const text = errorText(error); if (text !== lastPollError) { lastPollError = text; notice(`Agent event stream: ${text}`); } }); }, 700);
