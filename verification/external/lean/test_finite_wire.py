"""Proper finite wire mass, alignment and explicit counterexample to conservation alone."""
from pathlib import Path
import re
import tempfile
import unittest

from check_typed_programs import ROOT, check, require_success, run

HEADER = '''import VerumExternalReplay.FiniteWire
open ExactMass EnumerableMass CausalCoding FiniteWire
set_option warningAsError true

def fair : Kernel Unit where
  probability _ _ := half 1
  next _ _ := ()
  normalized _ := by decide_cbv

def bits : Model Unit where
  unitBits := 1
  positiveWidth := by decide
  length := geometric
  kernel _ := fair
  initial _ := ()
  prediction _ := []

def bytes : Model Unit := { bits with unitBits := 8, positiveWidth := by decide }
def leakWeight (n : Nat) := mul (half 1) (geometric.weight n)
def leakTail (n : Nat) := add (half 1) (mul (half 1) (geometric.remaining n))
'''


class FiniteWireTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.FiniteWire'))

    def prove(self, source, expected=None):
        with tempfile.TemporaryDirectory(prefix='verum-finite-wire-') as directory:
            path = Path(directory) / 'check.lean'
            path.write_text(HEADER + source)
            return check(path, expected)

    def test_axiom_roster(self):
        names = re.findall(r'^theorem (\w+)',
            (ROOT / 'VerumExternalReplay/FiniteWire.lean').read_text(), re.M)
        names += ['geometric', 'inducedLengthLaw']
        self.assertIn('geometric_tail', names)
        result = self.prove('\n'.join('#print axioms FiniteWire.' + name for name in names))
        rows = [row['data'] for row in result['diagnostics'] if row['severity'] == 'information']
        self.assertEqual(len(rows), len(names))
        for row in rows:
            self.assertTrue(row.endswith(('does not depend on any axioms',
                'depends on axioms: [propext]', 'depends on axioms: [propext, Quot.sound]')), row)

    def test_scoring_complete_finite_bits_and_bytes(self):
        self.prove('''
example : score bits [] = some (bodyMass bits 0 []) := by decide_cbv
example : Equal (bodyMass bits 0 []) (half 1) := by decide_cbv
example : Equal (bodyMass bits 1 [false]) (half 3) := by decide_cbv
example : score bytes [false] = none := by decide_cbv
example : score bytes (List.replicate 8 false) = some (bodyMass bytes 1 (List.replicate 8 false)) := by decide_cbv
example : Equal (bodyMass bytes 1 (List.replicate 8 false)) (half 10) := by decide_cbv
example (n : Nat) : Equal (sliceMass bytes n) (half (n+1)) := slice_exact bytes n
example : Equal (partialSum (sliceMass bits) 3) ⟨7, 8, by decide⟩ := by decide_cbv
example (n : Nat) : Equal (add (partialSum (sliceMass bytes) n) (half n)) one := wire_prefix_exact bytes n
example (n : Nat) (large : 1000 ≤ n) : Less (half n) (unit 1000 (by decide)) :=
  geometric_tail _ (by decide) n large
example : AtMost (partialSum (selectedSlice bits (fun b => b == [false])) 3) one :=
  selected_prefix_bounded bits _ 3
''')

    def test_conservation_can_leak_positive_mass_forever(self):
        self.prove('''
-- Half the geometric mass plus a persistent half has exact conservation.
example (n : Nat) : Equal (add (partialSum leakWeight n) (leakTail n)) one := by
  apply eq_trans (add_congr_right _ (prefix_scale _ _ n))
  apply eq_trans (eq_symm (add_assoc _ _ _))
  apply eq_trans (add_congr_right _ (add_comm _ _))
  apply eq_trans (add_assoc _ _ _)
  apply eq_trans (add_congr_left _ (eq_symm (mul_add _ _ _)))
  apply eq_trans (add_congr_left _ (mul_congr_left _ (geometric.conserved n)))
  apply eq_trans (add_congr_left _ (mul_one _))
  decide_cbv
example (n : Nat) : 0 < (leakWeight n).numerator := by decide_cbv
-- No proper length law can carry that residual, despite the conservation law.
example : ¬ ∃ law : LengthLaw, law.remaining = leakTail := by
  rintro ⟨law, same⟩
  apply no_persistent_tail law (half 1) (by decide)
  intro n
  rw [same]
  exact le_add _ _
''')

    def test_weakened_claims_rejected(self):
        claims = {
            'infinite_tail_is_not_zero': 'Less (leakTail 100) (half 1)',
            'body_mass_must_pay_for_length': 'Equal (bodyMass bits 1 [false]) (half 1)',
            'empty_body_has_length_mass': 'Equal (bodyMass bits 0 []) one',
            'partial_byte_refused': '(score bytes [false]).isSome = true',
            'byte_length_differs_from_bit_length':
                'Equal (bodyMass bytes 1 (List.replicate 8 false)) (half 17)',
            'finite_cut_is_not_whole_space': 'Equal (partialSum (sliceMass bits) 2) one',
            'empty_subprobability_not_normalized': 'Equal (partialSum (fun _ => zero) 3) one',
            'unit_mass_per_length_is_improper': 'AtMost (partialSum (fun _ => one) 2) one',
        }
        for name, claim in claims.items():
            with self.subTest(name=name):
                self.prove(f'example : {claim} := by decide_cbv\n',
                           '`decide_cbv` failed: the proposition evaluates to `false`')


if __name__ == '__main__':
    unittest.main()
