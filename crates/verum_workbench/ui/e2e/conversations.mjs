// node e2e/conversations.mjs /path/to/verum-studio /path/to/chrome-headless-shell
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';

const [binary, browser] = process.argv.slice(2);
assert.ok(binary && browser, 'Pass the packaged Studio and Chromium executable paths');
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
  const studio = launch(binary, ['--workspace', workspace, '--codex', provider], ['ignore','pipe','inherit']);
  const [startup] = await once(createInterface({input: studio.stdout}), 'line', {signal: AbortSignal.timeout(15000)});
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
  async function wait(expression) {
    const deadline = Date.now() + 15000;
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
  assert.doesNotMatch(await evaluate('transcript()'), /author response|Turn interrupted/);
  assert.equal(await evaluate("byId('agent-role').value"), 'reviewer');
  await evaluate("byId('agent-messages').querySelector('.approval .primary').click()");
  await wait("transcript().includes('Tool completed') && byId('agent-stop').hidden");
  assert.match(await evaluate("byId('agent-messages').querySelector('.approval details:last-of-type').textContent"), /fn main\(\) -> Int/);
  await evaluate('choose(0)');
  assert.equal(await evaluate("byId('agent-input').value"), 'author draft');
  console.log('Passed: concurrent author and reviewer, independent drafts and streamed item IDs, background review, owner-bound interrupt and approved workspace read.');
} finally {
  await Promise.all(children.map(async child => {
    if (child.exitCode !== null || child.signalCode !== null) return;
    const exited = once(child, 'exit');
    child.kill(); await exited;
  }));
  await rm(root, {recursive: true, force: true, maxRetries: 3});
}
