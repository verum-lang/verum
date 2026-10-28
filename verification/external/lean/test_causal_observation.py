"""Exact causal codec, input preservation and pre-outcome reading controls."""
from pathlib import Path
import re
import tempfile
import unittest
from check_typed_programs import ROOT, check, require_success, run

HEADER = '''import VerumExternalReplay.CausalObservation
set_option warningAsError true
open CausalObservation
def readCode : FiniteCode.Code RetainedNative.Primitive Nat :=
  .mk [⟨.primitive .add,[.input 0,.input 1],none⟩] (some 0) []
def updateCode : FiniteCode.Code RetainedNative.Primitive Nat :=
  .mk [⟨.primitive .identity,[.input 2],none⟩] (some 0) []
def readBinding : ObserverAdmission.Binding Nat := ⟨1,1,[.state,.external 0]⟩
def updateBinding : ObserverAdmission.Binding Nat := ⟨0,2,[.state,.external 0,.external 1]⟩
def policy := protocol readCode updateCode readBinding updateBinding []
def input : Input := ⟨(1,[5]),(0,[83])⟩
'''

class CausalObservationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls): require_success(run('lake','build','VerumExternalReplay.CausalObservation'))

    def prove(self, body, failure=False):
        with tempfile.TemporaryDirectory(prefix='verum-causal-codec-') as directory:
            path=Path(directory)/'check.lean';path.write_text(HEADER+body)
            expected='`decide_cbv` failed: the proposition evaluates to `false`' if failure else None
            result=check(path,expected)
            if failure:
                errors=[d for d in result['diagnostics'] if d['severity']=='error']
                self.assertEqual(len(errors),1,errors);self.assertTrue(errors[0]['data'].startswith(expected))
            return result

    def test_codec_and_input_authority(self):
        self.prove('''
example : decode [255,255,255,255,255,255,255,255] = some (2^64-1) := by decide_cbv
example : decode [256,0,0,0,0,0,0,0] = none := by decide_cbv
example : decode [7] = none := by decide_cbv
example : decode [7,0,0,0,0,0,0,0,0] = none := by decide_cbv
example : appendOutcome (0,[83]) [7,0,0,0,0,0,0,0] = some (0,[83,7]) := by decide_cbv
example : (policy.predict 9 PredictiveReplay.initial input).map (fun p => (p.context,p.mask)) =
    some ([],[14,0,0,0,0,0,0,0]) := by decide_cbv
example : policy.update 9 input [7,0,0,0,0,0,0,0] = some 7 := by decide_cbv
example : policy.update 9 ⟨(1,[5]),(0,[83,6])⟩ [7,0,0,0,0,0,0,0] = none := by decide_cbv
''')

    def test_exact_axiom_roster(self):
        names=re.findall(r'^theorem (\w+)',(ROOT/'VerumExternalReplay/CausalObservation.lean').read_text(),re.M)
        result=self.prove('\n'.join('#print axioms CausalObservation.'+n for n in names))
        rows=[d['data'] for d in result['diagnostics'] if d['severity']=='information'];self.assertEqual(len(rows),len(names))
        for row in rows:self.assertTrue(row.endswith(('does not depend on any axioms','depends on axioms: [propext]',
            'depends on axioms: [propext, Quot.sound]')),row)

    def test_forged_outcome_and_interface_fail(self):
        for claim in ['appendOutcome (0,[83]) [7,0,0,0,0,0,0,0] = some (1,[83,7])',
                      'appendOutcome (0,[83]) [7,0,0,0,0,0,0,0] = some (0,[83,8])',
                      'policy.update 9 input [7,0,0,0,0,0,0,0] = some 9']:
            with self.subTest(claim=claim):self.prove('example : '+claim+' := by decide_cbv',True)

if __name__=='__main__':unittest.main()
