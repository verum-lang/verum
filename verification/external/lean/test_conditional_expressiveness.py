"""Independence obstruction and conservative causal-kernel extension."""
import gzip
import hashlib
import json
from pathlib import Path
import re
import tempfile
import unittest

from check_typed_programs import ROOT, check, require_success, run

HEADER = '''import VerumExternalReplay.ConditionalExpressiveness
set_option warningAsError true
open ExactMass CausalCoding ConditionalExpressiveness
def uniform : PositionLaw := ⟨fun _ _ => ⟨1,2,by decide⟩, fun _ => by decide⟩
def bank : PositionPrediction.Bank Nat :=
  [(0,BinaryPrediction.fromCounts 7 7),(1,BinaryPrediction.fromCounts 7 7)]
'''
FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'


class ConditionalExpressivenessTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.ConditionalExpressiveness'))

    def prove(self, source, expected=None):
        with tempfile.TemporaryDirectory(prefix='verum-conditional-family-') as directory:
            path = Path(directory)/'check.lean'
            path.write_text(HEADER + source)
            result = check(path, expected)
            if expected:
                errors = [d for d in result['diagnostics'] if d['severity'] == 'error']
                self.assertEqual(len(errors), 1, errors)
                self.assertTrue(errors[0]['data'].startswith(expected), errors)
            return result

    def test_axiom_roster(self):
        names = re.findall(r'^theorem (\w+)',
            (ROOT/'VerumExternalReplay/ConditionalExpressiveness.lean').read_text(), re.M)
        result = self.prove('\n'.join('#print axioms ConditionalExpressiveness.'+n for n in names))
        rows = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
        self.assertEqual(len(rows), len(names))
        for row in rows:
            self.assertTrue(row.endswith(('does not depend on any axioms',
                'depends on axioms: [propext]', 'depends on axioms: [propext, Quot.sound]')), row)

    def test_normalized_positive_counterexample_and_realization(self):
        self.prove('''
example : Equal (sum [correlated false false,correlated false true,
  correlated true false,correlated true true]) one := correlated_normalized
example (x y : Bool) : 0 < (correlated x y).numerator := correlated_positive x y
example : ¬ RankOne correlated := correlated_not_rank_one
example (a b : Bool → Fraction) : ¬ ∀ x y, Equal (product a b x y) (correlated x y) :=
  correlated_not_product a b
example (x y : Bool) : Equal (likelihood autoregressive .start [x,y]) (correlated x y) :=
  autoregressive_realizes x y
example (n : Nat) : Equal (sum ((words n).map (likelihood autoregressive .start))) one :=
  autoregressive_normalized n
example (law : PositionLaw) (i : Nat) : ¬ ∀ x y,
  Equal (positional law i [x,y]) (likelihood autoregressive .start [x,y]) :=
  causal_strictly_exceeds_positional law i
''')

    def test_balanced_kt_and_fixed_mask(self):
        self.prove('''
example (x y : Bool) : Equal (PositionPrediction.frozen id bank 0 [x,y]) ⟨1,4,by decide⟩ := by
  cases x <;> cases y <;> decide_cbv
example (x y : Bool) : Equal (PositionPrediction.frozen id bank 0 (residual [true,false] [x,y]))
  ⟨1,4,by decide⟩ := by cases x <;> cases y <;> decide_cbv
example : RankOne (fun x y => PositionPrediction.frozen id bank 0 [x,y]) := frozen_pair_rank_one id bank 0
example (scale : Fraction) (a b : Bool → Fraction) : RankOne (fun x y => mul scale (product a b x y)) :=
  scaled_product_rank_one scale a b
example (body : List Bool) : likelihood (positionalKernel uniform) 0 body = positional uniform 0 body :=
  positional_embedding uniform 0 body
example (state : Prefix) (body : List Bool) :
  Equal (likelihood autoregressive (id state) body) (likelihood autoregressive state body) :=
  kernel_transport autoregressive autoregressive id (fun _ _ => eq_refl _) (fun _ _ => rfl) state body
''')

    def test_false_family_and_current_read_claims_fail(self):
        for claim in [
            'RankOne correlated',
            'Equal (likelihood autoregressive .start [false,true]) ⟨3,8,by decide⟩',
            'Equal (likelihood (positionalKernel uniform) 0 [false,false]) (likelihood autoregressive .start [false,false])',
            'Equal (PositionPrediction.frozen id bank 0 [false,false]) (correlated false false)',
        ]:
            with self.subTest(claim=claim):
                self.prove('example : '+claim+' := by decide_cbv', FALSE)

    def test_pinned_native_source(self):
        path = ROOT/'fixtures/conditional-family-source.json'
        receipt = json.loads(path.read_text())
        raw = gzip.decompress(path.with_suffix('.rs.gz').read_bytes())
        self.assertEqual(hashlib.sha256(raw).hexdigest(), receipt['source_sha256'])
        # Source identity is checked; no text search is treated as a proof of Rust semantics.
        self.assertEqual(receipt['scope'], 'one fixed-model, fixed-context, fixed-mask, fixed-length conditional family')


if __name__ == '__main__':
    unittest.main()
