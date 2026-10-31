"""FIFO liveness under finite completion witnesses; no grant-free time bound."""
import re
import unittest
from lean_test_fixture import IsolatedLeanTest

HEADER = '''import VerumExternalReplay.ResumableFifo
open ResumableFifo
set_option warningAsError true
def plan (turn : Nat) : Quantum Nat :=
  ⟨List.replicate (turn + 2) [turn + 10], ⟨true, [turn + 20]⟩⟩
def held : List (Step Nat) := [.hold [3], .hold [], .hold [4], .boundary ⟨true, [5]⟩]
'''


class ResumableFifoTests(IsolatedLeanTest):
    MODULES = ('ResumableFifo',)

    def test_shared_and_resumable_laws_have_constructive_axioms(self):
        names = [namespace + name for module, namespace in
                 [('FifoService', 'AcquisitionScheduling.'), ('ResumableFifo', 'ResumableFifo.')]
                 for name in re.findall(r'^theorem (\w+)', self.sources[module], re.M)]
        self.assertEqual(len(names), 16)
        self.assert_axioms(self.prove(HEADER + '\n'.join('#print axioms ' + n for n in names)), names)

    def test_held_arrivals_refusals_and_requeue_order(self):
        self.prove(HEADER + '''
example : run [1, 2] held = [2, 3, 4, 1, 5] := by decide_cbv
example : serviced [1, 2] held = [some 1] := by decide_cbv
example : advance [1, 2] (.hold []) = [1, 2] := by rfl
example : run [1, 2] [.hold [3], .boundary ⟨false, [4]⟩] = [2, 3, 4] := by decide_cbv
example : serviced [1, 2, 7] (through plan 3) = [some 1, some 2, some 7] := by decide_cbv
example : completed (through plan 3) = 3 := by decide_cbv
example : AcquisitionScheduling.selected [1, 2, 7] (fun _ => ⟨true, [8, 9]⟩) 3 =
    [some 1, some 2, some 7] := by decide_cbv
''')

    def test_arbitrary_pauses_do_not_imply_service(self):
        self.prove(HEADER + '''
example (queue : List Nat) (calls : Nat) :
    serviced queue (List.replicate calls (.hold [])) = [] := pauses_do_not_service queue calls
example (target : Nat) (ahead suffix : List Nat) (schedule : Nat → Quantum Nat) :
    ∃ count, some target ∈ serviced (ahead ++ target :: suffix) (through schedule count) :=
  eventual_service target ahead suffix schedule
''')

    def test_false_rotation_and_latency_claims_rejected(self):
        claims = {
            'microstep_rotates': 'advance [1, 2] (.hold [3]) = [2, 3, 1]',
            'refusal_discards': 'advance [1, 2] (.hold []) = [2]',
            'arrivals_after_requeue': 'run [1, 2] [.hold [3], .boundary ⟨true, []⟩] = [2, 1, 3]',
            'poll_is_service': 'serviced [1] [.hold []] = [some 1]',
            'calls_bound_latency': 'some 2 ∈ serviced [1, 2] [.hold [], .hold [], .boundary ⟨true, []⟩]',
            'double_count': 'completed ([.boundary ⟨true, []⟩] : List (Step Nat)) = 2',
            'unfinished_discarded': 'advance [1, 2] (.boundary ⟨true, [3]⟩) = [2, 3]',
            'unbounded_arrivals_fit_fixed_storage': '(advance [1, 2] (.hold [3, 4])).length ≤ 2',
            'empty_plan_is_fair': 'some 1 ∈ serviced [1] (through plan 0)',
        }
        for name, claim in claims.items():
            with self.subTest(claim=name):
                self.prove(HEADER + 'example : ' + claim + ' := by decide_cbv', failure=True)


if __name__ == '__main__':
    unittest.main()
