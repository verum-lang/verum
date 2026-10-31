"""Exact ordered-sum/product ranks; no native enumeration or capped factors."""
import re
import unittest
from lean_test_fixture import IsolatedLeanTest

HEADER = '''import VerumExternalReplay.WeightedOrdinals
open WeightedOrdinals
set_option warningAsError true
def alternatives : List Alternative := [⟨0, []⟩, ⟨1, [2, 2]⟩]
def radices : List Nat := [radix 1 alternatives, radix 2 alternatives]
def count (n : Nat) := n * (radices.take n).prod
def zeros : Digits radices := .cons ⟨0, by decide⟩ (.cons ⟨0, by decide⟩ .nil)
def lastWord : Digits [5, 9] := .cons ⟨4, by decide⟩ (.cons ⟨8, by decide⟩ .nil)
def selectedBranch : Choice [1, 4] := .there (.here ⟨3, by decide⟩)
'''


class WeightedOrdinalsTests(IsolatedLeanTest):
    MODULES = ('WeightedOrdinals',)

    def test_constructive_axiom_roster(self):
        names = re.findall(r'^theorem ([\w.]+)', self.sources['WeightedOrdinals'], re.M)
        names += ['Choice.rank', 'Digits.rank', 'Alternative.width', 'prefixSum', 'addressRank']
        names = ['WeightedOrdinals.' + n for n in names]
        self.assertEqual(len(names), 19)
        self.assert_axioms(self.prove(HEADER + '\n'.join('#print axioms ' + n for n in names)), names)

    def test_original_v2_rank_includes_unselected_child_products(self):
        self.prove(HEADER + '''
example : radix 1 alternatives = 5 := by rfl
example : radix 2 alternatives = 9 := by rfl
example : selectedBranch.rank = 4 := by rfl
example : count 1 = 5 := by rfl
example : count 2 = 90 := by rfl
example : prefixSum count 2 = 5 := by rfl
example : addressRank (prefixSum count 2) zeros ⟨0, by decide⟩ = 5 := by rfl
example : addressRank 5 lastWord ⟨1, by decide⟩ = 94 := by rfl
example : prefixSum count 3 = 95 := by rfl
''')

    def test_zero_unit_many_and_nested_child_cardinalities(self):
        self.prove(HEADER + '''
example : Alternative.width 0 ⟨0, []⟩ = 1 := by rfl
example : Alternative.width 3 ⟨2, []⟩ = 9 := by rfl
example : Alternative.width 3 ⟨2, [1, 1]⟩ = 9 := by rfl
example : Alternative.width 3 ⟨2, [2, 0, 4]⟩ = 0 := by rfl
example : Alternative.width 3 ⟨2, [2, 3, 4]⟩ = 216 := by rfl
example : Alternative.width 3 ⟨2, [2, 3 * 4]⟩ = 216 := by rfl
example : ((Choice.here ⟨0, by decide⟩ : Choice [1, 4]).skip [0, 0]).rank = 0 := by rfl
example : ¬ Nonempty (Choice [0, 0]) := by
  rintro ⟨choice⟩
  have bound := choice.rank_bound
  simp at bound
example : ¬ Nonempty (Digits [2, 0, 3]) := by
  rintro ⟨word⟩
  have bound := word.rank_bound
  simp at bound
''')

    def test_mixed_radix_append_and_exact_machine_boundary(self):
        self.prove(HEADER + '''
def initial : Digits [2, 3] := .cons ⟨1, by decide⟩ (.cons ⟨2, by decide⟩ .nil)
example : initial.rank = 5 := by rfl
example : (initial.append (.cons (radix := 4) ⟨3, by decide⟩ .nil)).rank = 23 := by rfl
example (rs : List Nat) (word : Digits rs) (d : Fin 7) :
    (word.append (.cons d .nil)).rank = word.rank * 7 + d.val := word.rank_step d
example : (Digits.cons (radix := 18446744073709551615)
    ⟨18446744073709551614, by omega⟩ .nil).rank = 18446744073709551614 := by rfl
example (sizes : List Nat) (n : Nat) (inside : n < sizes.sum) :
    ∃ choice : Choice sizes, choice.rank = n := choice_complete sizes n inside
example (rs : List Nat) (n : Nat) (inside : n < rs.prod) :
    ∃ word : Digits rs, word.rank = n := digits_complete rs n inside
''')

    def test_injective_ranks_and_disjoint_length_blocks(self):
        self.prove(HEADER + '''
example (rs : List Nat) (left right : Digits rs) (same : left.rank = right.rank) :
    left = right := left.rank_injective right same
example (rs : List Nat) (left right : Digits rs) (a b : Fin rs.length)
    (same : addressRank 17 left a = addressRank 17 right b) :
    left = right ∧ a = b := address_injective 17 left right a b same
example (c : Nat → Nat) (a b x y : Nat) (order : a < b) (inside : x < c a) :
    prefixSum c a + x < prefixSum c b + y := length_blocks_disjoint c a b x y order inside
example : 5 ≤ addressRank 5 lastWord ⟨1, by decide⟩ ∧
    addressRank 5 lastWord ⟨1, by decide⟩ < 5 + [5,9].prod * [5,9].length :=
  address_interval 5 lastWord ⟨1, by decide⟩
''')

    def test_false_flat_and_weighted_rank_claims_rejected(self):
        claims = {
            'v1_cardinality_in_v2': 'radix 1 alternatives = 2',
            'v1_zero_zero_rank': 'addressRank (prefixSum count 2) zeros ⟨0, by decide⟩ = 2',
            'child_sizes_added': 'Alternative.width 1 ⟨0, [2, 3]⟩ = 5',
            'zero_child_ignored': 'Alternative.width 1 ⟨0, [2, 0]⟩ = 2',
            'empty_product_is_zero': 'Alternative.width 1 ⟨0, []⟩ = 0',
            'output_choices_omitted': 'count 2 = 45',
            'suffix_radix_omitted': 'lastWord.rank = 12',
            'shorter_prefix_omitted': 'addressRank 5 lastWord ⟨1, by decide⟩ = 89',
            'selected_alternative_counts_only': 'selectedBranch.rank = 3',
            'last_address_outside_interval': 'addressRank 5 lastWord ⟨1, by decide⟩ = 95',
            'capped_child_is_exact': 'Alternative.width 1 ⟨0, [min 2 4, 3]⟩ = Alternative.width 1 ⟨0, [4, 3]⟩',
        }
        for name, claim in claims.items():
            with self.subTest(claim=name):
                self.prove(HEADER + 'example : ' + claim + ' := by decide_cbv', failure=True)


if __name__ == '__main__':
    unittest.main()
