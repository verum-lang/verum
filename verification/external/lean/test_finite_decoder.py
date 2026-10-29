"""Finite empirical decoder ceilings; no dataset or population generalization."""
from pathlib import Path
import re
import tempfile
import unittest
from check_typed_programs import ROOT, check, require_success, run

HEADER = '''import VerumExternalReplay.FiniteDecoder
open FiniteDecoder
set_option warningAsError true
def coarse : Cohort 2 1 := [(0,0),(0,0),(0,1),(1,0),(1,1)]
def fine : Cohort 3 1 := [(0,0),(0,0),(1,1),(2,0),(2,1)]
def coarsen (key : Fin 3) : Fin 2 := if key.val < 2 then 0 else 1
def conflict : Cohort 1 1 := [(0,0),(0,1)]
def training : Cohort 1 1 := [(0,0)]
def future : Cohort 1 1 := [(0,1)]
def otherClass : Cohort 2 2 := [(0,2),(0,2),(0,1),(1,1),(1,1)]
'''
FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'


class FiniteDecoderTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.FiniteDecoder'))

    def prove(self, source, failure=False):
        with tempfile.TemporaryDirectory(prefix='verum-finite-decoder-') as directory:
            path = Path(directory) / 'proof.lean'; path.write_text(HEADER + source)
            result = check(path, FALSE if failure else None)
        if failure:
            errors = [d for d in result['diagnostics'] if d['severity'] == 'error']
            self.assertEqual(len(errors), 1, errors)
            self.assertTrue(errors[0]['data'].startswith(FALSE), errors)
        return result

    def test_tight_bounds_count_occurrences_and_all_classes(self):
        self.prove('''
example : ceiling coarse = 3 := by decide_cbv
example : correct (majority coarse) coarse = 3 := by decide_cbv
example : coarse.length - ceiling coarse = 2 := by decide_cbv
example : majority coarse 1 = 0 := by decide_cbv
example : ceiling conflict = 1 := by decide_cbv
example : ceiling otherClass = 4 := by decide_cbv
example : majority otherClass 0 = 2 := by decide_cbv
example : ceiling ([] : Cohort 4 2) = 0 := by decide_cbv
example : ceiling ([(0,0),(0,0),(0,0)] : Cohort 1 1) = 3 := by decide_cbv
''')

    def test_refinement_and_general_factorization(self):
        self.prove('''
example : project coarsen fine = coarse := by decide_cbv
example : ceiling fine = 4 := by decide_cbv
example : correct (fun x => majority coarse (coarsen x)) fine = 3 := by decide_cbv
example (q : Fin n → Fin m) (rows : Cohort n k) :
    ceiling (project q rows) ≤ ceiling rows := refinement_monotone q rows
example (rows : Cohort n k) (decode : Fin n → Fin (k+1)) :
    correct decode rows ≤ ceiling rows := decoder_bound rows decode
example (rows : Cohort n k) :
    ∃ decode : Fin n → Fin (k+1), correct decode rows = ceiling rows :=
  ⟨majority rows, majority_attains rows⟩
example (X : Type) (q : X → Fin n) (decode : Fin n → Fin (k+1))
    (rows : List (X × Fin (k+1))) :
    correct (fun x => decode (q x)) rows ≤ ceiling (project q rows) :=
  factorized_decoder_bound q decode rows
example (rows : Cohort n k) (key : Fin n) (a b : Fin (k+1))
    (ha : (key,a) ∈ rows) (hb : (key,b) ∈ rows) (different : a ≠ b) :
    ceiling rows < rows.length := conflicting_fibre_prevents_perfection rows key a b ha hb different
''')

    def test_no_out_of_cohort_accuracy_guarantee(self):
        self.prove('''
example : correct (majority training) training = training.length := by decide_cbv
example : correct (majority training) future = 0 := by decide_cbv
example : ceiling future = future.length := by decide_cbv
''')

    def test_invalid_bounds_and_extrapolations_reach_kernel(self):
        claims = {
            'lossy_projection_perfect': 'ceiling coarse = coarse.length',
            'cap_below_attainable': 'ceiling coarse ≤ 2',
            'merge_cannot_increase_capacity': 'ceiling fine ≤ ceiling (project coarsen fine)',
            'refinement_need_not_improve': 'ceiling (project id fine) < ceiling fine',
            'retaining_raw_cannot_fix_wrong_labels': 'ceiling conflict = conflict.length',
            'discard_repeated_observations': 'ceiling ([(0,0),(0,0),(0,0)] : Cohort 1 1) = 1',
            'empty_cohort_not_accuracy_one': 'ceiling ([] : Cohort 4 2) = 1',
            'unseen_indices_add_no_capacity': 'ceiling ([(0,0)] : Cohort 5 3) = 5',
            'every_decoder_is_not_optimal': 'correct (fun _ => 1) coarse = ceiling coarse',
            'minority_cannot_attain': 'majority otherClass 0 = 1',
            'no_future_transfer': 'correct (majority training) future = future.length',
            'errors_cannot_disappear': 'coarse.length - ceiling coarse = 0',
        }
        for name, claim in claims.items():
            with self.subTest(control=name): self.prove(f'example : {claim} := by decide_cbv\n', True)

    def test_constructive_axiom_roster(self):
        source = (ROOT / 'VerumExternalReplay/FiniteDecoder.lean').read_text()
        names = re.findall(r'^theorem (\w+)', source, re.M)
        names += ['total', 'best', 'count', 'correct', 'majority', 'ceiling', 'project']
        result = self.prove('\n'.join('#print axioms FiniteDecoder.' + n for n in names) + '\n')
        rows = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
        self.assertEqual(len(rows), len(names))
        for row in rows:
            if 'depends on axioms: [' in row:
                self.assertLessEqual(set(row.split('depends on axioms: [')[1].removesuffix(']').split(', ')),
                                     {'propext', 'Quot.sound'}, row)
            else: self.assertTrue(row.endswith('does not depend on any axioms'), row)


if __name__ == '__main__': unittest.main()
