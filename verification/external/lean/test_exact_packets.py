"""Exact factor multiplicities and cancellation, independent of bank keys."""
import re
import unittest
from lean_test_fixture import IsolatedLeanTest

HEADER = '''import VerumExternalReplay.ExactPackets
import VerumExternalReplay.CountBank
open ExactMass ExactPackets
set_option warningAsError true
def half : Fraction := ⟨1,2,by decide⟩
def third : Fraction := ⟨1,3,by decide⟩
'''


class ExactPacketsTests(IsolatedLeanTest):
    MODULES = ('ExactPackets', 'CountBank')

    def test_axioms(self):
        names = ['ExactPackets.'+n for n in re.findall(r'^theorem (\w+)', self.sources['ExactPackets'], re.M)]
        self.assertEqual(len(names), 11)
        self.assert_axioms(self.prove(HEADER+'\n'.join('#print axioms '+n for n in names)), names)

    def test_multiplicity_and_positive_cancellation(self):
        self.prove(HEADER+'''
example : aggregate [(half,3),(third,2)] = ⟨1,72,by decide⟩ := by decide_cbv
example : aggregate [(half,1),(half,2),(third,2)] = aggregate [(half,3),(third,2)] := by decide_cbv
example : aggregate [] = one := by decide_cbv
example : Equal (quotient (power half 7) (power half 4) (by decide)) (power half 3) := by decide_cbv
example : Equal (quotient (power half 4) (power half 7) (by decide)) ⟨8,1,by decide⟩ := by decide_cbv
example (q : Fraction) (h : 0 < q.numerator) (m n : Nat) :
    Equal (quotient (power q m) (power q n) (power_positive q h _))
      (quotient (power q (m-min m n)) (power q (n-min m n)) (power_positive q h _)) :=
  cancel_minimum q h m n
''')

    def test_equal_probability_does_not_merge_position_keys(self):
        self.prove(HEADER+'''
example : ([0,1] : List Nat).map (fun _ => half) = [half,half] := rfl
example : CountBank.ingest id CountBank.empty ([0,1] : List Nat) 1 = 1 := by decide_cbv
example : CountBank.ingest id CountBank.empty ([0,0] : List Nat) 1 = 0 := by decide_cbv
''')

    def test_missing_factor_changed_multiplicity_or_zero_cancellation_is_refused(self):
        for claim in [
            'aggregate [(half,3),(third,1)] = aggregate [(half,3),(third,2)]',
            'aggregate [(half,2),(third,2)] = aggregate [(half,3),(third,2)]',
            'Equal (power half 7) (power half 3)',
            'CountBank.ingest id CountBank.empty ([0,1] : List Nat) 1 = 0',
            '0 < (zero : Fraction).numerator',
        ]:
            with self.subTest(claim=claim):
                self.prove(HEADER+'example : '+claim+' := by decide_cbv', failure=True)


if __name__ == '__main__':
    unittest.main()
