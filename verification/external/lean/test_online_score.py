"""Exact retained score updates against the existing full histogram scorer."""
import re
import unittest
from lean_test_fixture import IsolatedLeanTest

HEADER = '''import VerumExternalReplay.OnlineScore
open CategoricalScore OnlineScore
set_option warningAsError true
structure Bank where
  parent : List Nat
  children : List (List Nat)
  samples : Nat
  deriving DecidableEq
def start : Bank := ⟨[0,0,0], [[0,0,0],[0,0,0]], 0⟩
def full (bank : Bank) : Score := fromCounts 3 bank.parent bank.children bank.samples
def zeroScore : Score := ⟨one,one,0⟩
def ingest (bank : Bank) (row : Row) : Except Nat Bank :=
  if row.label ≥ 3 then .error 1 else
  if row.reading ≥ bank.children.length then .error 2 else
  .ok ⟨bank.parent.modify row.label (·+1),
       bank.children.modify row.reading (fun xs => xs.modify row.label (·+1)), bank.samples+1⟩
def factor (bank : Bank) (row : Row) : Score :=
  let child := bank.children[row.reading]?.getD []
  scoreStep 3 bank.parent.sum (bank.parent[row.label]?.getD 0)
    child.sum (child[row.label]?.getD 0)
def rows : List Row := [⟨0,0⟩,⟨1,1⟩,⟨0,0⟩,⟨0,1⟩,⟨1,2⟩,⟨1,1⟩]
def runRows (word : List Row) :=
  SharedUpdates.runFrom (track ingest factor) 0 word (start,zeroScore)
def matchesOracle (word : List Row) : Bool :=
  match runRows word with
  | .complete (bank,s) =>
      let oracle := contextScore ⟨3,word⟩
      equivalent s.compression oracle.compression && equivalent s.predictive oracle.predictive &&
      s.samples == oracle.samples && bank.samples == word.length
  | .refused .. => false
'''


class OnlineScoreTests(IsolatedLeanTest):
    MODULES = ('OnlineScore',)

    def test_constructive_axioms(self):
        names = ['OnlineScore.' + name for name in
                 re.findall(r'^theorem (\w+)', self.sources['OnlineScore'], re.M)]
        self.assertEqual(len(names), 32)
        self.assert_axioms(self.prove(HEADER + '\n'.join('#print axioms ' + n for n in names)), names)

    def test_empty_and_unseen_counts(self):
        self.prove(HEADER + '''
example : ktStep 2 0 0 = ⟨1,2⟩ := by rfl
example : mlStep 0 0 = ⟨1,1⟩ := by rfl
example : equivalent (kt 2 [0,0]) one = true := by decide_cbv
example : equivalent (parentML [0,0]) one = true := by decide_cbv
example : equivalent (parentML [2,1]) (mul (parentML [2,0]) (mlStep 2 0)) = true := by decide_cbv
example : equivalent (kt 3 [2,1,0]) (mul (kt 3 [2,0,0]) (ktStep 3 2 0)) = true := by decide_cbv
example : equivalent (mlStep 2 0) ⟨4,27⟩ = true := by decide_cbv
example : equivalent (mlStep 3 2) ⟨729,1024⟩ = true := by decide_cbv
''')

    def test_factored_positive_powers_preserve_the_same_mle(self):
        self.prove(HEADER + '''
example (total selected : Nat) : factoredML total selected = mlStep total selected :=
  factored_ml_exact total selected
example (total selected : Nat) : equivalent (factoredML total selected) (mlStep total selected) = true :=
  factored_ml_checked total selected
example (n : Nat) : Positive (symbolPower n) ∧ Positive (totalPower n) :=
  ⟨symbolPower_positive n, totalPower_positive n⟩
example : symbolPower 0 = one ∧ totalPower 0 = one ∧ factoredML 0 0 = one := ⟨rfl,rfl,rfl⟩
example : equivalent (factoredML 3 2) ⟨729,1024⟩ = true := by decide_cbv
example : equivalent (factoredML 2 0) ⟨4,27⟩ = true := by decide_cbv
example : equivalent (factoredML 5 5) one = true := by decide_cbv
''')

    def test_universal_full_bank_recurrence(self):
        self.prove(HEADER + '''
example (K : Nat) (hK : 0 < K) (parent : List Nat) (children : List (List Nat))
    (child label samples : Nat) (hc : child < children.length) (hp : label < parent.length)
    (hl : label < children[child].length) :
    Agrees (fromCounts K (parent.modify label (·+1))
      (children.modify child (fun xs => xs.modify label (·+1))) (samples+1))
      (accumulate (fromCounts K parent children samples)
        (scoreStep K parent.sum parent[label] children[child].sum children[child][label])) :=
  fromCounts_step K hK parent children child label samples hc hp hl
example (K n : Nat) (hK : 0 < K) :
    equivalent (kt K [n+1]) (mul (kt K [n]) (ktStep K n n)) = true := by
  apply equivalent_of_equal (kt_positive K hK _)
    (positive_mul (kt_positive K hK _) (ktStep_positive K n n hK))
  have h : kt K [n+1] = mul (kt K [n]) (ktStep K n n) := by
    simpa using kt_step K [n] 0 (by simp)
  rw [h]
  rfl
example (K : Nat) (parent : List Nat) (children : List (List Nat)) (samples : Nat) :
    fromCounts K parent (children ++ [List.replicate K 0]) samples =
      fromCounts K parent children samples := new_child_neutral K parent children samples
''')

    def test_retained_rows_match_independent_full_recomputation(self):
        self.prove(HEADER + '''
example : matchesOracle [] = true := by decide_cbv
example : matchesOracle [⟨0,0⟩] = true := by decide_cbv
example : matchesOracle [⟨0,0⟩,⟨0,0⟩,⟨0,0⟩] = true := by decide_cbv
example : matchesOracle rows = true := by decide_cbv
example : matchesOracle rows.reverse = true := by decide_cbv
example : matchesOracle (rows ++ [⟨0,2⟩,⟨1,0⟩]) = true := by decide_cbv
example : SharedUpdates.runFrom (track ingest factor) 0 (rows.take 3 ++ rows.drop 3) (start,zeroScore) =
    SharedUpdates.continueFrom (track ingest factor) (rows.take 3).length (rows.drop 3)
      (runRows (rows.take 3)) := SharedUpdates.append _ _ _ _ _
example : equivalent ((scoreStep 3 3 2 0 0).predictive) ⟨1024,2187⟩ = true := by decide_cbv
example : equivalent ((scoreStep 3 0 0 0 0).compression) one = true ∧
    equivalent ((scoreStep 3 0 0 0 0).predictive) ⟨1,3⟩ = true := by decide_cbv
''')

    def test_refusals_are_sticky_and_not_numeric_scores(self):
        self.prove(HEADER + '''
example : runRows [⟨0,3⟩,⟨0,0⟩] =
    .refused 0 (start,zeroScore) ⟨0,3⟩ 1 := by rfl
example : runRows [⟨9,0⟩,⟨0,0⟩] =
    .refused 0 (start,zeroScore) ⟨9,0⟩ 2 := by rfl
example (before suffix : List Row) (state : Bank × Score) (event : Row) (reason position : Nat)
    (failed : runRows before = .refused position state event reason) :
    runRows (before ++ suffix) = .refused position state event reason :=
  SharedUpdates.refusal_retained _ _ _ _ _ _ _ _ failed
example : score [⟨0,[]⟩] = none := by rfl
example : score [⟨3,[⟨0,3⟩]⟩] = none := by rfl
example : ktStep 3 18446744073709551615 18446744073709551615 =
    ⟨36893488147419103231,36893488147419103233⟩ := by rfl
''')

    def test_false_recurrences_rejected_semantically(self):
        claims = {
            'postincrement_kt': 'equivalent (kt 3 [3,1,0]) (mul (kt 3 [2,1,0]) (ktStep 3 4 3)) = true',
            'postincrement_mle': 'equivalent (parentML [3,1,0]) (mul (parentML [2,1,0]) (mlStep 4 3)) = true',
            'unseen_symbol_omitted': 'equivalent (parentML [2,1,0]) (parentML [2,0,0]) = true',
            'empty_mle_zero': 'equivalent (parentML [0,0,0]) ⟨0,1⟩ = true',
            'empty_first_kt_one': 'equivalent (ktStep 3 0 0) one = true',
            'drop_unseen_alphabet': 'equivalent (kt 3 [2,1,0]) (kt 2 [2,1]) = true',
            'child_mle_numerator': 'equivalent (scoreStep 3 3 2 0 0).predictive (div (mlStep 0 0) (mlStep 3 2)) = true',
            'parent_kt_in_predictive': 'equivalent (scoreStep 3 3 2 0 0).predictive (scoreStep 3 3 2 0 0).compression = true',
            'independent_batches_multiply': 'equivalent (kt 3 [2,0,0]) (mul (kt 3 [1,0,0]) (kt 3 [1,0,0])) = true',
            'swapped_parent_child': 'equivalent (scoreStep 3 3 2 0 0).compression (scoreStep 3 0 0 3 2).compression = true',
            'bad_row_erased': 'matchesOracle [⟨0,3⟩,⟨0,0⟩] = true',
            'zero_alphabet_positive': '(ktStep 0 0 0).denominator > 0',
            'unchecked_u64_doubling': '(ktStep 3 18446744073709551615 1).denominator < 18446744073709551616',
            'zero_symbol_base_is_positive': 'Positive (⟨1,0⟩ : Ratio)',
            'zero_total_base_is_positive': 'Positive (⟨0,1⟩ : Ratio)',
            'skip_nonzero_symbol_power': 'equivalent (factoredML 3 2) (mul ⟨3,4⟩ (totalPower 3)) = true',
            'skip_nonzero_total_power': 'equivalent (factoredML 3 2) (mul ⟨3,4⟩ (symbolPower 2)) = true',
            'reverse_scalar_factor': 'equivalent (factoredML 3 2) (mul ⟨4,3⟩ (mul (symbolPower 2) (totalPower 3))) = true',
        }
        for name, claim in claims.items():
            with self.subTest(claim=name):
                tactic = 'unfold Positive; decide_cbv' if claim.startswith('Positive ') else 'decide_cbv'
                self.prove(HEADER + 'example : ' + claim + ' := by ' + tactic, failure=True)


if __name__ == '__main__':
    unittest.main()
