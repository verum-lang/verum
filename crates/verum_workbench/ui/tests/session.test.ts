import test from 'node:test';
import assert from 'node:assert/strict';
import { StudioSession, SessionAccessError, SessionChangedError, type SessionState } from '../src/session.ts';

function deferred<T>() {
  let resolve!: (value: T) => void, reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

test('missing credentials cannot send workspace requests or unlock the session', async () => {
  let calls = 0;
  const session = new StudioSession(async () => { calls++; }, () => {});
  await assert.rejects(session.call('document.read', {}), SessionAccessError);
  await assert.rejects(session.connect('   '), SessionAccessError);
  assert.equal(session.state, 'locked'); assert.equal(calls, 0);
});

test('a token unlocks only after server validation, and is used for each request', async () => {
  const handshake = deferred<unknown>(), states: SessionState[] = [];
  const calls: unknown[] = [];
  const session = new StudioSession(async (token, method, params) => {
    calls.push([token, method, params]);
    return method === 'operation.list' ? handshake.promise : 'contents';
  }, state => states.push(state));
  const connecting = session.connect('  actual-token  ');
  assert.equal(session.state, 'checking');
  await assert.rejects(session.call('document.read', {}), SessionAccessError);
  handshake.resolve({ receipts: [] }); await connecting;
  assert.equal(await session.call('document.read', { path: 'main.vr' }), 'contents');
  assert.deepEqual(states, ['checking', 'connected']);
  assert.deepEqual(calls, [['actual-token', 'operation.list', { known: {} }], ['actual-token', 'document.read', { path: 'main.vr' }]]);
});

test('invalid credentials keep the session locked', async () => {
  const session = new StudioSession(async () => { throw new SessionAccessError(); }, () => {});
  await assert.rejects(session.connect('wrong'), SessionAccessError);
  assert.equal(session.state, 'locked');
  await assert.rejects(session.call('document.read', {}), SessionAccessError);
});

test('expired access locks the session and rejects an older in-flight read', async () => {
  const pending = deferred<unknown>();
  const session = new StudioSession(async (_token, method) => {
    if (method === 'document.read') return pending.promise;
    if (method === 'expired') throw new SessionAccessError();
    return {};
  }, () => {});
  await session.connect('first');
  const read = session.call('document.read', {});
  await assert.rejects(session.call('expired', {}), SessionAccessError);
  assert.equal(session.state, 'locked');
  pending.resolve('obsolete contents'); await assert.rejects(read, SessionChangedError);
  await session.connect('second'); assert.equal(session.state, 'connected');
});

test('an obsolete handshake or refusal cannot change a newer authenticated session', async () => {
  for (const refused of [false, true]) {
    const old = deferred<unknown>();
    const session = new StudioSession(async token => token === 'old' ? old.promise : {}, () => {});
    const first = session.connect('old');
    await session.connect('new');
    if (refused) old.reject(new SessionAccessError()); else old.resolve({});
    await assert.rejects(first, SessionChangedError);
    assert.equal(session.state, 'connected');
    assert.deepEqual(await session.call('document.read', {}), {});
  }
});

test('a late API refusal cannot revoke a replacement connection', async () => {
  const old = deferred<unknown>();
  const session = new StudioSession(async (token, method) => token === 'old' && method === 'document.read' ? old.promise : {}, () => {});
  await session.connect('old');
  const read = session.call('document.read', {});
  await session.connect('new');
  old.reject(new SessionAccessError()); await assert.rejects(read, SessionChangedError);
  assert.equal(session.state, 'connected');
});

test('ordinary operation failures do not invalidate a verified credential', async () => {
  const session = new StudioSession(async (_token, method) => {
    if (method === 'document.read') throw new Error('File unavailable');
    return {};
  }, () => {});
  await session.connect('valid');
  await assert.rejects(session.call('document.read', {}), /File unavailable/);
  assert.equal(session.state, 'connected');
});
