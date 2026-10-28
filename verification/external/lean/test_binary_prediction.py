"""The wire kernel shares categorical KT scoring and retained counts exactly."""
from pathlib import Path
import re
import tempfile
import unittest

from check_typed_programs import ROOT, check, require_success, run

HEADER = '''import VerumExternalReplay.BinaryPrediction
import VerumExternalReplay.FiniteWire
open ExactMass CausalCoding BinaryPrediction FiniteWire
set_option warningAsError true

def trained := fromCounts 3 1

def bytePredictor : Model BinaryPrediction.State where
  unitBits := 8
  positiveWidth := by decide
  length := geometric
  kernel _ := BinaryPrediction.kernel
  initial _ := trained
  prediction _ := [true]
'''


class BinaryPredictionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.BinaryPrediction',
                            'VerumExternalReplay.FiniteWire'))

    def prove(self, source, expected=None):
        with tempfile.TemporaryDirectory(prefix='verum-binary-prediction-') as directory:
            path = Path(directory) / 'check.lean'
            path.write_text(HEADER + source)
            return check(path, expected)

    def test_axiom_roster(self):
        names = re.findall(r'^theorem (\w+)',
            (ROOT / 'VerumExternalReplay/BinaryPrediction.lean').read_text(), re.M) + ['kernel']
        self.assertIn('path_correspondence', names)
        names = ['BinaryPrediction.' + name for name in names] + ['ExactMass.divide_common_scale']
        result = self.prove('\n'.join('#print axioms ' + name for name in names))
        rows = [row['data'] for row in result['diagnostics'] if row['severity'] == 'information']
        self.assertEqual(len(rows), len(names))
        for row in rows:
            self.assertTrue(row.endswith(('does not depend on any axioms',
                'depends on axioms: [propext]', 'depends on axioms: [propext, Quot.sound]')), row)

    def test_exact_counts_probabilities_and_wire_composition(self):
        self.prove('''
example : Equal (kernel.probability trained false) ⟨7, 10, by decide⟩ := by decide_cbv
example : Equal (likelihood kernel trained [true, false]) ⟨7, 40, by decide⟩ := by decide_cbv
example : (advance kernel trained [true, false]).val.counts = [4, 2] := by decide_cbv
example : Equal (likelihood kernel (fromCounts 0 0) [false, false]) ⟨3, 8, by decide⟩ := by decide_cbv
example (s : BinaryPrediction.State) (body : List Bool) :
    CategoricalPrediction.path s.val (body.map symbol) =
      some (likelihood kernel s body, (advance kernel s body).val) := path_correspondence s body
example (n : Nat) : Equal (sliceMass bytePredictor n) (geometric.weight n) := slice_exact bytePredictor n
example (body : List Bool) : (advance kernel trained body).val.counts.sum = 4 + body.length :=
  counts_every_bit trained body
example (s : BinaryPrediction.State) (body : List Bool) : 0 < (likelihood kernel s body).numerator :=
  likelihood_positive s body
example (q p scale : Fraction) (hp : 0 < p.numerator) (hs : 0 < scale.numerator) :
    Equal (divide (mul scale q) (mul scale p) (Nat.mul_pos hs hp)) (divide q p hp) :=
  divide_common_scale q p scale hp hs
''')

    def test_state_and_alphabet_drift_rejected(self):
        claims = {
            'reversed_symbols': 'symbol true = 0',
            'training_reset':
                'Equal (likelihood kernel trained [true]) (likelihood kernel (fromCounts 0 0) [true])',
            'score_after_update':
                'Equal (kernel.probability trained true) (kernel.probability (next trained true) true)',
            'forgotten_update':
                'Equal (likelihood kernel (fromCounts 0 0) [false, false]) (unit 4 (by decide))',
            'double_count': '(next trained true).val.counts = [3, 3]',
            'omitted_bit': '(advance kernel trained [true, false]).val.counts = [3, 2]',
            'cancelling_only_one_length_factor':
                'Equal (divide (unit 4 (by decide)) (unit 8 (by decide)) (by decide)) one',
        }
        for name, claim in claims.items():
            with self.subTest(name=name):
                self.prove(f'example : {claim} := by decide_cbv\n',
                           '`decide_cbv` failed: the proposition evaluates to `false`')
        self.prove('example : BinaryPrediction.State := ⟨⟨[0, 0, 0], by decide⟩, by decide_cbv⟩\n',
                   '`decide_cbv` failed: the proposition evaluates to `false`')


if __name__ == '__main__':
    unittest.main()
