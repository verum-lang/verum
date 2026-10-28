"""Frozen forecast semantics, pause safety and repeated-cell multiplicity."""
from pathlib import Path
import re
import tempfile
import unittest
from check_typed_programs import ROOT, check, require_success, run

HEADER = '''import VerumExternalReplay.FrozenTraining
set_option warningAsError true
open ExactMass CausalCoding FrozenTraining
def zeroBank : Counts Nat := fun _ _ => 0
def rule := ktRule (fun _ : Nat => 0) (fun n _ => n+1)
def start := begin rule zeroBank 0
def finished := feed rule start [false,false]
'''
FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'

class FrozenTrainingTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake','build','VerumExternalReplay.FrozenTraining'))

    def prove(self, source, expected=None):
        with tempfile.TemporaryDirectory(prefix='verum-frozen-training-') as directory:
            path=Path(directory)/'check.lean';path.write_text(HEADER+source)
            result=check(path,expected)
            if expected:
                errors=[d for d in result['diagnostics'] if d['severity']=='error']
                self.assertEqual(len(errors),1,errors)
                self.assertTrue(errors[0]['data'].startswith(expected),errors)
            return result

    def test_axioms(self):
        names=re.findall(r'^theorem (\w+)',(ROOT/'VerumExternalReplay/FrozenTraining.lean').read_text(),re.M)
        result=self.prove('\n'.join('#print axioms FrozenTraining.'+n for n in names))
        rows=[d['data'] for d in result['diagnostics'] if d['severity']=='information']
        self.assertEqual(len(rows),len(names))
        for row in rows:
            self.assertTrue(row.endswith(('does not depend on any axioms','depends on axioms: [propext]',
                'depends on axioms: [propext, Quot.sound]')),row)

    def test_aliasing_keeps_forecast_but_counts_both_visits(self):
        self.prove('''
example : Equal finished.mass ⟨1,4,by decide⟩ := by decide_cbv
example : Equal (likelihood (PositionPrediction.kernel (fun _ => (0:Nat))) ⟨0,[]⟩ [false,false])
  ⟨3,8,by decide⟩ := by decide_cbv
example : finished.frozen 0 false = 0 := by decide_cbv
example : finished.delta 0 false = 2 := by decide_cbv
example : (result rule finished).bank 0 false = 2 := by decide_cbv
example : (result rule finished).bank 0 true = 0 := by decide_cbv
example (body : List Bool) : Equal (feed rule start body).mass (likelihood (rule.forecast zeroBank) 0 body) :=
  score_matches_forecast rule zeroBank 0 body
example (n : Nat) : Equal (sum ((words n).map (fun body => (feed rule start body).mass))) one :=
  scored_normalized rule zeroBank 0 n
''')

    def test_pause_chunk_and_whole_row_boundaries(self):
        self.prove('''
example : resume rule start [some false,none,none,some false] = finished :=
  pauses_preserve_result rule start _
example (first second : List Bool) : feed rule start (first++second) = feed rule (feed rule start first) second :=
  feed_append rule start first second
example : (finish rule ⟨step rule start false,[false]⟩).isNone = true := by decide_cbv
example : ((finish rule ⟨finished,[]⟩).map (fun r => r.bank 0 false)) = some 2 := by decide_cbv
example : (advance rule ⟨finished,[]⟩).frame.delta 0 false = 2 := by decide_cbv
example : (completion rule (advance rule ⟨start,[false,false]⟩)).bank 0 false =
    (completion rule ⟨start,[false,false]⟩).bank 0 false :=
  congrArg (fun r => r.bank 0 false) (advance_preserves_completion rule ⟨start,[false,false]⟩)
example : collect [(3,false),(3,false),(4,true),(3,true)] (fun _ _ => 0) 3 false = 2 := by decide_cbv
''')

    def test_incorrect_early_learning_and_deduplication_rejected(self):
        for claim in [
            'Equal finished.mass ⟨3,8,by decide⟩',
            'finished.frozen 0 false = 2',
            'finished.delta 0 false = 1',
            '(finish rule ⟨step rule start false,[false]⟩).isSome = true',
            '(resume rule start [none,none]).delta 0 false = 2',
        ]:
            with self.subTest(claim=claim):self.prove('example : '+claim+' := by decide_cbv',FALSE)

if __name__=='__main__':unittest.main()
