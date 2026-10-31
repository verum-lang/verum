"""Exact admission-round accounting; no inferred native FIFO correspondence."""
import re
import unittest
from lean_test_fixture import IsolatedLeanTest

HEADER = '''import VerumExternalReplay.AdmissionRound
open AdmissionRound
set_option warningAsError true
'''


class AdmissionRoundTests(IsolatedLeanTest):
    MODULES = ('AdmissionRound',)

    def test_universal_laws_and_axioms(self):
        names = ['AdmissionRound.' + name for name in
                 re.findall(r'^theorem (\w+)', self.sources['AdmissionRound'], re.M)]
        self.assertEqual(len(names), 14)
        rows = self.prove(HEADER + '\n'.join('#print axioms ' + name for name in names))
        self.assert_axioms(rows, names)

    def test_full_width_state_and_exact_visit_count(self):
        self.prove(HEADER + '''
example : (issued (2^64 - 1) true).remaining (2^64 - 1) = none := by decide_cbv
example : (issued (2^64 - 1) true).visited.remaining (2^64 - 1) = some (2^64 - 1) := by decide_cbv
example : (issued (2^32 - 1) true).remaining (2^32 - 1) = none := by decide_cbv
example : (issued 3 true).visits 4 = issued 0 false := by decide_cbv
example : (issued 3 true).visits 7 = issued 0 false := by decide_cbv
example : (issued 3 true).ready true = true := by decide_cbv
example : (issued 3 true).ready false = false := by decide_cbv
example (queued : Nat) (active : Bool) (count : Nat) :
    ((issued queued active).visits count).ready false = true ↔
      queued + (if active then 1 else 0) ≤ count := ready_after_visits_iff _ _
''')

    def test_false_overflow_readiness_and_visit_claims_fail(self):
        claims = {
            'wrapped_diagnostic': '(issued (2^64 - 1) true).remaining (2^64 - 1) = some 0',
            'active_visit_also_debits_queue': '(issued 3 true).visited = issued 2 false',
            'too_few_visits': '((issued 3 true).visits 3).ready false = true',
            'issuance_is_service': '(issued 3 true).ready false = true',
            'empty_bypass_proves_exhaustion': '(issued 3 true).debt = 0',
            'unchecked_oversize_state': '(issued 9 false).remaining 8 = some 9',
        }
        for name, claim in claims.items():
            with self.subTest(claim=name):
                self.prove(HEADER + 'example : ' + claim + ' := by decide_cbv', failure=True)


if __name__ == '__main__':
    unittest.main()
