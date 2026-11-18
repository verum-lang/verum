import test from 'node:test';
import assert from 'node:assert/strict';
import { sourceHash } from '../src/source.ts';
import { quotedHandoff, verifiedHandoff, type Handoff } from '../src/handoff.ts';

const from = { connectionId: 'codex-connection', threadId: 'shared' };
const to = { connectionId: 'agy-connection', threadId: 'shared' };
const text = '> literal <b>λ</b>\r\n\nnext\n';
const message: Handoff = { id: 'envelope', from, to, seq: 9, turnId: 'T', itemId: 'I', text, sha256: await sourceHash(text) };

test('a handoff binds exact UTF-8 text and both provider connections even for matching thread IDs', async () => {
  assert.equal(await verifiedHandoff(message, { from, to, seq: 9 }), message);
  for (const candidate of [{ ...message, from: to }, { ...message, to: from }, { ...message, seq: 8 },
    { ...message, text: text.trim() }, { ...message, sha256: 'unknown' }, { ...message, itemId: '' }]) {
    assert.equal(await verifiedHandoff(candidate, { from, to, seq: 9 }), null);
  }
});

test('inserting a handoff preserves blank lines, visible attribution and the source digest', () => {
  assert.equal(quotedHandoff(message, 'Quoted message from Codex'),
    'Quoted message from Codex\n' + JSON.stringify(from) + '\nSHA-256: ' + message.sha256 + '\n> > literal <b>λ</b>\r\n> \n> next\n> ');
});
