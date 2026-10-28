"""Structured causal inputs retain bytes and every pre-event query field."""
from pathlib import Path
import re
import tempfile
import unittest
from check_typed_programs import ROOT, check, require_success, run

HEADER='''import VerumExternalReplay.AcquiredCausalObservation
set_option warningAsError true
open AcquiredObserver AcquiredCausalObservation
def event : Event := ⟨7,11,some ⟨19,23⟩,29,31,3,37,41,some 43,some ⟨[47],[53]⟩⟩
def source := eventSource event
def row : External := (source,[.event event])
def query := queryRow event
def context : Nat × UInt64 × UInt8 := (7,31,3)
def readCode : FiniteCode.Code Op Value :=
  .mk [⟨.primitive .queryRaw,[.input 0],none⟩] (some 0) []
def updateCode : FiniteCode.Code Op Value :=
  .mk [⟨.primitive .identity,[.input 0],none⟩] (some 0) []
def readBinding : ObserverAdmission.Binding Source := ⟨query.1,1,[.external 0]⟩
def updateBinding : ObserverAdmission.Binding Source := ⟨source,2,[.external 1]⟩
def prediction : Value → Option PredictiveReplay.Prediction
  | .nat n => some ⟨[],[n.toNat]⟩ | _ => none
def policy := protocol 8 readCode updateCode readBinding updateBinding prediction
def input : Input := ⟨query,row⟩
'''

class AcquiredCausalObservationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake','build','VerumExternalReplay.AcquiredCausalObservation'))

    def prove(self,body,failure=False):
        with tempfile.TemporaryDirectory(prefix='verum-acquired-causal-') as tmp:
            path=Path(tmp)/'check.lean';path.write_text(HEADER+body)
            expected='`decide_cbv` failed: the proposition evaluates to `false`' if failure else None
            result=check(path,expected)
            if failure:
                errors=[d for d in result['diagnostics'] if d['severity']=='error']
                self.assertEqual(len(errors),1,errors)
                self.assertTrue(errors[0]['data'].startswith(expected),errors)
            return result

    def test_complete_wire_and_pre_event_query(self):
        self.prove('''
example : decode [] = some (.bytes []) := by decide_cbv
example : decode [0,255,1,0] = some (.bytes [0,255,1,0]) := by decide_cbv
example : decode [256] = none := by decide_cbv
example : appendOutcome row [255,0] = some (source,[.event event,.bytes [255,0]]) := by decide_cbv
example : validRow source row query (.bytes [255,0]) context = true := by decide_cbv
example : validRow source row query (.nat 255) context = false := by decide_cbv
example : matchesCompleted row (.bytes [255,0]) ⟨row,[255,0]⟩ = true := by decide_cbv
example : matchesCompleted row (.bytes []) ⟨row,[]⟩ = true := by decide_cbv
example : (policy.predict (.bytes [99]) PredictiveReplay.initial input).map
  (fun p => (p.context,p.mask)) = some ([],[29]) := by decide_cbv
example : policy.update (.nat 0) input [255,0] = some (.bytes [255,0]) := by decide_cbv
example : policy.update (.nat 0) ⟨query,({source with epoch := 8},row.2)⟩ [255,0] = none := by decide_cbv
example : policy.update (.nat 0) ⟨query,(source,row.2++[.bytes [1]])⟩ [255,0] = none := by decide_cbv
example : policy.update (.nat 0) input [256] = none := by decide_cbv
example : OutcomeTransitions.evolve (update 8 updateCode updateBinding)
  [⟨row,[1]⟩,⟨row,[255,0]⟩] (.nat 0) = some (.bytes [255,0]) := by decide_cbv
''')

    def test_aliases_and_foreign_rows_fail_in_kernel(self):
        claims=[
            'decode [256] = some (.bytes [0])',
            'appendOutcome row [255,0] = some (source,[.event event,.bytes [255]])',
            'policy.update (.nat 0) input [255,0] = some (.bytes [53])',
            'validRow source row (queryRow {event with sensory := some ⟨[47],[54]⟩}) (.bytes []) context = true',
            'validRow source row (queryRow {event with pose := some 44}) (.bytes []) context = true',
            'validRow source row (queryRow {event with owner := some ⟨19,24⟩}) (.bytes []) context = true',
            'validRow source row (queryRow {event with action := 4}) (.bytes []) context = true',
            'validRow source row query (.bytes []) (7,31,4) = true',
            'validRow {source with sensory := false} row query (.bytes []) context = true',
            'matchesCompleted row (.bytes [255]) ⟨row,[255,0]⟩ = true',
            'matchesCompleted row (.bytes [0]) ⟨row,[256]⟩ = true',
            'matchesCompleted ({source with epoch := 8},row.2) (.bytes []) ⟨row,[]⟩ = true',
        ]
        for claim in claims:
            with self.subTest(claim=claim):self.prove('example : '+claim+' := by decide_cbv',True)

    def test_exact_axiom_roster(self):
        names=re.findall(r'^theorem (\w+)',
            (ROOT/'VerumExternalReplay/AcquiredCausalObservation.lean').read_text(),re.M)
        result=self.prove('\n'.join('#print axioms AcquiredCausalObservation.'+n for n in names))
        rows=[d['data'] for d in result['diagnostics'] if d['severity']=='information']
        self.assertEqual(len(rows),len(names))
        for row in rows:self.assertTrue(row.endswith(('does not depend on any axioms',
            'depends on axioms: [propext]','depends on axioms: [propext, Quot.sound]')),row)

if __name__=='__main__':unittest.main()
