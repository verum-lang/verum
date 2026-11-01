import test from 'node:test';
import assert from 'node:assert/strict';
import { proofSourceRange } from '../src/proof.ts';
import { sourceByteRange, sourceHash } from '../src/source.ts';
import { rangeOffsets } from '../src/language.ts';
import type { Receipt } from '../src/model.ts';

const source = '// λ🙂\r\nfn t(x:Bool)->Bool { helper(x) }\n';
async function evidence() {
  const hash = await sourceHash(source), call = 'helper(x)', at = source.indexOf(call);
  const start = new TextEncoder().encode(source.slice(0, at)).length;
  const span = { start, end: start + call.length, sha256: await sourceHash(call) };
  const receipt: Receipt = { id: 'proof', kind: 'proof', path: 't.vr', source_hash: hash,
    status: 'succeeded', stdout: '', stderr: '', exit_code: 0, elapsed_ms: 1,
    declaration: 't', proof_profile: 'verum-source-finite/1',
    proof: { profile: 'verum-source-finite/1', status: 'verified', source: { declaration: 't', sha256: hash } } };
  return { receipt, span, hash, at, call };
}

test('proof navigation maps hashed UTF-8 ranges to exact Unicode editor positions', async () => {
  const {receipt,span,hash,at,call} = await evidence();
  const range = await proofSourceRange(receipt,span,'t.vr',hash,source);
  assert.ok(range);
  assert.deepEqual(rangeOffsets(source,range), { from:at, to:at+call.length });
  const bytes = new TextEncoder().encode(source);
  assert.equal(bytes.length,source.length+3);
  assert.equal(sourceByteRange(source,3,5)?.text,'λ');
  assert.equal(sourceByteRange(source,5,9)?.text,'🙂');
  assert.equal(sourceByteRange(source,9,9)?.text,'');
  assert.equal(sourceByteRange('\ufeffx',0,3)?.text,'\ufeff');
});

test('partial characters, reversed ranges, CRLF interiors and malformed source refuse', () => {
  for (const [start,end] of [[4,5],[5,8],[9,8],[0,9999],[-1,0],[1.5,2],[10,10]]) {
    assert.equal(sourceByteRange(source,start,end),null,`${start}..${end}`);
  }
  assert.equal(sourceByteRange('\ud800',0,0),null);
  assert.equal(sourceByteRange(source,'0',1),null);
});

test('stale source, declaration, profile, evidence status and span hashes never navigate', async () => {
  const {receipt,span,hash} = await evidence();
  for (const [path,currentHash,text] of [['other.vr',hash,source],['t.vr',null,source],
    ['t.vr','stale',source],['t.vr',hash,source+'// changed']]) {
    assert.equal(await proofSourceRange(receipt,span,path!,currentHash,text!),null);
  }
  assert.equal(await proofSourceRange(receipt,{...span,sha256:'wrong'},'t.vr',hash,source),null);
  for (const change of ['declaration','profile','source_hash','status']) {
    const changed = structuredClone(receipt);
    const report = changed.proof as { profile:string; status:string; source:{declaration:string;sha256:string} };
    if (change==='declaration') report.source.declaration='other';
    else if (change==='profile') report.profile='verum-source-pi-id/1';
    else if (change==='source_hash') report.source.sha256='stale';
    else changed.status='inconclusive';
    assert.equal(await proofSourceRange(changed,span,'t.vr',hash,source),null,change);
  }
});

test('validated rejected diagnostics can reveal an exact insertion point', async () => {
  const {receipt,hash} = await evidence();
  receipt.status='failed'; (receipt.proof as {status:string}).status='rejected';
  const span = { start:0,end:0,sha256:await sourceHash('') };
  assert.deepEqual(await proofSourceRange(receipt,span,'t.vr',hash,source),
    {start:{line:0,character:0},end:{line:0,character:0}});
});
