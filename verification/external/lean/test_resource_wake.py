"""Consumed wait gates, distinct from monotone eligibility or native progress."""
import re
import unittest
from lean_test_fixture import IsolatedLeanTest

HEADER = '''import VerumExternalReplay.ResourceWake
open ResourceLedger ResourceWake
set_option warningAsError true
def available : Ledger := ⟨1, 2^64 - 1, some 8, 0, false, []⟩
def allocated : Ledger := admitted available 0 8
def wait : Wait Nat := ⟨1, .capacity, 8, 37⟩
def gate : Gate (Nat × Nat) Nat := ⟨(2, 1), some wait⟩
'''


class ResourceWakeTests(IsolatedLeanTest):
    MODULES = ('ResourceWake',)

    def test_universal_consumption_and_boundary_laws(self):
        names = ['ResourceWake.' + name for name in
                 re.findall(r'^theorem (\w+)', self.sources['ResourceWake'], re.M)]
        self.assertEqual(len(names), 10)
        self.assert_axioms(self.prove(HEADER + '\n'.join('#print axioms ' + name for name in names)), names)

    def test_resumed_allocation_cannot_reactivate_a_consumed_gate(self):
        self.prove(HEADER + '''
example : ready available wait = true := by decide_cbv
example : ready allocated wait = false := by decide_cbv
example : gate.waiting allocated (2, 1) = true := by decide_cbv
example : blocked allocated (2, 1) (consume available (2, 1) (some gate)) = false := by decide_cbv
example : (Gate.mk (2, 1) (none : Option (Wait Nat))).waiting available (2, 1) = true := by decide_cbv
example : gate.waiting allocated (3, 1) = false := by decide_cbv
example : (Gate.mk (2, 1) (some { wait with account := 2 })).waiting available (2, 1) = true := by decide_cbv
example : gate.waiting { available with poisoned := true } (2, 1) = true := by decide_cbv
''')

    def test_false_monotonicity_and_unearned_wake_claims_fail(self):
        claims = {
            'retained_gate_cannot_reblock': 'gate.waiting allocated (2, 1) = false',
            'eligibility_reserves_capacity': 'ready allocated wait = true',
            'consumption_can_restore_old_gate': 'blocked allocated (2, 1) (consume available (2, 1) (some gate)) = true',
            'unrelated_memory_wakes_evidence_only': '(Gate.mk (2, 1) (none : Option (Wait Nat))).waiting available (2, 1) = false',
            'consumption_discards_a_blocked_gate': 'blocked allocated (2, 1) (consume allocated (2, 1) (some gate)) = false',
        }
        for name, claim in claims.items():
            with self.subTest(claim=name):
                self.prove(HEADER + 'example : ' + claim + ' := by decide_cbv', failure=True)


if __name__ == '__main__':
    unittest.main()
