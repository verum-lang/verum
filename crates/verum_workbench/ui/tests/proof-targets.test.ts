import test from 'node:test';
import assert from 'node:assert/strict';
import { ProofTargets } from '../src/proof-targets.ts';
import type { Analysis, Declaration } from '../src/language.ts';

const logical = 'verum-source-pi-id/1', finite = 'verum-source-finite/1';
const range = { start: { line: 0, character: 0 }, end: { line: 0, character: 1 } };
const declaration = (name: string, kind = 'theorem', contract = false): Declaration => ({
  id: name, name, kind, range, selection_range: range, requirements: [],
  postconditions: contract ? [{ source: 'true', range }] : [],
});
const analysis = (...declarations: Declaration[]): Analysis => ({
  source_hash: 'hash', version: 1, parsed: true, analysis_scope: 'module', diagnostics: [], declarations, mounts: [],
});
const one = { path: 'one.vr', source: 'one' }, two = { path: 'two.vr', source: 'two' };

test('proof scope and target belong to a document, never the last globally selected function', () => {
  const targets = new ProofTargets();
  targets.begin(one); targets.accept(one, analysis(declaration('id')));
  assert.deepEqual(targets.request(one), { profile: logical, declaration: 'id' });
  targets.begin(two);
  assert.equal(targets.request(two), null);
  targets.accept(two, analysis(declaration('publish', 'function', true)));
  assert.equal(targets.request(two), null, 'a finite scope must be chosen explicitly');
  targets.select(two, { profile: finite, declaration: '' });
  assert.deepEqual(targets.request(two), { profile: finite, declaration: 'publish' });
  targets.begin(one); targets.accept(one, analysis(declaration('id')));
  assert.deepEqual(targets.request(one), { profile: logical, declaration: 'id' });
  targets.begin(two); targets.accept(two, analysis(declaration('publish', 'function', true)));
  assert.deepEqual(targets.request(two), { profile: finite, declaration: 'publish' });
});

test('pending analysis, source edits, different files and removed targets cannot authorize proof actions', () => {
  const targets = new ProofTargets();
  targets.begin(one); targets.accept(one, analysis(declaration('id')));
  const changed = { ...one, source: 'renamed' };
  assert.equal(targets.request(changed), null);
  assert.equal(targets.request({ ...one, path: 'other.vr' }), null);
  targets.begin(changed);
  assert.equal(targets.request(changed), null);
  assert.equal(targets.accept(one, analysis(declaration('id'))), false);
  targets.accept(changed, analysis(declaration('other')));
  assert.equal(targets.request(changed), null, 'renaming must not silently select another target');
  targets.select(changed, { profile: logical, declaration: 'other' });
  assert.equal(targets.request(changed)?.declaration, 'other');
  targets.begin(changed);
  assert.equal(targets.request(changed), null, 'even unchanged source awaits the requested refresh');
});

test('ambiguous names, axioms and uncontracted functions are not offered as proof targets', () => {
  const targets = new ProofTargets();
  targets.begin(one);
  targets.accept(one, analysis(declaration('dup'), declaration('dup', 'function', true),
    declaration('assumed', 'axiom'), declaration('ordinary', 'function'),
    declaration('lemma', 'lemma'), declaration('corollary', 'corollary'),
    declaration('bounded', 'function', true)));
  assert.deepEqual(targets.options(one, logical)?.map(item => item.name), ['lemma', 'corollary']);
  assert.equal(targets.request(one), null, 'multiple targets require an explicit choice');
  assert.deepEqual(targets.options(one, finite)?.map(item => item.name), ['bounded']);
  targets.select(one, { profile: logical, declaration: 'assumed' });
  assert.equal(targets.request(one), null);
  targets.select(one, { profile: logical, declaration: 'lemma' });
  assert.equal(targets.request(one)?.declaration, 'lemma');
  targets.begin(one); targets.accept(one, { ...analysis(declaration('lemma')), parsed: false });
  assert.equal(targets.request(one), null, 'a partial AST from a failed parse is not usable');
});

test('locking a session retires proof choices before a different workspace can connect', () => {
  const targets = new ProofTargets();
  targets.begin(one); targets.accept(one, analysis(declaration('publish', 'function', true)));
  targets.select(one, { profile: finite, declaration: 'publish' });
  targets.clear();
  assert.equal(targets.request(one), null);
  assert.deepEqual(targets.selection(one.path), { profile: logical, declaration: '' });
  targets.begin(one); targets.accept(one, analysis(declaration('id')));
  assert.deepEqual(targets.request(one), { profile: logical, declaration: 'id' });
});
