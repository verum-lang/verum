"""Full finite KT capacity theorem and strict counterexamples to weakened claims."""
from pathlib import Path
import tempfile
import unittest
from check_typed_programs import check, require_success, run

HEADER='import VerumExternalReplay.CapacityRecurrence\nopen RefinementFeasibility RefinementCapacity\n'
REDUCE='''  simp only [partitionScore, histogramRatio, capacity, parentPower,
  BalancedProduct.bound, BalancedProduct.ratio, BalancedProduct.product,
  factorProduct, cell, RefinementFeasibility.pure, histogram, mergeCounts,
  weight, rising, List.sum_cons, List.sum_nil, List.replicate_succ,
  List.replicate_zero, Rat.pow_succ, Rat.pow_zero, Nat.div_eq_of_lt (by decide : 1 < 8),
  Nat.div_eq_of_lt (by decide : 2 < 8)]
  grind'''

class RefinementCapacityTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake','build','VerumExternalReplay.CapacityRecurrence'))

    def test_axiom_roster(self):
        names=('histogram_bound capacity_formula capacity_zero capacity_positive quotient_weights '
               'partition_to_histogram partition_capacity pure_partition_score capacity_attained '
               'partitionScore_eq_likelihood_ratio capacity_single_outcome contextual_capacity capacity_sharp balanced_bound_succ weight_ratio_eq_step capacity_succ_step capacity_recurrence capacity_first').split()
        with tempfile.TemporaryDirectory() as d:
            p=Path(d)/'axioms.lean';p.write_text(HEADER+'\n'.join('#print axioms RefinementCapacity.'+n for n in names))
            rows=[r['data'] for r in check(p)['diagnostics'] if r['severity']=='information']
        self.assertEqual(len(rows),len(names))
        for row in rows:
            if row.endswith('does not depend on any axioms'):continue
            self.assertTrue(set(row.split('depends on axioms: [')[1].rstrip(']').split(', '))<=
                            {'propext','Classical.choice','Quot.sound'},row)

    def test_universal_upper_bound_sharpness_and_empty_case(self):
        source=HEADER+'''
example (K : Nat) (hK : 0 < K) (rows : List (List Nat))
    (width : ∀ row ∈ rows, row.length = K) :
    partitionScore K rows ≤ capacity K (rows.map List.sum).sum := partition_capacity hK rows width
example (K n : Nat) (hK : 0 < K) :
    ∃ rows : List (List Nat), (∀ row ∈ rows, row.length = K) ∧
      (rows.map List.sum).sum = n ∧ partitionScore K rows = capacity K n := capacity_sharp hK n
example (K : Nat) : capacity K 0 = 1 := capacity_zero K
example (n : Nat) : capacity 1 n = 1 := capacity_single_outcome n
example : capacity 8 2 < capacity 8 1 := by
'''+REDUCE+'''
example : partitionScore 2 [[1,0],[0,1]] = 1 := by
'''+REDUCE
        with tempfile.TemporaryDirectory() as d:
            p=Path(d)/'positive.lean';p.write_text(source);check(p)

    def test_wrong_dimension_monotonicity_and_partition_bound_rejected(self):
        claims=['partitionScore 1 [[1,1]] ≤ capacity 1 2',
                'capacity 8 1 ≤ capacity 8 2',
                'partitionScore 2 [[1,0],[0,1]] < capacity 2 2']
        with tempfile.TemporaryDirectory() as d:
            for n,claim in enumerate(claims):
                p=Path(d)/f'negative{n}.lean';p.write_text(HEADER+f'example : {claim} := by\n'+REDUCE)
                check(p,'`grind` failed')

    def test_recurrence_rollover_and_wrong_quotient(self):
        with tempfile.TemporaryDirectory() as d:
            p=Path(d)/'recurrence.lean';p.write_text('import VerumExternalReplay.CapacityRecurrence\nopen RefinementCapacity RefinementFeasibility\nset_option warningAsError true\n\nexample : capacity 3 1 = 1/3 := capacity_first (by decide)\nexample : capacity 3 2 = capacity 3 1*(4/3) := by\n  have h := capacity_recurrence (K := 3) (by decide) 1\n  have hq : 1/3 = (0 : Nat) := by decide\n  simp only [hq, Rat.pow_succ, Rat.pow_zero, Rat.natCast_ofNat] at h\n  grind\nexample : (2 : Nat)%3 = 3-1 := by decide\nexample : capacity 3 3 = capacity 3 2*(9/4) := by\n  have h := capacity_recurrence (K := 3) (by decide) 2\n  have hq : 2/3 = (0 : Nat) := by decide\n  simp only [hq, Rat.pow_succ, Rat.pow_zero, Rat.natCast_ofNat] at h\n  grind\nexample (n : Nat) : capacity 1 (n+1) = capacity 1 n := by\n  simp only [capacity_single_outcome]\n\n');check(p)
            p=Path(d)/'rollover.lean';p.write_text('import VerumExternalReplay.CapacityRecurrence\nopen RefinementCapacity RefinementFeasibility\nset_option warningAsError true\n\n-- Using q=(n+1)/K too early at K=3,n=2 produces the false multiplier 81/80.\nexample : capacity 3 3 = capacity 3 2*(81/80) := by\n  have h := capacity_recurrence (K := 3) (by decide) 2\n  have hp := capacity_positive (K := 3) (by decide) 2\n  have hq : 2/3 = (0 : Nat) := by decide\n  simp only [hq, Rat.pow_succ, Rat.pow_zero, Rat.natCast_ofNat] at h\n  grind\n');check(p,'`grind` failed')

if __name__=='__main__':unittest.main()
