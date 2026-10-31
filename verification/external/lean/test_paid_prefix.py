"""Exact frozen identity coverage; paid visit counts alone are insufficient."""
import re
import unittest
from lean_test_fixture import IsolatedLeanTest

HEADER = '''import VerumExternalReplay.PaidPrefix
open PaidPrefix
set_option warningAsError true
def oldRound : List Event := [⟨1, 1⟩, ⟨2, 1⟩, ⟨0, 1⟩, ⟨1, 1⟩, ⟨2, 1⟩]
'''


class PaidPrefixTests(IsolatedLeanTest):
    MODULES = ('PaidPrefix',)

    def test_constructive_coverage_and_machine_word_bounds(self):
        names = ['PaidPrefix.' + name for name in
                 re.findall(r'^theorem (\w+)', self.sources['PaidPrefix'], re.M)]
        self.assertEqual(len(names), 14)
        self.assert_axioms(self.prove(HEADER + '\n'.join('#print axioms ' + name for name in names)), names)

    def test_identity_order_appends_refusals_and_full_width(self):
        self.prove(HEADER + '''
example : ((issued 5).run oldRound).next = 3 := by decide_cbv
example : ((issued 5).run (oldRound ++ [⟨3, 1⟩, ⟨4, 1⟩])).ready = true := by decide_cbv
example : ((issued 2).run [⟨0, 1⟩, ⟨2, 50⟩, ⟨1, 0⟩, ⟨1, 1⟩]).ready = true := by decide_cbv
example : ((issued 0).run [⟨0, 5⟩]).ready = true := by decide_cbv
example : (State.mk (2^64 - 2) (2^64 - 1)).visit ⟨2^64 - 2, 1⟩ = ⟨2^64 - 1, 2^64 - 1⟩ := by decide_cbv
example : (State.mk (2^64 - 1) (2^64 - 1)).visit ⟨2^64 - 1, 1⟩ = ⟨2^64 - 1, 2^64 - 1⟩ := by decide_cbv
example : (State.mk (2^32 - 2) (2^32 - 1)).visit ⟨2^32 - 2, 1⟩ = ⟨2^32 - 1, 2^32 - 1⟩ := by decide_cbv
''')

    def test_false_coverage_claims_fail_semantically(self):
        claims = {
            'arbitrary_five_visits_cover_five_members': '((issued 5).run oldRound).ready = true',
            'duplicate_identity_pays_twice': '((issued 2).run [⟨0, 1⟩, ⟨0, 1⟩]).ready = true',
            'zero_paid_dispatch_advances': '((issued 1).visit ⟨0, 0⟩).ready = true',
            'out_of_order_dispatch_covers_prefix': '((issued 2).run [⟨1, 1⟩, ⟨0, 1⟩]).ready = true',
            'appended_identity_discharges_old_member': '((issued 1).visit ⟨1, 100⟩).ready = true',
            'completed_prefix_wraps': '(State.mk (2^64 - 1) (2^64 - 1)).visit ⟨2^64 - 1, 1⟩ = ⟨0, 2^64 - 1⟩',
        }
        for name, claim in claims.items():
            with self.subTest(claim=name):
                self.prove(HEADER + 'example : ' + claim + ' := by decide_cbv', failure=True)


if __name__ == '__main__':
    unittest.main()
