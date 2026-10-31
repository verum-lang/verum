"""Constructive arbitrary-arity partition, tied maxima and false-claim controls."""
import re
import unittest
from lean_test_fixture import IsolatedLeanTest

HEADER = '''import VerumExternalReplay.ArrivalSlabs
open ArrivalSlabs
set_option warningAsError true
def triple (a b c : Nat) : Fin 3 → Nat :=
  fun index => match index.val with | 0 => a | 1 => b | _ => c
'''


class ArrivalSlabTests(IsolatedLeanTest):
    MODULES = ('ArrivalSlabs',)

    def test_arbitrary_arity_laws_have_constructive_axioms(self):
        names = ['ArrivalSlabs.' + name for name in
                 re.findall(r'^theorem (\w+)', self.sources['ArrivalSlabs'], re.M)]
        self.assertEqual(len(names), 7)
        self.assert_axioms(self.prove(HEADER + '\n'.join('#print axioms ' + n for n in names)), names)

    def test_first_tie_singleton_and_word_limit(self):
        self.prove(HEADER + '''
example : firstMaximum (fun _ : Fin 1 => 11) = (11, 0) := by rfl
example : firstMaximum (triple 0 0 0) = (0, 0) := by rfl
example : firstMaximum (triple 8 8 3) = (8, 0) := by rfl
example : firstMaximum (triple 2 8 8) = (8, 1) := by rfl
example : firstMaximum (triple 1 3 8) = (8, 2) := by rfl
example : firstMaximum (triple 0 18446744073709551615 18446744073709551615) =
    (18446744073709551615, 1) := by rfl
example : Slab (triple 2 8 8) 8 1 := by decide_cbv
example : ¬ Slab (triple 2 8 8) 8 2 := by decide_cbv
example : ¬ ∃ anchor : Fin 0, Slab (fun i => Fin.elim0 i) 0 anchor := by
  intro ⟨anchor, _⟩; exact Fin.elim0 anchor
''')

    def test_filter_preserves_partition_without_inventing_admission(self):
        self.prove(HEADER + '''
example (n : Nat) (row : Fin (n + 1) → Nat) (sizes : Nat → Nat) (limit : Nat)
    (key : Nat × Fin (n + 1)) :
    (1 + (List.ofFn (fun index => sizes (row index))).sum ≤ limit ∧ Slab row key.1 key.2) ↔
      (1 + (List.ofFn (fun index => sizes (row index))).sum ≤ limit ∧ key = firstMaximum row) :=
  filtered_membership (fun row => 1 + (List.ofFn (fun index => sizes (row index))).sum ≤ limit) row key
''')

    def test_false_partition_claims_rejected_semantically(self):
        claims = {
            'last_tie': 'firstMaximum (triple 2 8 8) = (8, 2)',
            'nonmaximum': 'Slab (triple 2 8 3) 3 2',
            'left_equal_permitted': 'Slab (triple 8 8 3) 8 1',
            'right_equal_forbidden': '¬ Slab (triple 2 8 8) 8 1',
            'all_zero_has_no_anchor': '¬ Slab (triple 0 0 0) 0 0',
            'word_max_wraps': '(firstMaximum (triple 0 18446744073709551615 3)).1 = 0',
        }
        for name, claim in claims.items():
            with self.subTest(claim=name):
                self.prove(HEADER + 'example : ' + claim + ' := by decide_cbv', failure=True)


if __name__ == '__main__':
    unittest.main()
