"""Observed-outcome updates, exact replay and nominal publication controls."""
from pathlib import Path
import re
import tempfile
import unittest
from check_typed_programs import ROOT, check, require_success, run

HEADER='''import VerumExternalReplay.OutcomeTransitions
set_option warningAsError true
open OutcomeTransitions
def protocol : Protocol (List Bool) Nat Unit Nat Bool where
  predict state bank _ := some (state.length+bank)
  update state _ outcome := some (state++[outcome])
  learn bank _ _ := some (bank+1)
def ticket : Frozen Nat (List Bool) Nat Unit Nat := ⟨42,[],0,(),0⟩
def ready : Ready Nat (List Bool) Nat Unit Nat Bool := ⟨ticket,true,[true],1⟩
def first : List (Row Unit Bool) := [⟨(),false⟩,⟨(),true⟩]
def second : List (Row Unit Bool) := [⟨(),true⟩,⟨(),false⟩]
def origin (_ : Nat) : AtomicPublication.Frontier Nat Nat Nat Nat := ⟨⟨1,2,3⟩,4⟩
'''
FALSE='`decide_cbv` failed: the proposition evaluates to `false`'

class OutcomeTransitionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):require_success(run('lake','build','VerumExternalReplay.OutcomeTransitions'))

    def prove(self, source, expected=None):
        with tempfile.TemporaryDirectory(prefix='verum-outcome-transition-') as directory:
            path=Path(directory)/'check.lean';path.write_text(HEADER+source)
            result=check(path,expected)
            if expected:
                errors=[d for d in result['diagnostics'] if d['severity']=='error']
                self.assertEqual(len(errors),1,errors)
                self.assertTrue(errors[0]['data'].startswith(expected),errors)
            return result

    def test_axiom_roster(self):
        names=re.findall(r'^theorem (\w+)',(ROOT/'VerumExternalReplay/OutcomeTransitions.lean').read_text(),re.M)
        result=self.prove('\n'.join('#print axioms OutcomeTransitions.'+n for n in names))
        rows=[d['data'] for d in result['diagnostics'] if d['severity']=='information']
        self.assertEqual(len(rows),len(names))
        for row in rows:
            self.assertTrue(row.endswith(('does not depend on any axioms','depends on axioms: [propext]',
                'depends on axioms: [propext, Quot.sound]')),row)

    def test_old_invariance_and_outcome_sensitive_history(self):
        self.prove('''
example (old : Nat → Unit → Option Nat) : evolve (ignoreOutcome old) first 36 =
    evolve (ignoreOutcome old) second 36 := old_outcome_invariance old first second 36 rfl
example : evolve (ignoreOutcome (fun s (_ : Unit) => some s)) first 36 = some 36 := by decide_cbv
example : (evolve (ignoreOutcome (fun (_ : Nat) (_ : Unit) => none)) first 36).isNone = true := by decide_cbv
example : evolve protocol.update first [] = some [false,true] := by decide_cbv
example : evolve protocol.update second [] = some [true,false] := by decide_cbv
example : replay protocol first ([],0) = some ([false,true],2) := by decide_cbv
example : replay protocol second ([],0) = some ([true,false],2) := by decide_cbv
example (before after : List (Row Unit Bool)) : replay protocol (before++after) ([],0) =
    (replay protocol before ([],0)).bind (replay protocol after) := replay_append protocol before after ([],0)
''')

    def test_forecast_capture_and_publication_boundaries(self):
        self.prove('''
example : (complete protocol ticket ⟨43,true⟩).isNone = true := by decide_cbv
example : (complete protocol ticket ⟨42,true⟩).map (fun r => r.before.forecast) = some 0 := by decide_cbv
example : (complete protocol ticket ⟨42,false⟩).map (fun r => r.before.forecast) = some 0 := by decide_cbv
example : (complete protocol ticket ⟨42,true⟩).map (fun r => r.state) = some [true] := by decide_cbv
example : (complete {protocol with learn := fun _ _ _ => none} ticket ⟨42,true⟩).isNone = true := by decide_cbv
example : (AtomicPublication.commit (origin 42) (publication origin ready 5 6)).current.cut.state = 5 := by decide_cbv
example : (AtomicPublication.commit (origin 42) (publication origin ready 5 6)).current.bank = 6 := by decide_cbv
example : (AtomicPublication.commit (origin 42) (publication origin ready 5 6)).current.cut.owner = 2 := by decide_cbv
example : AtomicPublication.admissible (AtomicPublication.fork (origin 42) 7)
    (publication origin ready 5 6) = false := by decide_cbv
example : AtomicPublication.admissible
    (AtomicPublication.commit (origin 42) (publication origin ready 5 6)).current
    (publication origin ready 5 6) = false := by decide_cbv
def shared := FrozenTraining.ktRule (fun _ : Nat => 0) (fun n _ => n+1)
def joint := fromFrozen shared (fun (_ : List Bool) _ (_ : Unit) => some 0)
  (fun state _ outcome => some (state++outcome))
def jointTicket : Frozen Nat (List Bool) (FrozenTraining.Counts Nat) Unit Nat :=
  ⟨42,[],fun _ _ => 0,(),0⟩
example : (complete joint jointTicket ⟨42,[false,false]⟩).map (fun r => (r.state,r.bank 0 false)) =
  some ([false,false],2) := by decide_cbv
''')

    def test_existing_predictive_replay_embeds_with_failures(self):
        self.prove('''
def oldRead (state : Nat) (_ : Unit) : Option PredictiveReplay.Prediction :=
  if state = 99 then none else some ⟨[],[]⟩
def oldUpdate (state : Nat) (_ : Unit) : Option Nat :=
  if state = 100 then none else some (state+1)
def oldRows : List (PredictiveReplay.Record Unit Unit) := [⟨(),(),[]⟩,⟨(),(),[0]⟩]
example (rows : List (PredictiveReplay.Record Unit Unit)) (state : PredictiveReplay.State Nat) :
    replay (fromPredictive oldRead oldUpdate) (rows.map predictiveRow) (state.vm,state.bank) =
    (PredictiveReplay.replay oldRead oldUpdate rows state).map (fun s => (s.vm,s.bank)) :=
  predictive_replay oldRead oldUpdate rows state
example : replay (fromPredictive oldRead oldUpdate) (oldRows.map predictiveRow)
    (99,PredictiveReplay.initial) = none := by decide_cbv
example : replay (fromPredictive oldRead oldUpdate) (oldRows.map predictiveRow)
    (100,PredictiveReplay.initial) = none := by decide_cbv
example : (replay (fromPredictive oldRead oldUpdate) (oldRows.map predictiveRow)
    (0,PredictiveReplay.initial)).map (fun s => (s.1,s.2.body.samples,s.2.body.bits)) =
    some (2,2,8) := by decide_cbv
''')

    def test_wrong_authority_or_outcome_is_rejected(self):
        for claim in [
            '(complete protocol ticket ⟨43,true⟩).isSome = true',
            '(complete protocol ticket ⟨42,true⟩).map (fun r => r.state) = some [false]',
            '(complete protocol ticket ⟨42,true⟩).map (fun r => r.before.forecast) = some 1',
            'replay protocol first ([],0) = replay protocol second ([],0)',
            'AtomicPublication.admissible (AtomicPublication.fork (origin 42) 7) (publication origin ready 5 6) = true',
        ]:
            with self.subTest(claim=claim):self.prove('example : '+claim+' := by decide_cbv',FALSE)

if __name__=='__main__':unittest.main()
