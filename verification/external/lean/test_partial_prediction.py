"""Partial selector mass bounds preserve typed refusal and pending evidence."""
from pathlib import Path
import re
import tempfile
import unittest
from check_typed_programs import ROOT, check, require_success, run

HEADER = '''import VerumExternalReplay.PartialPrediction
import VerumExternalReplay.ContextualStop
set_option warningAsError true
deriving instance DecidableEq for Except
open ExactMass PartialPrediction
def fair : Choice := ⟨fun _ => ⟨1,2,by decide⟩,by decide⟩
def machine : Nat → Step Nat Nat
  | 0 => .choose fair (fun b => if b then 2 else 1)
  | 1 => .complete
  | 2 => .refused 42
  | 3 => .pending
  | 4 => .choose fair (fun b => if b then 3 else 1)
  | _ => .choose fair (fun _ => 5)
def partialModel : Model (List Bool) Nat where
  unitBits := 2
  positiveWidth := by decide
  stop past := if past.isEmpty then .ready fair else .refused 7
  bit _ := .ready fair
  next past bit := bit::past
def fixed : ContextualStop.Selector Nat := fun _ _ => ⟨0,false⟩
def emptyBank : ContextualStop.Bank Nat := fun _ _ => 0
def totalModel := ContextualStop.model fixed emptyBank 2 (by decide)
'''
FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'


class PartialPredictionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.PartialPrediction',
                            'VerumExternalReplay.ContextualStop'))

    def prove(self, source, expected=None):
        with tempfile.TemporaryDirectory(prefix='verum-partial-prediction-') as directory:
            path = Path(directory) / 'check.lean'
            path.write_text(HEADER + source)
            result = check(path, expected)
            if expected:
                errors = [d for d in result['diagnostics'] if d['severity'] == 'error']
                self.assertEqual(len(errors), 1, errors)
                self.assertTrue(errors[0]['data'].startswith(expected), errors)
            return result

    def test_axiom_roster(self):
        source = (ROOT / 'VerumExternalReplay/PartialPrediction.lean').read_text()
        names = re.findall(r'^theorem (\w+)', source, re.M)
        result = self.prove('\n'.join('#print axioms PartialPrediction.' + n for n in names))
        rows = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
        self.assertEqual(len(rows), len(names))
        for row in rows:
            self.assertTrue(row.endswith(('does not depend on any axioms',
                'depends on axioms: [propext]', 'depends on axioms: [propext, Quot.sound]')), row)

    def test_refused_and_pending_are_distinct(self):
        self.prove('''
example : point machine 0 [false] = .ok ⟨1,2,by decide⟩ := by decide_cbv
example : point machine 0 [true] = .error (.refused 42) := by decide_cbv
example : point machine 4 [true] = .error .pending := by decide_cbv
example : point machine 0 [] = .error .incomplete := by decide_cbv
example : point machine 1 [false] = .error .pastTerminal := by decide_cbv
example : walk machine 0 [true] = .ok (⟨1,2,by decide⟩,2) := by decide_cbv
example : walk machine 4 [true] = .ok (⟨1,2,by decide⟩,3) := by decide_cbv
example : frontier machine 2 0 =
  [⟨[false],.complete,⟨1,2,by decide⟩⟩,⟨[true],.refused 42,⟨1,2,by decide⟩⟩] := by decide_cbv
example : frontier machine 2 4 =
  [⟨[false],.complete,⟨1,2,by decide⟩⟩,⟨[true],.pending 3,⟨1,2,by decide⟩⟩] := by decide_cbv
''')

    def test_subprobability_and_unresolved_tail(self):
        self.prove('''
example : Equal (mass (frontier machine 3 0)) one := by decide_cbv
example : Equal (completed (frontier machine 3 0)) ⟨1,2,by decide⟩ := by decide_cbv
example : Equal (completed (frontier machine 3 4)) ⟨1,2,by decide⟩ := by decide_cbv
example : Equal (completed (frontier machine 3 5)) zero := by decide_cbv
example : Equal (mass (frontier machine 3 5)) one := by decide_cbv
example : Equal (completed (frontier machine 0 1)) zero := by decide_cbv
example (step : S → Step S E) (n : Nat) (s : S) : AtMost (completed (frontier step n s)) one :=
  subprobability step n s
example (n : Nat) : Less (completed (frontier machine 3 0)) (mass (frontier machine n 0)) := by
  exact le_less_trans (by decide : AtMost (completed (frontier machine 3 0)) ⟨1,2,by decide⟩)
    (by have h := frontier_conserved machine n 0; simp only [Equal,one,Nat.one_mul,Nat.mul_one] at h
        simp only [Less]; rw [h]; have := (mass (frontier machine n 0)).positive; omega)
''')

    def test_trace_products_and_prefix_bounds(self):
        self.prove('''
example : Equal (mul ⟨1,2,by decide⟩ (completed (frontier machine 3 2))) zero := by decide_cbv
example (n : Nat) : AtMost (mul ⟨1,2,by decide⟩ (completed (frontier machine n 2))) ⟨1,2,by decide⟩ :=
  (prefix_upper_bound machine 0 [true] ⟨1,2,by decide⟩ 2 (by decide) n).2
example (n : Nat) (large : 1 < n) : (⟨[false],.complete,⟨1,2,by decide⟩⟩ : Entry Nat Nat) ∈ frontier machine n 0 :=
  trace_in_frontier machine 0 [false] ⟨1,2,by decide⟩ (by decide) n large
example (bits : List Bool) : point machine 0 ([false]++bits) =
    (point machine 1 bits).map (mul ⟨1,2,by decide⟩) :=
  point_extension machine 0 [false] bits ⟨1,2,by decide⟩ 1 (by decide)
''')

    def test_partial_stop_bit_and_total_adapter(self):
        self.prove('''
example : point (execute partialModel) (.boundary []) [true] = .ok ⟨1,2,by decide⟩ := by decide_cbv
example : point (execute partialModel) (.boundary []) [false,false,true,true] = .error (.refused 7) := by decide_cbv
example : walk (execute partialModel) (.boundary []) [false,false,true] =
    .ok (⟨1,8,by decide⟩,.boundary [true,false]) := by decide_cbv
example : Equal (completed (frontier (execute partialModel) 5 (.boundary []))) ⟨1,2,by decide⟩ := by decide_cbv
example : Equal (mass (frontier (execute partialModel) 5 (.boundary []))) one := by decide_cbv
example : (point (stopped totalModel) (.boundary []) [false,false,true,true]).toOption.map
    (fun p => decide (Equal p (StoppedProcess.point totalModel [] [[false,true]]))) = some true := by decide_cbv
example : point (stopped totalModel) (.boundary []) [false,false,true] = .error .incomplete := by decide_cbv
''')

    def test_false_promotions_rejected(self):
        claims = {
            'undefined_is_zero': 'point machine 0 [true] = .ok zero',
            'pending_is_zero': 'point machine 4 [true] = .ok zero',
            'pending_is_undefined': 'point machine 4 [true] = .error (.refused 42)',
            'completed_renormalized': 'point machine 0 [false] = .ok one',
            'subprobability_promoted': 'Equal (completed (frontier machine 3 0)) one',
            'prefix_exact_without_totality': 'Equal (mul ⟨1,2,by decide⟩ (completed (frontier machine 3 2))) ⟨1,2,by decide⟩',
            'pending_mass_discarded': 'Equal (mass (frontier machine 3 4)) ⟨1,2,by decide⟩',
            'refused_mass_discarded': 'Equal (mass (frontier machine 3 0)) ⟨1,2,by decide⟩',
            'horizon_exhaustion_is_done': 'Equal (completed (frontier machine 0 1)) one',
            'infinite_path_is_done': 'Equal (completed (frontier machine 3 5)) one',
            'known_prefix_proves_stop': 'point (execute partialModel) (.boundary []) [false,false,true,true] = .ok ⟨1,16,by decide⟩',
            'no_final_stop_needed': 'point (stopped totalModel) (.boundary []) [false,false,true] = .ok ⟨1,8,by decide⟩',
        }
        for name, claim in claims.items():
            with self.subTest(name=name):
                self.prove('example : ' + claim + ' := by decide_cbv', FALSE)


if __name__ == '__main__':
    unittest.main()
