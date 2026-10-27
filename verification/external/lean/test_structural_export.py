"""Structural codec controls and finite frozen witnesses, not a universal native certificate."""
import copy
import gzip
import json
from pathlib import Path
import tempfile
import unittest
import structural_export
from check_typed_programs import ROOT, check, require_success, run
from observer_export_replay import batches

HEADER = '''import VerumExternalReplay.StructuralNames
import VerumExternalReplay.AcquiredObserver
open StructuralNames AcquiredObserver
set_option warningAsError true
namespace FrozenObserverExport
'''
MODEL = HEADER + '''
def child : FiniteCode.Code Op Value := .mk [⟨.primitive .zero, [], none⟩] (some 0) []
def g : Grammar Op Value := ⟨42, 1, some 3, [(.identity, 1), (.pair, 2)], [.nat 7], [(1, child)]⟩
def a : Address := ⟨42, [⟨0, [0]⟩, ⟨1, [1,2]⟩, ⟨2, [2]⟩], 1⟩
-- gamma(3), then opcode/operand fields, then a NONFINAL selected root:
-- 011 | 00 0 | 01 01 10 | 10 10 | 01
def w : List Bool := [false,true,true,false,false,false,false,true,false,true,true,false,true,false,true,false,false,true]
def expected : FiniteCode.Code Op Value := .mk [⟨.primitive .identity, [.input 0], none⟩,
  ⟨.primitive .pair, [.register 0, .literal (.nat 7)], none⟩, ⟨.call 0, [.register 1], none⟩] (some 1) [child]
'''


class StructuralTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.StructuralNames',
                            'VerumExternalReplay.AcquiredObserver'))
        cls.native = json.loads(gzip.decompress((ROOT / 'fixtures/observer-joint-v3.json.gz').read_bytes()))

    def compile(self, source, rejection=None):
        with tempfile.TemporaryDirectory(prefix='verum-structural-codec-') as directory:
            p = Path(directory) / 'claims.lean'; p.write_text(source)
            return check(p, rejection)

    def extraction(self, data):
        lines = [HEADER.strip()]; names = []
        def claim(equation, proof='rfl'):
            name = 's' + str(len(names)); names.append(name)
            lines.append(f'theorem {name} : {equation} := by\n  {proof}')
        self.assertEqual(structural_export.emit_witnesses(data, lines, claim), 3)
        return '\n\n'.join(lines + ['#print axioms ' + name for name in names] + ['end FrozenObserverExport'])

    def test_native_structural_witnesses(self):
        source = self.extraction(self.native)
        for part, names in batches(source):
            result = self.compile(part)
            rosters = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
            self.assertEqual(len(rosters), len(names))
            for roster in rosters:
                self.assertTrue(roster.endswith('does not depend on any axioms') or
                                roster.endswith('depends on axioms: [propext]') or
                                roster.endswith('depends on axioms: [propext, Quot.sound]'))

    def test_multistep_literal_register_call_and_root(self):
        source = MODEL + '''
example : decode g w = some a := by rfl
example : encode g a = some w := by rfl
example : materialize g a = some expected := by rfl
example : decode g (w ++ [true]) = none := by rfl
example : decode {g with limit := none} w = some a := by rfl
example : decode {g with limit := some 2} w = none := by rfl
example : (List.range 9).map width = [0,0,1,2,2,3,3,3,3] := by rfl
example : decode ({g with operations := [], calls := []} : Grammar Op Value) [true] = none := by rfl
example : materialize g ⟨42, [], 0⟩ = none := by rfl
#print axioms StructuralNames.verified_name_injective
#print axioms StructuralNames.decoded_materialization_unique
#print axioms StructuralNames.foreign_owner_refused
#print axioms StructuralNames.foreign_owner_not_encoded
end FrozenObserverExport
'''
        result = self.compile(source)
        rosters = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
        self.assertEqual(len(rosters), 4)
        for roster in rosters:
            self.assertTrue(roster.endswith('depends on axioms: [propext]') or
                            roster.endswith('depends on axioms: [propext, Quot.sound]'))

    def test_false_codec_claims_are_rejected(self):
        false = {
            'trailing': 'decode g (w ++ [false]) = some a',
            'truncated': 'decode g (w.take 17) = some a',
            'foreign': 'materialize g {a with owner := 43} = some expected',
            'arity': 'materialize g {a with instructions := [⟨0, []⟩], output := 0} = some expected',
            'forward_reference': 'materialize g {a with instructions := [⟨0, [2]⟩], output := 0} = some expected',
            'root': 'encode g {a with output := 3} = some w',
            'extent': 'decode {g with limit := some 2} w = some a',
            'overflow': 'materialize {g with inputs := 2^64-1} a = some expected',
            'unused_opcode': 'decode g [true,true,true,false,false] = some ⟨42,[⟨3,[0]⟩],0⟩',
            'wrong_literal': 'materialize {g with literals := [.nat 8]} a = some expected',
            'changed_callee': 'materialize {g with calls := [(1, .mk [] none [])]} a = some expected',
        }
        for name, equation in false.items():
            with self.subTest(name=name):
                self.compile(MODEL + f'example : {equation} := by rfl\nend FrozenObserverExport\n', 'Tactic `rfl` failed:')

    def test_changed_native_observations_rejected(self):
        for change in ('word', 'decoded_owner', 'original_owner', 'program', 'decoded_program',
                       'consumed', 'arity', 'grammar_owner_reuse', 'binding_slots', 'audit', 'extent'):
            data = copy.deepcopy(self.native); c = data['candidate']; w = c['structural_witnesses'][1]
            if change == 'word':
                w['word'][-1] ^= 1
                c['structural_code_words'][1][-1] ^= 1  # Coherent bit change still must decode to original address.
            elif change == 'decoded_owner': w['decoded_address']['owner'] = 99
            elif change == 'original_owner': w['address']['owner'] = 99
            elif change == 'program': w['materialized']['steps'][0]['operation']['Primitive'] = 'EventInput'
            elif change == 'decoded_program': w['decoded_materialized']['output'] = None
            elif change == 'consumed': w['consumed_bits'] -= 1
            elif change == 'arity': w['language']['operations'][0]['arity'] = 0
            elif change == 'grammar_owner_reuse':
                for key in ('language', 'address', 'decoded_address'): w[key]['owner'] = 0
            elif change == 'binding_slots': data['definition']['update_binding']['slots'] = [{'External': 0}]
            elif change == 'audit': w['audit']['decode_work'] = w['audit']['allowance'] + 1
            else: w['language']['extent']['steps'] = 0
            with self.subTest(change=change):
                self.compile(self.extraction(data),
                    'Tactic `decide` proved that the proposition' if change == 'audit' else 'Tactic `rfl` failed:')

    def test_incomplete_or_unknown_schema_rejected(self):
        for change in ('witness', 'field', 'kind', 'bit', 'literal', 'operation', 'address_field'):
            data = copy.deepcopy(self.native); w = data['candidate']['structural_witnesses'][0]
            if change == 'witness': data['candidate']['structural_witnesses'].pop()
            elif change == 'field': w['trusted'] = True
            elif change == 'kind': w['language']['extent']['kind'] = 'UnboundedTrust'
            elif change == 'bit': w['word'][0] = True
            elif change == 'literal': w['language']['literals'] = [{'kind': 'Trusted'}]
            elif change == 'operation': w['language']['operations'][0]['primitive'] = 'Trusted'
            else: w['decoded_address']['trusted'] = True
            with self.subTest(change=change), self.assertRaises(ValueError): self.extraction(data)


if __name__ == '__main__':
    unittest.main()
