"""Finite KT claims use the shared predictor and strictly reject false variants."""
from pathlib import Path
import re
import tempfile
import unittest

from check_typed_programs import ROOT, check, require_success, run

HEADER = '''import VerumExternalReplay.KTMerge
open ExactMass CausalCoding CategoricalPrediction KTMerge
set_option warningAsError true
'''


class KTMergeTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.KTMerge'))

    def prove(self, source, expected=None):
        with tempfile.TemporaryDirectory(prefix='verum-kt-merge-') as directory:
            path = Path(directory) / 'check.lean'
            path.write_text(HEADER + source)
            result = check(path, expected)
            if expected:
                errors = [row for row in result['diagnostics'] if row['severity'] == 'error']
                self.assertEqual(len(errors), 1, result)
                self.assertTrue(errors[0]['data'].startswith(expected), result)
            return result

    def test_axiom_roster(self):
        names = re.findall(r'^theorem (\w+)',
            (ROOT / 'VerumExternalReplay/KTMerge.lean').read_text(), re.M)
        self.assertEqual(len(names), 10)
        result = self.prove('\n'.join('#print axioms KTMerge.' + name for name in names))
        rows = [row['data'] for row in result['diagnostics'] if row['severity'] == 'information']
        self.assertEqual(len(rows), len(names))
        for name, row in zip(names, rows):
            # Existing predictor proofs use Lean's standard propositional
            # extensionality and quotient soundness. Pin the full roster: no
            # domain axioms, sorryAx, native reduction trust, or choice.
            axioms = '[propext]' if name.endswith('_under_fair_iid') else '[propext, Quot.sound]'
            self.assertEqual(row, f"'KTMerge.{name}' depends on axioms: {axioms}")

    def test_actual_predictor_ratios_and_purchase(self):
        self.prove('''
example : Equal (mergerRatio zeros16 zeros16)
    ⟨315208830571481, 62142696120150, by decide⟩ := same_ratio
example : Equal (mergerRatio zeros16 ones16) ⟨1, 601080390, by decide⟩ := opposite_ratio
example : codePreference (unit 3 (by decide)) (wordMass (zeros16 ++ zeros16))
    (mul (wordMass zeros16) (wordMass zeros16)) := same_histories_pay_pair_price
example : ¬ codePreference (unit 3 (by decide)) (wordMass (zeros16 ++ ones16))
    (mul (wordMass zeros16) (wordMass ones16)) := opposite_histories_do_not_pay_pair_price
example (left right : List Bool) (w : Fraction) (hw : 0 < w.numerator) :
    codePreference w (wordMass (left ++ right)) (mul (wordMass left) (wordMass right)) ↔
    Less (divide one w hw) (mergerRatio left right) := merger_threshold left right w hw
''')

    def test_same_iid_law_does_not_force_matching_samples(self):
        self.prove('''
example : Equal (likelihood fairIID () (zeros16 ++ zeros16))
    (unit 4294967296 (by decide)) := same_histories_possible_under_fair_iid
example : Equal (likelihood fairIID () (zeros16 ++ ones16))
    (unit 4294967296 (by decide)) := opposite_histories_possible_under_fair_iid
example : ¬ Equal (mergerRatio zeros16 zeros16) (mergerRatio zeros16 ones16) := by decide_cbv
''')

    def test_empty_words_ties_and_ordered_word_mass(self):
        self.prove('''
example : Equal (mergerRatio [] zeros16) one := by decide_cbv
example : ¬ codePreference one (wordMass zeros16) (mul (wordMass []) (wordMass zeros16)) := by decide_cbv
example : Equal (wordMass [false, true]) (unit 8 (by decide)) := by decide_cbv
example : Equal (add (wordMass [false, true]) (wordMass [true, false]))
    (unit 4 (by decide)) := by decide_cbv
''')

    def test_false_claims_reach_proof_rejection(self):
        claims = {
            'binary_eternal_refusal':
                '¬ codePreference (unit 3 (by decide)) (wordMass (zeros16 ++ zeros16)) '
                '(mul (wordMass zeros16) (wordMass zeros16))',
            'same_law_forces_matching_histories':
                'Equal (mergerRatio zeros16 zeros16) (mergerRatio zeros16 ones16)',
            'opposite_histories_gain': 'Less one (mergerRatio zeros16 ones16)',
            'rounded_numerator':
                'Equal (mergerRatio zeros16 zeros16) ⟨315208830571480, 62142696120150, by decide⟩',
            'reset_merged_counts':
                'Equal (mergerRatio zeros16 zeros16) one',
            'counts_event_is_ordered_word':
                'Equal (wordMass [false, true]) (unit 4 (by decide))',
            'tie_pays':
                'codePreference one (wordMass zeros16) (mul (wordMass []) (wordMass zeros16))',
            'same_iid_law_excludes_opposites':
                'Equal (likelihood fairIID () (zeros16 ++ ones16)) zero',
        }
        for name, claim in claims.items():
            with self.subTest(name=name):
                self.prove(f'example : {claim} := by decide_cbv\n',
                           '`decide_cbv` failed: the proposition evaluates to `false`')


if __name__ == '__main__':
    unittest.main()
