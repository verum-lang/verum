"""Checked telemetry cannot turn incomplete coverage or overflow into zero."""
import re
import unittest
from lean_test_fixture import IsolatedLeanTest

HEADER = '''import VerumExternalReplay.WorkObservation
open WorkObservation
set_option warningAsError true
def maximum : Nat := 2^128 - 1
def zeroes : Nat → Option Nat := fun _ => some 0
def failedAction (_ : Nat) : Nat × Except String Unit := (4, .error "returned refusal")
'''


class WorkObservationTests(IsolatedLeanTest):
    MODULES = ('WorkObservation',)

    def test_constructive_conservation_and_noninterference(self):
        names = ['WorkObservation.' + name for name in
                 re.findall(r'^theorem (\w+)', self.sources['WorkObservation'], re.M)]
        self.assertEqual(len(names), 12)
        self.assert_axioms(self.prove(HEADER + '\n'.join('#print axioms ' + name for name in names)), names)

    def test_checked_boundaries_phase_isolation_and_returned_errors(self):
        self.prove(HEADER + '''
example : measure maximum 10 [8, 5, 4] = some 6 := by decide_cbv
example : checked maximum maximum = some maximum := by decide_cbv
example : checked maximum (maximum + 1) = none := by decide_cbv
example : credit maximum zeroes 2 (delta 4 5) 2 = none := by decide_cbv
example : credit maximum zeroes 2 (delta 4 5) 3 = some 0 := by decide_cbv
example : add maximum (some maximum) (some 1) = none := by decide_cbv
example : add maximum (add maximum (some maximum) (some 1)) (some 0) = none := by decide_cbv
example : instrument id failedAction 10 = ((4, .error "returned refusal"), some 6) := by rfl
''')

    def test_false_missing_coverage_and_overflow_claims_fail(self):
        claims = {
            'missing_final_interval_counts_as_complete': 'measure maximum 10 [8, 5] = some 6',
            'omitted_owner_is_global_zero': 'add maximum (some 0) (some 7) = some 0',
            'unknown_is_zero': 'add maximum (some 4) none = some 4',
            'overflow_is_zero': 'add maximum (some maximum) (some 1) = some 0',
            'budget_increase_is_free_work': 'delta 4 5 = some 0',
            'entry_only_phase_has_correct_attribution': 'credit maximum zeroes 0 (delta 10 4) 0 = some 2',
            'returned_error_erases_work': '(instrument id failedAction 10).2 = some 0',
        }
        for name, claim in claims.items():
            with self.subTest(claim=name):
                self.prove(HEADER + 'example : ' + claim + ' := by decide_cbv', failure=True)


if __name__ == '__main__':
    unittest.main()
