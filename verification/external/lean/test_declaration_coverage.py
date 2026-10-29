"""Finite lexical-tree coverage, independent bounds and strict refusal controls."""
import gzip
import hashlib
import json
from pathlib import Path
import re
import tempfile
import unittest
from check_typed_programs import ROOT, check, require_success, run
from replay_assertions import ReplayAssertions

STEM='declaration-coverage-native-v1'
DEFINITIONS='''import VerumExternalReplay.DeclarationCoverage

open DeclarationCoverage

set_option warningAsError true

namespace FrozenObserverExport

def primitives : List Nat := [0,1,2]

def zero (width : Nat) : Tree := .node width [⟨.primitive 0,[]⟩] 0 []

def pick (width index : Nat) : Tree := .node width [⟨.primitive 1,[index]⟩] 0 []

def two (width : Nat) : Tree := .node width [⟨.primitive 0,[]⟩,⟨.primitive 1,[width]⟩] 1 []

def branch : Tree := .node 3 [⟨.branch 2,[0,1,2]⟩] 0 [pick 2 0,two 2]

def nested : Tree := .node 3 [⟨.fold 3,[1,0,2]⟩] 0 [branch]

def mixed : Tree := .node 3 [⟨.branch 0,[0]⟩,⟨.fold 2,[1,2]⟩] 1 [zero 0,zero 0,pick 2 0]

'''

def emit(expression):
    return DEFINITIONS+f'theorem c0 : {expression} := by decide_cbv\n\nend FrozenObserverExport\n',{'claims':1}

class DeclarationCoverageTests(ReplayAssertions,unittest.TestCase):
    emitter=staticmethod(emit)

    @classmethod
    def setUpClass(cls):require_success(run('lake','build','VerumExternalReplay.DeclarationCoverage'))

    def test_positive_bounds_and_distinct_branch_bodies(self):
        expressions=[
            'nested.needs = (⟨2,2,3⟩ : Needs)',
            'covers primitives 0 2 3 2 nested = true',
            'covers primitives 0 4 6 5 nested = true',
            'covers primitives 0 1 0 0 (zero 100) = true',
            'covers primitives 0 1 0 1 (.node 1 [⟨.branch 0,[0]⟩] 0 [zero 0,zero 0]) = true',
            'covers primitives 1 1 0 0 (pick 0 0) = true',
            'covers primitives 0 2 2 1 mixed = true',
            'catalogue 3 1 3 = [.primitive 0,.primitive 1,.primitive 2,.branch 0,.branch 1,.branch 2,.fold 2,.branch 3,.fold 3]']
        for expression in expressions:
            with self.subTest(expression=expression):self.prove(expression)

    def test_each_bound_is_independently_necessary(self):
        for steps,width,depth in ((1,3,2),(2,2,2),(2,3,1)):
            with self.subTest(bounds=(steps,width,depth)):
                self.prove(f'covers primitives 0 {steps} {width} {depth} nested = true','covers ')

    def test_malformed_trees_fail_in_kernel(self):
        trees={
            'empty':'.node 0 [] 0 []',
            'output':'.node 0 [⟨.primitive 0,[]⟩] 1 []',
            'future_register':'.node 0 [⟨.primitive 1,[0]⟩] 0 []',
            'missing_primitive':'.node 0 [⟨.primitive 3,[]⟩] 0 []',
            'arity':'.node 0 [⟨.primitive 1,[]⟩] 0 []',
            'fold_width':'.node 1 [⟨.fold 1,[0]⟩] 0 [zero 1]',
            'branch_missing_body':'.node 3 [⟨.branch 2,[0,1,2]⟩] 0 [pick 2 0]',
            'branch_wrong_width':'.node 3 [⟨.branch 2,[0,1,2]⟩] 0 [pick 2 0,zero 1]',
            'branch_invalid_second_body':'.node 3 [⟨.branch 2,[0,1,2]⟩] 0 [pick 2 0,pick 2 2]',
            'child_permutation':'.node 3 [⟨.branch 0,[0]⟩,⟨.fold 2,[1,2]⟩] 1 [zero 0,pick 2 0,zero 0]',
            'deep_invalid_body':'.node 3 [⟨.fold 3,[1,0,2]⟩] 0 [.node 3 [⟨.branch 2,[0,1,2]⟩] 0 [pick 2 0,pick 2 2]]',
            'extra_child':'.node 0 [⟨.primitive 0,[]⟩] 0 [zero 0]',
            'bad_fold_capture':'.node 3 [⟨.fold 3,[0,1,3]⟩] 0 [branch]'}
        for label,tree in trees.items():
            with self.subTest(label=label):
                self.prove(f'covers primitives 0 4 5 4 ({tree}) = true','covers ')

    def test_generic_coverage_theorem_applies(self):
        source=DEFINITIONS+'''
theorem bounds_are_independent (steps depth width : Nat)
    (enough : nested.needs.Within ⟨steps,depth,width⟩) :
    covers primitives 0 steps width depth nested = true :=
  finite_tree_coverage _ _ _ _ _ nested
    (coverage_sound primitives 0 2 3 2 nested (by decide_cbv)) enough

#print axioms bounds_are_independent
end FrozenObserverExport
'''
        self.audit(source,1)

    def audit(self,source,count):
        with tempfile.TemporaryDirectory(prefix='verum-declaration-laws-') as directory:
            p=Path(directory)/'laws.lean';p.write_text(source);result=check(p)
        rows=[d['data'] for d in result['diagnostics'] if d['severity']=='information']
        self.assertEqual(len(rows),count)
        for row in rows:self.assertTrue(row.endswith(('does not depend on any axioms',
            'depends on axioms: [propext]','depends on axioms: [propext, Quot.sound]')),row)

    def test_exact_axiom_roster(self):
        source=(ROOT/'VerumExternalReplay/DeclarationCoverage.lean').read_text()
        names=re.findall(r'^theorem (\w+)',source,re.M)
        self.audit('import VerumExternalReplay.DeclarationCoverage\n'+
            '\n'.join('#print axioms DeclarationCoverage.'+name for name in names),len(names))

    def test_frozen_source_identity(self):
        root=ROOT/'fixtures';manifest=json.loads((root/(STEM+'.provenance.json')).read_text())
        receipt=json.loads(gzip.decompress((root/(STEM+'.receipt.json.gz')).read_bytes()))
        self.assertEqual(manifest['status'],'source_identity_verified')
        self.assertEqual(receipt['status'],'passed')
        self.assertTrue(all(s['status']=='passed' and s['returncode']==0 for s in receipt['stages']))
        self.assertEqual(manifest['native_binary_sha256'],receipt['kan_binary']['sha256'])
        for item in manifest['retained_files']:
            raw=gzip.decompress((root/item['file']).read_bytes())
            self.assertEqual(len(raw),item['bytes'])
            self.assertEqual(hashlib.sha256(raw).hexdigest(),item['sha256'])
            if 'source_path' in item:
                self.assertEqual(item['sha256'],receipt['source'][item['source_path']]['sha256'])

if __name__=='__main__':unittest.main()
