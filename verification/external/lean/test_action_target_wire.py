"""Kernel-checked wire laws, capacity boundaries and false codec claims."""
from pathlib import Path
import re
import tempfile
import unittest

from check_typed_programs import ROOT, check, require_success, run

HEADER = '''import VerumExternalReplay.ActionTargetWire
open AcquiredObserver ActionCapture ActionTargetWire
set_option warningAsError true
set_option maxRecDepth 4096
def outcome : Sensory := ⟨[0, 255], [7, 0, 255]⟩
def birth : Sensory := ⟨[0, 255], [9, 0, 255]⟩
def captured : Capture UInt8 Unit :=
  ⟨7, .ok (.terminal .ended .resumed), some outcome, some birth⟩
def target : Target := ⟨.terminal .ended, outcome⟩
def empty : Target := ⟨.step, ⟨[], []⟩⟩
'''


class ActionTargetWireTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.ActionTargetWire'))

    def prove(self, source, expected=None):
        with tempfile.TemporaryDirectory(prefix='action-target-wire-') as directory:
            path = Path(directory)/'check.lean'
            path.write_text(HEADER + source)
            return check(path, expected)

    def test_universal_axiom_roster(self):
        names = re.findall(r'^theorem (\w+)', (ROOT/'VerumExternalReplay/ActionTargetWire.lean').read_text(), re.M)
        for required in ('target_roundtrip', 'encoding_injective',
                         'successful_materialization_decodes', 'materialization_within_capacity'):
            self.assertIn(required, names)
        result = self.prove('\n'.join('#print axioms ActionTargetWire.'+name for name in names))
        rosters = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
        self.assertEqual(len(rosters), len(names))
        for row in rosters:
            self.assertTrue(row.endswith(('does not depend on any axioms',
                'depends on axioms: [propext]', 'depends on axioms: [propext, Quot.sound]')), row)

    def test_roundtrip_includes_full_alphabet_and_u64_lengths(self):
        self.prove('''
example : readLittleEndian (littleEndian 8 (2^64-1)) = 2^64-1 := by rfl
example : littleEndian 8 256 = [0, 1, 0, 0, 0, 0, 0, 0] := by rfl
example : decode (encode target) = some target := by rfl
example : decode (encode empty) = some empty := by rfl
def alphabet : List UInt8 := (List.range 256).map UInt8.ofNat
def full : Target := ⟨.terminal .death, ⟨alphabet, alphabet.reverse⟩⟩
example : decode (encode full) = some full := by
  apply target_roundtrip
  simp [Fits, full, alphabet]
example : (encode full).length = 530 := by simp [encoded_length, full, alphabet]
example : materialize 23 captured = .ok (encode target) := by rfl
''')

    def test_capacity_and_refusal_priority(self):
        self.prove('''
example : materialize 22 captured = .error .capacity := by rfl
example : materialize 0 {captured with observation := none} = .error .missing := by rfl
example : materialize 0 {captured with result := .error ⟨.act 7, none, .action, ()⟩} =
  .error .unobserved := by rfl
example : materialize (Detail := Unit) 18 {captured with result := .ok .step, observation := some ⟨[], []⟩} =
  .ok (encode empty) := by rfl
example : materialize 17 {captured with observation := some ⟨[], []⟩} = .error .capacity := by rfl
example : materialize 23 {captured with resumed := none} = materialize 23 captured := by rfl
''')

    def test_malformed_truncated_and_noncanonical_wires(self):
        self.prove('''
example : decode [] = none := by rfl
example : decode [1] = none := by rfl
example : decode ([2, 5] ++ frame outcome.schema ++ frame outcome.body) = none := by rfl
example : decode ([1, 255] ++ frame outcome.schema ++ frame outcome.body) = none := by rfl
example : readFrame [0, 0, 0, 0, 0, 0, 0] = none := by rfl
example : readFrame (littleEndian 8 3 ++ [1, 2]) = none := by rfl
example : decode ((encode target).take 22) = none := by rfl
example : decode (encode target ++ [0]) = none := by rfl
''')

    def test_ambiguous_split_kind_birth_and_truncation_claims_fail(self):
        claims = [
            'encode ⟨.step, ⟨[1], [2]⟩⟩ = encode ⟨.step, ⟨[], [1, 2]⟩⟩',
            'encode ⟨.step, outcome⟩ = encode ⟨.refused, outcome⟩',
            'encode ⟨.terminal .death, outcome⟩ = encode ⟨.terminal .ended, outcome⟩',
            'materialize 23 captured = .ok (encode ⟨.terminal .ended, birth⟩)',
            'materialize 22 captured = .ok ((encode target).take 22)',
            'materialize 0 {captured with observation := none} = .error .capacity',
            'decode (encode target ++ [0]) = some target',
            'readLittleEndian (littleEndian 8 (2^64)) = 2^64',
        ]
        for claim in claims:
            with self.subTest(claim=claim):
                self.prove(f'example : {claim} := by rfl', 'Tactic `rfl` failed:')


if __name__ == '__main__':
    unittest.main()
