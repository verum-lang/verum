"""Proper learned byte-length law: universal proofs and strict false controls."""
from pathlib import Path
import re
import tempfile
import unittest

from check_typed_programs import ROOT, check, require_success, run

HEADER = '''import VerumExternalReplay.AdaptiveLength
open ExactMass EnumerableMass FiniteWire AdaptiveLength
set_option warningAsError true
def learned : Counts := [(0, 2), (1, 3), (4, 1)]
'''
FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'


class AdaptiveLengthTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.AdaptiveLength'))

    def prove(self, source, expected=None):
        with tempfile.TemporaryDirectory(prefix='verum-adaptive-length-') as directory:
            path = Path(directory) / 'check.lean'
            path.write_text(HEADER + source)
            result = check(path, expected)
            if expected:
                errors = [d for d in result['diagnostics'] if d['severity'] == 'error']
                self.assertEqual(len(errors), 1, errors)
            return result

    def test_axiom_roster(self):
        names = re.findall(r'^theorem (\w+)',
            (ROOT / 'VerumExternalReplay/AdaptiveLength.lean').read_text(), re.M)
        names += ['ledger', 'law']
        result = self.prove('\n'.join('#print axioms AdaptiveLength.' + n for n in names))
        rows = [r['data'] for r in result['diagnostics'] if r['severity'] == 'information']
        self.assertEqual(len(rows), len(names))
        for row in rows:
            self.assertTrue(row.endswith(('does not depend on any axioms',
                'depends on axioms: [propext]', 'depends on axioms: [propext, Quot.sound]')), row)

    def test_exact_masses_and_tail(self):
        self.prove('''
example : total learned = 6 := by decide_cbv
example : Equal (weight learned 0) ⟨5,14,by decide⟩ := by decide_cbv
example : Equal (weight learned 1) ⟨13,28,by decide⟩ := by decide_cbv
example : Equal (weight learned 2) ⟨1,56,by decide⟩ := by decide_cbv
example : Equal (remaining learned 1) ⟨9,14,by decide⟩ := by decide_cbv
example : Equal (remaining learned 4) ⟨17,112,by decide⟩ := by decide_cbv
example : Equal (remaining learned 5) ⟨1,224,by decide⟩ := by decide_cbv
example : Equal (remaining learned 0) one := by decide_cbv
example (n : Nat) : Equal (add (partialSum (weight learned) n) (remaining learned n)) one :=
  normalized_prefix learned n
example (e : Fraction) (positive : 0 < e.numerator) :
    ∃ bound, ∀ n, bound ≤ n → Less (remaining learned n) e := tail_vanishes learned e positive
example (n : Nat) : 0 < (weight learned n).numerator := weight_positive learned n
example : Equal (weight [(1,1), (1,2)] 1) (weight [(1,3)] 1) := by decide_cbv
''')

    def test_completed_row_update_and_cancellation(self):
        self.prove('''
example : total (commit learned 0) = 7 := by decide_cbv
example : countAt 0 (commit learned 0) = 3 := by decide_cbv
example : Equal (weight (commit learned 0) 0) ⟨7,16,by decide⟩ := by decide_cbv
example (counts : Counts) (n : Nat) (q p : Fraction) (positive : 0 < p.numerator) :
    Equal (divide (mul (weight counts n) q) (mul (weight counts n) p)
      (Nat.mul_pos (weight_positive counts n) positive)) (divide q p positive) :=
  shared_ratio counts n q p positive
''')

    def test_wrong_laws_rejected(self):
        claims = {
            'omitted_empty': 'total learned = 4',
            'old_geometric': 'Equal (weight learned 1) (half 2)',
            'point_is_not_tail': 'Equal (weight learned 1) (remaining learned 1)',
            'strict_tail': 'Equal (remaining learned 4) ⟨1,112,by decide⟩',
            'bit_units': 'Equal (weight learned 1) (weight learned 8)',
            'finite_support': '(weight learned 100).numerator = 0',
            'postupdate_scoring': 'Equal (weight learned 0) (weight (commit learned 0) 0)',
            'missing_prior': 'Equal (weight learned 0) ⟨2,6,by decide⟩',
            'finite_cut_normalized': 'Equal (partialSum (weight learned) 5) one',
        }
        for name, claim in claims.items():
            with self.subTest(name=name):
                self.prove(f'example : {claim} := by decide_cbv', FALSE)


if __name__ == '__main__':
    unittest.main()
