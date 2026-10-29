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

def captured_program():
    return dict(steps=[dict(operation={'Primitive':'Zero'},arguments=[])],output=0,library=[])

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

    def test_invalid_declarations_fail_before_arity_projection(self):
        for kind in ('duplicate_primitive','duplicate_bit','duplicate_sequence','empty'):
            data=fixture();p=data['packets'][0];body=p['parts'][2]['language']['controls'][0]['body']
            if kind=='duplicate_primitive':
                # Different arities must not conceal identical primitive IDs.
                body['operations'][1]['primitive']='Identity';count=24
            elif kind=='empty':
                body['operations']=[];count=0
            else:
                value={'Bit':False} if kind=='duplicate_bit' else {'Sequence':[False,True]}
                body['literals']=[value,value];count=80
            # Each reported count is correct for the old arity-only projection.
            # Rejection must therefore come from declaration validity itself.
            p['family_names']=count;p['charged_comparisons']=count*2
            with self.subTest(kind=kind):self.prove(data,'family = some ')

    def test_distinct_values_equal_arities_and_captured_calls_are_admitted(self):
        for kind in ('same_arity','distinct_values','duplicate_calls','control_program'):
            data=fixture();p=data['packets'][0];body=p['parts'][2]['language']['controls'][0]['body']
            if kind=='same_arity':
                body['operations'][1]['arity']=1;count=16
            elif kind=='distinct_values':
                body['literals']=[{'Bit':False},{'Sequence':[]},{'Sequence':[False]},{'Bit':True}]
                count=168
            elif kind=='duplicate_calls':
                body['calls']=[dict(arity=0,program=captured_program()) for _ in range(2)];count=32
            else:
                # A typed captured Program need not succeed at execution; all
                # constructors and its lexical library must still be decoded.
                child=captured_program()
                child['steps'].extend([
                    dict(operation={'Branch':{'yes':0,'no':1}},arguments=[{'Literal':{'Bit':True}}]),
                    dict(operation={'Call':0},arguments=[{'Input':0}]),
                    dict(operation={'Fold':1},arguments=[{'Register':0},{'Literal':{'Sequence':[False]}}])])
                child['library']=[captured_program()]
                body['calls']=[dict(arity=0,program=child)];count=28
            p['family_names']=count;p['charged_comparisons']=count*2
            with self.subTest(kind=kind):self.prove(data)

    def test_source_value_codec_is_strict(self):
        for value in ('Unknown',1,None,{'Identity':None}):
            data=fixture();data['packets'][0]['parts'][0]['language']['operations'][0]['primitive']=value
            with self.subTest(primitive=value),self.assertRaises(ValueError):emit(data)
        for value in ({'Bit':1},{'Sequence':[False,0]},{'Sequence':'01'},
                      {'Other':False},{'Bit':False,'Sequence':[]},False):
            data=fixture();data['packets'][0]['parts'][0]['language']['literals']=[value]
            with self.subTest(literal=value),self.assertRaises(ValueError):emit(data)

    def test_captured_program_payloads_are_not_ignored(self):
        for kind in ('fields','output','library','primitive','operation','branch',
                     'callee','fold_object','callee_overflow','argument','literal','nested','arguments','steps'):
            data=fixture();child=captured_program();step=child['steps'][0]
            if kind=='fields':child['extra']=0
            elif kind=='output':child['output']=True
            elif kind=='library':child['library']={}
            elif kind=='primitive':step['operation']={'Primitive':'Unknown'}
            elif kind=='operation':step['operation']={'Unknown':0}
            elif kind=='branch':step['operation']={'Branch':{'yes':0,'no':False}}
            elif kind=='callee':step['operation']={'Fold':-1}
            elif kind=='fold_object':step['operation']={'Fold':{'body':0}}
            elif kind=='callee_overflow':step['operation']={'Call':2**64}
            elif kind=='argument':step['arguments']=[{'Register':True}]
            elif kind=='literal':step['arguments']=[{'Literal':{'Bit':0}}]
            elif kind=='nested':child['library']=[dict(steps=[],output=None)]
            elif kind=='arguments':step['arguments']={}
            else:child['steps']=None
            data['packets'][0]['parts'][0]['language']['calls']=[dict(arity=0,program=child)]
            with self.subTest(kind=kind),self.assertRaises(ValueError):emit(data)

    def test_branch_condition_must_fit_native_width(self):
        data=fixture();p=data['packets'][0];root=p['parts'][2]['language']
        body=root['controls'][0]['body']
        body['inputs']=2**64-1;body['extent']['steps']=0
        root['inputs']=0;root['operations']=[dict(primitive='Zero',arity=0)]
        root['controls']=[dict(kind='Branch',yes=body,no=body)]
        # The child family is empty. Its zero contribution must not conceal an
        # invalid branch declaration behind the parent's valid Zero alternative.
        p['index']='0';p['family_names']=1;p['charged_comparisons']=2
        self.prove(data,'family = some ')

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
example : branchArity 8 6 6 = some 7 := by decide_cbv
example : branchArity 8 7 7 = none := by decide_cbv
example : branchArity 8 3 4 = none := by decide_cbv
example : control [⟨0,2^64-2,0⟩] (.branch 0 0) = some ⟨2^64-1,[0,0]⟩ := by decide_cbv
example : control [⟨0,2^64-1,0⟩] (.branch 0 0) = none := by decide_cbv
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
