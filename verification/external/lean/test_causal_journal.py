"""Publication authority and the distinction between completion and target bytes."""
from pathlib import Path
import re
import tempfile
import unittest
from check_typed_programs import ROOT, check, require_success, run

HEADER = '''import VerumExternalReplay.CausalJournal
set_option warningAsError true
open CausalJournal ActionCapture
def journal : Journal Nat Nat := ⟨0,true,[]⟩
def reserved : Reservation Nat Nat Nat Nat := ⟨0,1,7,83,257⟩
def pending : Pending Nat Nat Nat Nat := ⟨journal,reserved⟩
def known : Capture Nat Unit := ⟨257,.ok .step,none,none⟩
def foreign : Capture Nat Unit := {known with action := 9}
def failed (scope : TerminalScope) (action : Nat) : Capture Nat Unit :=
  ⟨257,.error ⟨.act action,some .completed,scope,()⟩,none,none⟩
def publication (capture : Capture Nat Unit) := observe (invoke pending (fun _ => capture))
'''

class CausalJournalTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls): require_success(run('lake','build','VerumExternalReplay.CausalJournal'))

    def prove(self, body, failure=False):
        with tempfile.TemporaryDirectory(prefix='verum-causal-journal-') as directory:
            path=Path(directory)/'check.lean';path.write_text(HEADER+body)
            expected='`decide_cbv` failed: the proposition evaluates to `false`' if failure else None
            result=check(path,expected)
            if failure:
                errors=[d for d in result['diagnostics'] if d['severity']=='error']
                self.assertEqual(len(errors),1,errors)
                self.assertTrue(errors[0]['data'].startswith(expected),errors)
            return result

    def test_completion_retains_missing_bytes_and_exact_authority(self):
        self.prove('''
example : (bind journal reserved).isSome = true := by decide_cbv
example : (bind {journal with anchor := 2} reserved).isNone = true := by decide_cbv
example : (bind {journal with complete := false} reserved).isNone = true := by decide_cbv
example : (publication known).toOption.map (fun p => (p.journal,p.slot,p.capture.action)) =
  some (⟨1,true,[(83,some ⟨.step,none⟩)]⟩,7,257) := by decide_cbv
example : (publication foreign).toOption.isNone = true := by decide_cbv
example : (publication (failed .episode 257)).toOption.isNone = true := by decide_cbv
example : (publication (failed .action 9)).toOption.isNone = true := by decide_cbv
example : (publication (failed .action 257)).toOption.map (fun p => p.journal.events) =
  some [(83,some ⟨.terminal .completed .stopped,none⟩)] := by decide_cbv
example : (completedRows (publish pending none).events).isNone = true := by decide_cbv
example : (completedRows (publish pending (some ⟨.step,none⟩)).events).isSome = true := by decide_cbv
example : (target ⟨.step,none⟩).toOption = none := by decide_cbv
example : (observe (invoke {pending with reservation := {reserved with slot := 8}}
  (fun _ => known))).toOption.map (fun p => p.slot) = some 8 := by decide_cbv
''')

    def test_forged_authority_and_invented_bytes_fail(self):
        claims=['(bind {journal with anchor := 2} reserved).isSome = true',
                '(publication foreign).toOption.isSome = true',
                '(publication (failed .episode 257)).toOption.isSome = true',
                '(publication known).toOption.map (fun p => p.slot) = some 8',
                '(completedRows (publish pending none).events).isSome = true',
                '(target ⟨.step,none⟩).toOption = some ⟨.step,⟨[],[]⟩⟩']
        for claim in claims:
            with self.subTest(claim=claim):self.prove('example : '+claim+' := by decide_cbv',True)

    def test_exact_axiom_roster(self):
        names=re.findall(r'^theorem (\w+)',(ROOT/'VerumExternalReplay/CausalJournal.lean').read_text(),re.M)
        result=self.prove('\n'.join('#print axioms CausalJournal.'+n for n in names))
        rows=[d['data'] for d in result['diagnostics'] if d['severity']=='information']
        self.assertEqual(len(rows),len(names))
        for row in rows:self.assertTrue(row.endswith(('does not depend on any axioms',
            'depends on axioms: [propext]', 'depends on axioms: [propext, Quot.sound]')),row)

if __name__=='__main__':unittest.main()
