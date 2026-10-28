"""Universal KT factors and balanced products; no finite-search proof substitute."""
from pathlib import Path
import tempfile
import unittest
from check_typed_programs import check, require_success, run

HEADER = ('import VerumExternalReplay.CapacityFactors\n'
          'import VerumExternalReplay.BalancedProduct\nopen RefinementFeasibility\n')

class CapacityProductTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake','build','VerumExternalReplay.CapacityFactors','VerumExternalReplay.BalancedProduct'))

    def test_axioms(self):
        names = ['RefinementFeasibility.' + n for n in (
            'rising_pos self_power_pos weight_zero weight_pos power_gap baseStep_pos '
            'baseStep_next_lt weightStep_pos weightStep_next_lt weight_succ weightStep_antitone '
            'weight_slope_antitone').split()]
        names += ['BalancedProduct.' + n for n in (
            'lower_growth upper_growth envelope product_envelope maximum balanced_length '
            'balanced_sum balanced_attains').split()]
        with tempfile.TemporaryDirectory() as directory:
            p=Path(directory)/'axioms.lean';p.write_text(HEADER+'\n'.join('#print axioms '+n for n in names))
            rows=[d['data'] for d in check(p)['diagnostics'] if d['severity']=='information']
        self.assertEqual(len(rows),len(names))
        allowed={'propext','Classical.choice','Quot.sound'}
        for row in rows:
            if row.endswith('does not depend on any axioms'): continue
            self.assertIn('depends on axioms: [',row)
            self.assertTrue(set(row.split('depends on axioms: [')[1].rstrip(']').split(', '))<=allowed,row)

    def test_exact_factors_and_universal_maximum(self):
        text=HEADER+'''
example : weight 2 2 = 3/32 := by
  simp only [weight, rising, Rat.pow_succ, Rat.pow_zero]; grind
example : weight 1 2 = 1/4 := by
  simp only [weight, rising, Rat.pow_succ, Rat.pow_zero]; grind
example (K : Nat) (hK : 0 < K) (xs : List Nat) :
    BalancedProduct.product (weight K) xs ≤ BalancedProduct.bound (weight K) xs.length xs.sum :=
  BalancedProduct.maximum (weight K) (weight_pos hK) (fun _ _ h => weight_slope_antitone hK h) xs
example (K n : Nat) (hK : 0 < K) :
    BalancedProduct.product (weight K) (BalancedProduct.balancedCounts K n) =
      BalancedProduct.bound (weight K) K n :=
  BalancedProduct.balanced_attains (weight K) (weight_pos hK) K n hK
example : (BalancedProduct.balancedCounts 3 8).sum = 8 :=
  BalancedProduct.balanced_sum 3 8 (by decide)
example : (BalancedProduct.balancedCounts 3 8).length = 3 :=
  BalancedProduct.balanced_length 3 8 (by decide)
example : weight 2 1001*weight 2 0 ≤ weight 2 1000*weight 2 1 :=
  weight_slope_antitone (by decide) (by decide)
'''
        with tempfile.TemporaryDirectory() as directory:
            p=Path(directory)/'positive.lean';p.write_text(text);check(p)

    def test_reversed_transfer_and_empty_alphabet_rejected(self):
        claims=['weight 2 1*weight 2 1 ≤ weight 2 0*weight 2 2','0 < weight 0 1']
        with tempfile.TemporaryDirectory() as directory:
            for n,claim in enumerate(claims):
                p=Path(directory)/f'negative{n}.lean'
                p.write_text(HEADER+f'example : {claim} := by\n  simp only [weight, rising, Rat.pow_succ, Rat.pow_zero]\n  grind\n')
                check(p,'`grind` failed')

if __name__=='__main__':unittest.main()
