"""Finite source counts, exact pricing and strict native census controls."""
import gzip
import hashlib
import json
from pathlib import Path
import re
import tempfile
import unittest
from check_compositional_census import emit
from check_typed_programs import ROOT, check, require_success, run
from replay_assertions import ReplayAssertions

STEM='compositional-census-native-v2'

def fixture():
    raw=gzip.decompress((ROOT/'fixtures'/(STEM+'.jsonl.gz')).read_bytes())
    return dict(packets=[json.loads(line) for line in raw.splitlines()])

class CompositionalCensusTests(ReplayAssertions,unittest.TestCase):
    emitter=staticmethod(emit)

    @classmethod
    def setUpClass(cls):require_success(run('lake','build','VerumExternalReplay.CompositionalFamily'))

    def test_frozen_census(self):
        self.assertEqual(self.prove(fixture()),dict(declarations=4,controls=1,components=3,claims=4))

    def test_source_and_receipt_identity(self):
        root=ROOT/'fixtures';manifest=json.loads((root/(STEM+'.provenance.json')).read_text())
        receipt=json.loads(gzip.decompress((root/(STEM+'.receipt.json.gz')).read_bytes()))
        self.assertEqual(receipt['status'],'passed')
        self.assertTrue(all(s['status']=='passed' and s['returncode']==0 for s in receipt['stages']))
        self.assertEqual(manifest['native_binary_sha256'],receipt['kan_binary']['sha256'])
        for item in manifest['retained_files']:
            raw=gzip.decompress((root/item['file']).read_bytes())
            self.assertEqual(len(raw),item['bytes'])
            self.assertEqual(hashlib.sha256(raw).hexdigest(),item['sha256'])
            if 'source_path' in item:
                self.assertEqual(item['sha256'],receipt['source'][item['source_path']]['sha256'])
        self.assertEqual(hashlib.sha256(gzip.decompress((root/(STEM+'.jsonl.gz')).read_bytes())).hexdigest(),
                         manifest['export_sha256'])

    def test_undercounting_and_underpricing_fail_in_kernel(self):
        for kind in ('family','child_literals','root_operands','price','look','body_extent','fold_width'):
            data=fixture();p=data['packets'][0];root=p['parts'][2]['language'];body=root['controls'][0]['body']
            select='family = some '
            if kind=='family':p['family_names']=4
            elif kind=='child_literals':body['literals'].append({'Bit':False})
            elif kind=='root_operands':root['inputs']=3
            elif kind=='price':p['charged_comparisons']=24;select='(24 : Nat) = 24 *'
            elif kind=='look':p['look']='1';select='(48 : Nat) = 24 *'
            elif kind=='body_extent':body['extent']['steps']=2
            else:body['inputs']=1
            with self.subTest(kind=kind):self.prove(data,select)

    def test_ambiguous_declarations_are_refused(self):
        for kind in ('owner','cycle','version','extent','boolean_arity','control','decimal'):
            data=fixture();p=data['packets'][0];root=p['parts'][2]['language'];body=root['controls'][0]['body']
            if kind=='owner':body['owner']=0
            elif kind=='cycle':body['owner']=root['owner']
            elif kind=='version':root['version']=True
            elif kind=='extent':body['extent']={'kind':'AllFinite'}
            elif kind=='boolean_arity':body['operations'][0]['arity']=True
            elif kind=='control':root['controls'][0]['kind']='Unknown'
            else:p['look']='00'
            with self.subTest(kind=kind),self.assertRaises(ValueError):emit(data)

    def test_general_enumeration_and_branch_contract(self):
        body='''open CompositionalFamily
example : (selections [2,3,2]).length = 12 := by decide_cbv
example : (selections [2,0,3]).isEmpty = true := by decide_cbv
example : (selections []).length = 1 := by decide_cbv
example : count [⟨1,[]⟩,⟨2,[]⟩] 2 0 1 = 6 := by decide_cbv
example : count [⟨1,[]⟩,⟨2,[]⟩] 2 0 2 = 150 := by decide_cbv
example : count [⟨2,[6]⟩] 2 0 1 = 24 := by decide_cbv
example : count [⟨2,[2,3]⟩] 3 0 1 = 54 := by decide_cbv
example : control [⟨0,1,2⟩,⟨1,1,3⟩] (.branch 0 1) = some ⟨2,[2,3]⟩ := by decide_cbv
example : control [⟨0,1,2⟩,⟨1,2,3⟩] (.branch 0 1) = none := by decide_cbv
example : control [⟨0,1,2⟩] (.fold 0) = none := by decide_cbv
example : census [] [⟨0,2,0,1,[],[.fold 0]⟩] = none := by decide_cbv
example : census [] [⟨0,0,0,1,[0],[]⟩,⟨0,0,0,1,[0],[]⟩] = none := by decide_cbv
'''
        with tempfile.TemporaryDirectory(prefix='verum-family-laws-') as tmp:
            p=Path(tmp)/'laws.lean'
            p.write_text('import VerumExternalReplay.CompositionalFamily\nset_option warningAsError true\n'+body)
            check(p)

    def test_exact_axiom_roster(self):
        names=re.findall(r'^theorem (\w+)',(ROOT/'VerumExternalReplay/CompositionalFamily.lean').read_text(),re.M)
        with tempfile.TemporaryDirectory(prefix='verum-family-axioms-') as tmp:
            p=Path(tmp)/'axioms.lean'
            p.write_text('import VerumExternalReplay.CompositionalFamily\n'+
                         '\n'.join('#print axioms CompositionalFamily.'+n for n in names))
            result=check(p)
        rows=[d['data'] for d in result['diagnostics'] if d['severity']=='information']
        self.assertEqual(len(rows),len(names))
        for row in rows:self.assertTrue(row.endswith(('does not depend on any axioms',
            'depends on axioms: [propext]','depends on axioms: [propext, Quot.sound]')),row)

if __name__=='__main__':unittest.main()
