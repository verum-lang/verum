// node e2e/conversations.mjs /path/to/verum-studio /path/to/chrome-headless-shell [/path/to/verum]
// VERUM_STUDIO_E2E_STARTUP_MS extends only the packaged server's cold-start budget.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { once } from 'node:events';
import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';

const [binary, browser, compiler] = process.argv.slice(2);
assert.ok(binary && browser, 'Pass the packaged Studio and Chromium executable paths');
const verum = compiler ? resolve(compiler) : 'verum';
const root = await mkdtemp(join(tmpdir(), 'verum-conversations-'));
const workspace = join(root, 'project');
const children = [];
function launch(executable, args, stdio) {
  const child = spawn(resolve(executable), args, {stdio});
  children.push(child);
  return child;
}
try {
  await mkdir(workspace);
  await writeFile(join(workspace, 'experiment.vr'), 'fn main() -> Int { 42 }');
  const provider = fileURLToPath(new URL('./conversation-provider.mjs', import.meta.url));
  const studio = launch(binary, ['--workspace', workspace, '--codex', provider, '--verum', verum], ['ignore','pipe','inherit']);
  const startupMs = Number(process.env.VERUM_STUDIO_E2E_STARTUP_MS ?? 15000);
  const deadline = AbortSignal.timeout(startupMs);
  const [startup] = await Promise.race([
    once(createInterface({input: studio.stdout}), 'line', {signal: deadline}),
    once(studio, 'exit', {signal: deadline}).then(([code, signal]) => {
      throw Error('Studio exited before publishing its launch URL: ' + (signal || code));
    }),
  ]).catch(error => {
    if (deadline.aborted) throw Error('Studio startup exceeded ' + startupMs + ' ms');
    throw error;
  });
  const url = new URL(startup.replace(/^Verum Studio: /, ''));
  const chrome = launch(browser, ['--remote-debugging-pipe', '--no-first-run', '--disable-background-networking', '--user-data-dir=' + join(root, 'profile')], ['ignore','ignore','ignore','pipe','pipe']);
  const stream = chrome.stdio[4].setEncoding('utf8');
  const pending = new Map();
  let buffer = '', sequence = 0;
  stream.on('data', chunk => {
    const messages = (buffer + chunk).split('\0');
    buffer = messages.pop();
    for (const line of messages) {
      const message = JSON.parse(line);
      pending.get(message.id)?.(message);
    }
  });
  function rpc(method, params = {}, sessionId) {
    return new Promise((resolve, reject) => {
      const id = ++sequence;
      const timer = setTimeout(() => { pending.delete(id); reject(Error('Chromium timeout: ' + method)); }, 10000);
      pending.set(id, message => {
        clearTimeout(timer); pending.delete(id);
        if (message.error) reject(Error(message.error.message)); else resolve(message.result);
      });
      chrome.stdio[3].write(JSON.stringify({id, method, params, sessionId}) + '\0');
    });
  }
  const {targetId} = await rpc('Target.createTarget', {url: 'about:blank'});
  const {sessionId} = await rpc('Target.attachToTarget', {targetId, flatten: true});
  async function evaluate(expression) {
    const result = await rpc('Runtime.evaluate', {expression, returnByValue: true}, sessionId);
    if (result.exceptionDetails) throw Error(result.exceptionDetails.exception?.description || result.exceptionDetails.text);
    return result.result.value;
  }
  async function wait(expression, timeout = 15000) {
    const deadline = Date.now() + timeout;
    do { if (await evaluate(`Boolean(${expression})`)) return; await delay(75); } while (Date.now() < deadline);
    throw Error('Studio did not reach ' + expression);
  }
  await rpc('Emulation.setDeviceMetricsOverride', {width: 1440, height: 900, deviceScaleFactor: 1, mobile: false}, sessionId);
  await rpc('Page.navigate', {url: url.href}, sessionId);
  await wait('document.querySelector("#agent-connect") && !document.querySelector("#app").inert && document.querySelector(".cm-content")');
  await evaluate(`window.byId = id => document.getElementById(id);
    window.choose = n => { byId('agent-conversation').selectedIndex = n; byId('agent-conversation').dispatchEvent(new Event('change')); };
    window.compose = text => { byId('agent-input').value = text; byId('agent-form').requestSubmit(); };
    window.transcript = () => byId('agent-messages').innerText;
    if (byId('agent-body').hidden) byId('agent-restore').click();
    byId('agent-connect').click();`);
  await wait("byId('agent-input').getClientRects().length > 0 && !byId('agent-send').disabled");
  await evaluate("compose('Inspect author source')");
  await wait("transcript().includes('author response')");
  await evaluate("byId('agent-input').value = 'author draft'; byId('new-thread').click()");
  assert.equal(await evaluate("byId('agent-input').value"), '');
  await evaluate("byId('agent-role').value = 'reviewer'; byId('agent-role').dispatchEvent(new Event('change')); compose('Review saved code'); choose(0)");
  await wait("byId('agent-progress').innerText.includes('Other conversations awaiting review')");
  assert.equal(await evaluate("byId('agent-input').value"), 'author draft');
  assert.doesNotMatch(await evaluate('transcript()'), /reviewer response|Review a workspace read/);
  // Even a programmatic activation on the hidden reviewer card must be inert.
  await evaluate("byId('agent-messages').querySelector('.approval .primary').click(); byId('agent-stop').click()");
  await wait("transcript().includes('Turn interrupted')");
  await evaluate('choose(1)');
  assert.match(await evaluate('transcript()'), /reviewer response/);
  await wait("byId('agent-sequence').dataset.status === 'incomplete' && byId('agent-sequence').textContent.includes('Awaiting a decision: 1')");
  assert.doesNotMatch(await evaluate('transcript()'), /author response|Turn interrupted/);
  assert.equal(await evaluate("byId('agent-role').value"), 'reviewer');
  await evaluate("byId('agent-messages').querySelector('.approval .primary').click()");
  await wait("transcript().includes('Tool completed') && byId('agent-stop').hidden");
  await wait("byId('agent-sequence').dataset.status === 'checked' && byId('agent-sequence').textContent.includes('Succeeded: 1 · failed: 0')");
  assert.match(await evaluate("byId('agent-messages').querySelector('.approval details:last-of-type').textContent"), /fn main\(\) -> Int/);
  await evaluate('choose(0)');
  assert.equal(await evaluate("byId('agent-input').value"), 'author draft');

  // Hold selected real HTTP responses to exercise journal recovery and late UI delivery.
  await evaluate(`window.card = i => document.querySelectorAll('#agent-messages > div:not([hidden]) > .approval')[i];
    window.fields = el => Object.fromEntries([...el.querySelectorAll('dt')].map(dt => [dt.textContent, dt.nextElementSibling.textContent]));
    window.language = value => { byId('language').value = value; byId('language').dispatchEvent(new Event('change')); };
    window.previews = []; window.receiptReads = []; window.checkReviews = [];
    const send = window.fetch.bind(window);
    let journalReady;
    const recovery = new Promise(resolve => { journalReady = resolve; });
    window.fetch = async (url, options) => {
      const {method, params} = JSON.parse(options.body);
      if (method === 'operation.read') receiptReads.push(params.id);
      if (method === 'agent.respond' && params.threadId === '4') checkReviews.push(params.reviewId);
      const scopedPoll = method === 'agent.events' && params.threadId === '3';
      if (scopedPoll) await recovery;
      const response = await send(url, options);
      if (method === 'agent.request' && params.method === 'turn/start' && params.params.threadId === '3') journalReady();
      if (scopedPoll) {
        const {result} = await response.clone().json();
        if (result.recoveredReviews.length) window.recoveredId = result.recoveredReviews[0].reviewId;
        if (window.latePreviewReturned) window.polledAfterReturn = true;
      }
      if (method === 'agent.preview') {
        previews.push(params);
        if (window.holdPreview) {
          await new Promise(resolve => { window.releasePreview = resolve; });
          window.latePreviewReturned = true;
        }
      }
      return response;
    };
    byId('new-thread').click();`);
  await wait("byId('agent-conversation').selectedIndex === 2");
  await evaluate("compose('Review execution')");
  await wait("window.recoveredId && card(0)?.querySelector('.primary')");
  const source = 'print("<b>λ</b>");\r\n';
  assert.deepEqual(await evaluate('fields(card(0))'), {
    Operation: 'Execution', 'File path': 'experiment.vr', 'Source mode': 'Script entry', 'Time limit': '60,000 ms',
    'Script permissions': 'Source / CLI defaults', 'Runtime diagnostics': 'Off',
    Executable: verum, 'Source SHA-256': createHash('sha256').update(source).digest('hex'),
  });
  assert.deepEqual(await evaluate('previews[0]'), { reviewId: await evaluate('recoveredId'), threadId: '3' });
  assert.equal(await evaluate("card(0).querySelector('details').open"), false);
  await evaluate("card(0).querySelector('summary').click(); language('ru')");
  assert.equal(await evaluate("card(0).querySelector('details pre').textContent"), source);
  assert.equal(await evaluate("fields(card(0))['Разрешения скрипта']"), 'Из исходника / настройки CLI');
  await evaluate("language('en'); card(0).querySelector('.approval-actions button').click()");
  await wait("card(1)?.querySelector('.preview-error')");
  assert.match(await evaluate('card(1).textContent'), /Execution plan unavailable.*Timeout/s);
  assert.equal(await evaluate("card(1).querySelector('.primary')"), null);
  await evaluate("window.holdPreview = true; card(1).querySelector('.approval-actions button').click()");
  await wait("window.releasePreview && card(1).textContent.includes('Request declined.') && card(2)");
  assert.equal(await evaluate("card(2).querySelector('.primary')"), null);
  await evaluate("byId('agent-stop').click()");
  await wait("card(2).textContent.includes('This review is no longer pending.')");
  await evaluate('releasePreview()');
  await wait('window.polledAfterReturn');
  await wait("byId('agent-sequence').dataset.status === 'checked' && byId('agent-sequence').textContent.includes('declined: 2')");
  assert.equal(await evaluate("card(2).querySelector('.primary, .approval-plan')"), null);
  assert.equal(await evaluate("card(2).querySelector('button').disabled"), true);
  assert.equal(await evaluate('new Set(previews.map(p => p.reviewId)).size'), 3);
  await evaluate("window.holdPreview = false; byId('new-thread').click()");
  await wait("byId('agent-conversation').selectedIndex === 3");
  await evaluate("compose('Check both source revisions')");
  for (const index of [0, 1]) {
    await wait(`card(${index})?.querySelector('.primary')`);
    await evaluate(`card(${index}).querySelector('.primary').click()`);
    await wait(`card(${index})?.querySelector('[data-key="response"] pre')`, 75000);
    const result = await evaluate(`JSON.parse(card(${index}).querySelector('[data-key="response"] pre').textContent).tool_result`);
    assert.equal(result.status, index ? 'failed' : 'succeeded', result.error || result.stderr);
    assert.equal(await evaluate(`card(${index}).querySelector('.approval-actions button').textContent`), 'Open operation receipt');
  }
  await wait("card(2)?.querySelector('.primary') && byId('agent-sequence').dataset.status === 'incomplete'");
  assert.equal(await evaluate("card(2).querySelector('.primary').textContent"), 'Allow inspection');
  await evaluate("card(2).querySelector('.primary').click()");
  await wait("card(2)?.querySelector('[data-key=\"response\"] pre')");
  const inspection = await evaluate("JSON.parse(card(2).querySelector('[data-key=\"response\"] pre').textContent)");
  assert.equal(inspection.success, true);
  assert.equal(inspection.tool_result.threadId, '4');
  assert.equal(inspection.tool_result.actionSequence.status, 'checked');
  assert.deepEqual([inspection.tool_result.actionSequence.succeeded, inspection.tool_result.actionSequence.failed,
    inspection.tool_result.actionSequence.retained], [1, 1, 6]);
  const answered = await evaluate('checkReviews');
  assert.equal(new Set(answered).size, 3);
  assert.equal(inspection.tool_result.actionSequence.inspecting_review, answered[2]);
  // The browser's next observation includes the completed inspection itself.
  await wait("byId('agent-sequence').dataset.status === 'checked' && byId('agent-sequence').textContent.includes('Succeeded: 2 · failed: 1')");
  await evaluate(`window.receipt = () => JSON.parse(byId('result-detail').querySelector('[data-key="receipt"] pre')?.textContent || 'null');
    window.fromCard = i => JSON.parse(card(i).querySelector('[data-key="response"] pre').textContent).tool_result;
    language('ru');`);
  assert.equal(await evaluate("byId('sequence-status').textContent"), 'Порядок действий · проверено');
  for (const index of [1, 0]) {
    assert.equal(await evaluate(`card(${index}).querySelector('.approval-actions button').textContent`), 'Открыть отчёт об операции');
    await evaluate(`card(${index}).querySelector('.approval-actions button').click()`);
    await wait(`byId('operations-tab').getAttribute('aria-selected') === 'true' && receipt()?.id === fromCard(${index}).id && document.activeElement?.dataset.focusKey === receipt().id`);
    assert.equal(await evaluate('receiptReads.at(-1)'), await evaluate('receipt().id'));
    assert.equal(await evaluate('receipt().status'), index ? 'failed' : 'succeeded');
    assert.equal(await evaluate('receipt().agent.review_id'), await evaluate(`previews[${index + 3}].reviewId`));
  }
  console.log('Passed concurrent conversations, scoped reviews, execution previews, exact receipt navigation, and host action-order checks through both the browser and the reviewed agent tool.');
} finally {
  await Promise.all(children.map(async child => {
    if (child.exitCode !== null || child.signalCode !== null) return;
    const exited = once(child, 'exit');
    child.kill(); await exited;
  }));
  await rm(root, {recursive: true, force: true, maxRetries: 3});
}
