"""Frozen native codec correspondence and semantic refusal controls.

These fixtures are actual native outputs. Mutations are test controls and never
count as a native observation or proof of general Rust execution semantics.
"""
import copy
import gzip
import hashlib
import json
from pathlib import Path
import unittest

from check_prefix_export import Catalog, emit, emit_components
from lean_test_fixture import IsolatedLeanTest
from observer_export_replay import batches

FIXTURES = Path(__file__).resolve().parent / 'fixtures'


def frozen(name):
    raw = gzip.decompress((FIXTURES / (name+'.json.gz')).read_bytes())
    provenance = json.loads((FIXTURES / (name+'.provenance.json')).read_text())
    if hashlib.sha256(raw).hexdigest() != provenance['export_sha256']:
        raise ValueError('Frozen prefix fixture hash mismatch')
    return json.loads(raw)


def joint(version=3):
    return frozen(f'prefix-generated-v{version}')


def components(version=3):
    return frozen(f'prefix-generated-controls-v{version}')


class PrefixReplay(IsolatedLeanTest):
    MODULES = ('PrefixSyntax',)

    def test_frozen_joint_and_actual_lexical_recursive_components(self):
        cases = [(joint(v), lambda data,v=v: emit(data,[v]*3)) for v in (1,2,3)]
        cases += [(components(v), lambda data,v=v: emit_components(data,v)) for v in (2,3)]
        for data, emitter in cases:
            source, counts = emitter(data)
            with self.subTest(versions=counts['versions'],components=counts['components']):
                if 'candidate' in data:
                    # A V3 catalog by itself is not recursive-path coverage.
                    self.assertEqual(counts['selected_controls'], {})
                else:
                    self.assertGreater(counts['selected_controls']['Branch'],0)
                    self.assertGreater(counts['selected_controls']['Fold'],0)
                    self.assertEqual(counts['joint_candidate_receipts'],0)
                    if data['language_version']==3:
                        self.assertEqual(counts['selected_controls']['Recur'],2)
                        self.assertIn({'depth':1,'kind':'Recur','ancestor':1,'member':None,'arity':2},
                                      counts['selected_events']['nested_parent_recur'])
                checked = 0
                for part,names in batches(source):
                    self.assert_axioms(self.prove(part),['FrozenObserverExport.'+n for n in names])
                    checked += len(names)
                self.assertEqual(checked,counts['claims'])

    def reject_claim(self, source, match, *, definitional=False):
        selected = [(part,names) for part,names in batches(source,size=1)
                    if any(line.startswith('theorem ') and match in line for line in part.splitlines())]
        self.assertEqual(len(selected),1,match)
        if definitional:
            self.prove(selected[0][0],failure=True,expected_error='Tactic `rfl` failed:')
        else:
            self.prove(selected[0][0],failure=True)

    def test_changed_native_word_body_owner_suffix_and_price_fail_semantically(self):
        for change in ('child_order','foreign_owner','word','body','suffix','price','arity','ancestor'):
            data = components(); sample=data['samples']['nested_parent_recur']
            w=sample['structural_witness']
            if change=='child_order':
                w['address']['children'].reverse()
            elif change=='foreign_owner':
                w['address']['owner']=99
            elif change=='word':
                w['word'][1] ^= 1
            elif change=='body':
                w['materialized']['library'][1]['steps'][0]['operation']['Recur']['ancestor']=0
            elif change=='suffix':
                sample['prefix_receipt']['suffix'][0]=252
            elif change=='price':
                sample['component_name_denominator']=str(int(sample['component_name_denominator'])//2)
            else:
                # Change the same complete catalog in every specimen. The word
                # must still be rechecked against that catalog and native body.
                for row in data['samples'].values():
                    def alter(g):
                        for control in g.get('controls',[]):
                            if control['kind']=='Recur' and control['ancestor']==1:
                                control['arity' if change=='arity' else 'ancestor'] += 1
                            if control['kind']=='Branch':
                                alter(control['yes']);alter(control['no'])
                            elif control['kind']=='Fold':alter(control['body'])
                    alter(row['structural_witness']['language'])
            source,_=emit_components(data,3)
            # Match the statement, independent of generated theorem numbering.
            if change in ('child_order','foreign_owner','word','arity'):
                match=' catalog 0 word2 = some (address2, [])'
            elif change in ('body','ancestor'):
                # Three body observations are distinct obligations; target only
                # the first (native materialized body) to demand one semantic error.
                targets=[line for line in source.splitlines() if line.startswith('theorem ') and 'catalog 0 address2 = some (.mk' in line]
                match=targets[0]
            elif change=='suffix':match='input2.drop 1 = word2 ++'
            else:match='= 2^word2.length'
            with self.subTest(change=change):
                self.reject_claim(source,match,definitional=change not in ('suffix','price'))

    def test_refusal_boundaries_preserve_holes_and_suffixes(self):
        self.prove('''import VerumExternalReplay.PrefixSyntax
open PrefixSyntax
set_option warningAsError true
def g : Grammar := ⟨0,0,some 1,1,[("Unit",0),("Zero",0),("Empty",0)],[],[],[]⟩
example : readBits 0 [255] 0 = some (0,[255]) := by decide_cbv
example : readBits 1 [2] 0 = none := by decide_cbv
example : readBits 1 [1] (2^63) = none := by decide_cbv
example : readGamma (List.replicate 64 0 ++ [1] ++ List.replicate 64 0) 0 = none := by decide_cbv
example : decode 1 [g] 0 [1,1,1] = none := by rfl
example : decode 1 [g] 0 [1,1] = none := by rfl
example : decode 1 [g] 0 [1,0,1,255] = some (.mk 0 [⟨1,[]⟩] 0 [],[255]) := by rfl
example : decode 1 [g] 9 [1,0,1] = none := by rfl
example : materialize 1 [g] 0 (.mk 0 [⟨0,[]⟩] 1 []) = none := by rfl
example : materialize 1 [g] 0 (.mk 0 [⟨0,[0]⟩] 0 []) = none := by rfl
example : materialize 1 [g] 0 (.mk 0 [⟨0,[]⟩] 0 [(0,0,.mk 0 [⟨0,[]⟩] 0 [])]) = none := by rfl
''')


class PrefixIntake(unittest.TestCase):
    def test_unexercised_controls_are_not_coverage(self):
        with self.assertRaisesRegex(ValueError,'Required selected'):
            emit(joint(),[3]*3,['Recur'])
        with self.assertRaisesRegex(ValueError,'Required selected'):
            emit_components(components(2),2,['Recur'])

    def test_mutated_source_authority_and_schema_are_refused(self):
        changes = [
            lambda d:d['candidate']['provenance'].update(owner=7),
            lambda d:d['candidate']['grammar'].update(count=1),
            lambda d:d['candidate'].update(declared_triples=[]),
            lambda d:d['candidate']['structural_code_words'][0].append(2),
            lambda d:d.update(language_versions=[3,3,True]),
            lambda d:d['candidate'].update(evidence_look=True),
        ]
        for change in changes:
            data=joint();change(data)
            with self.subTest(change=change),self.assertRaises(ValueError):emit(data,[3]*3)
        data=components();data['samples'].pop('root_recur')
        with self.assertRaisesRegex(ValueError,'Incomplete'):emit_components(data,3)
        data=components();data['enumeration_complete']=True
        with self.assertRaisesRegex(ValueError,'not exhaustive'):emit_components(data,3)

    def test_owner_catalog_and_lexical_cycles_cannot_be_laundered(self):
        grammar=joint()['candidate']['structural_witnesses'][0]['language']
        catalog=Catalog();catalog.register(grammar)
        changed=copy.deepcopy(grammar);changed['operations'][0]['arity']=1
        with self.assertRaisesRegex(ValueError,'Same owner changed'):catalog.register(changed)
        cyclic=copy.deepcopy(grammar);cyclic['controls'][0]['yes']=cyclic
        with self.assertRaisesRegex(ValueError,'Cyclic'):Catalog().register(cyclic)


if __name__=='__main__':unittest.main()
