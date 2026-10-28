"""Atomic paired publication and causal retained-tail replay, with false controls."""
from pathlib import Path
import re
import tempfile
import unittest

from check_typed_programs import ROOT, check, require_success, run

HEADER = '''import VerumExternalReplay.AtomicPublication
import VerumExternalReplay.PredictiveReplay
set_option warningAsError true

def owner : AtomicPublication.Frontier Nat Nat Nat Nat := ⟨⟨11,21,31⟩,41⟩
def update : AtomicPublication.Update Nat Nat Nat Nat (List Nat) :=
  ⟨owner.cut, ⟨owner.cut,32⟩,41,42,[9],rfl⟩
def readVM (v q : Nat) : Option PredictiveReplay.Prediction := some ⟨[q],[v]⟩
def updateVM (v e : Nat) : Option Nat := some (v+e)
def start : PredictiveReplay.State Nat := ⟨1,PredictiveReplay.initial⟩
def first : PredictiveReplay.Record Nat Nat := ⟨8,1,[]⟩
def second : PredictiveReplay.Record Nat Nat := ⟨8,1,[2]⟩
def learned := (PredictiveReplay.replay readVM updateVM [first,second] start).getD start
'''
FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'


class PredictivePublicationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake','build','VerumExternalReplay.AtomicPublication',
                            'VerumExternalReplay.PredictiveReplay'))

    def prove(self, source, expected=None):
        with tempfile.TemporaryDirectory(prefix='verum-predictive-publication-') as directory:
            path = Path(directory)/'check.lean'; path.write_text(HEADER + source)
            result = check(path, expected)
            if expected:
                errors = [d for d in result['diagnostics'] if d['severity']=='error']
                self.assertEqual(len(errors),1,errors)
                self.assertTrue(errors[0]['data'].startswith(expected),errors)
            return result

    def test_axiom_roster(self):
        names = []
        for module in ('AtomicPublication','PredictiveReplay'):
            names += [module+'.'+name for name in re.findall(r'^theorem (\w+)',
                (ROOT/f'VerumExternalReplay/{module}.lean').read_text(),re.M)]
        result = self.prove('\n'.join('#print axioms '+name for name in names))
        rows = [d['data'] for d in result['diagnostics'] if d['severity']=='information']
        self.assertEqual(len(rows),len(names))
        for row in rows:
            self.assertTrue(row.endswith(('does not depend on any axioms',
                'depends on axioms: [propext]', 'depends on axioms: [propext, Quot.sound]')),row)

    def test_publication_is_paired_and_rejection_retains_state(self):
        self.prove('''
open AtomicPublication
example : admissible owner update = true := by decide_cbv
example : (commit owner update).current = ⟨⟨11,21,32⟩,42⟩ := by decide_cbv
example : (commit (fork owner 22) update).current = fork owner 22 := by decide_cbv
example : admissible (commit owner update).current update = false := by decide_cbv
example : (match (commit owner update).receipt with | .ok r => r.target | .error _ => []) = [9] := by decide_cbv
example : (commit {owner with bank:=43} update).current = {owner with bank:=43} := by decide_cbv
example : (commit (fork owner 22) update).receipt = .error update :=
  congrArg Result.receipt (sibling_refused owner update 22 rfl (by decide))
example : ObserverOwnership.commit owner.cut update.prepared =
    ⟨(published owner update).cut, .ok ⟨update.prepared.before,(published owner update).cut⟩⟩ :=
  underlying_commit_succeeds owner update (by decide)
''')

    def test_causal_whole_row_tail_transport(self):
        self.prove('''
open PredictiveReplay
example : learned.vm = 3 := by decide_cbv
example : learned.bank.body.samples = 2 := by decide_cbv
example : learned.bank.body.bits = 8 := by decide_cbv
example : AdaptiveLength.countAt 0 learned.bank.lengths = 1 := by decide_cbv
example : AdaptiveLength.countAt 1 learned.bank.lengths = 1 := by decide_cbv
example : Coherent learned.bank := by decide_cbv
example : learned.bank = learn (learn initial ⟨[8],[1]⟩ []) ⟨[8],[2]⟩ [2] := by decide_cbv
example : replay readVM updateVM [first,second] start =
    (replay readVM updateVM [first] start).bind (replay readVM updateVM [second]) :=
  replay_append readVM updateVM [first] [second] start
example : step (fun _ _ => none) updateVM start first = none := by decide_cbv
example : step readVM (fun _ _ => none) start first = none := by decide_cbv
''')

    def test_forged_guards_and_transport_are_rejected(self):
        claims = [
            'AtomicPublication.admissible {owner with bank:=43} update = true',
            'AtomicPublication.admissible {owner with cut:=⟨12,21,31⟩} update = true',
            'AtomicPublication.admissible {owner with cut:=⟨11,22,31⟩} update = true',
            'AtomicPublication.admissible {owner with cut:=⟨11,21,32⟩} update = true',
            'AtomicPublication.admissible (AtomicPublication.commit owner update).current update = true',
            '(AtomicPublication.commit owner update).current.bank = owner.bank',
            '(AtomicPublication.commit owner update).current.cut = owner.cut',
            'learned.bank.body.samples = 1',
            'learned.bank = PredictiveReplay.initial',
            'learned.bank = PredictiveReplay.learn (PredictiveReplay.learn PredictiveReplay.initial ⟨[8],[2]⟩ []) ⟨[8],[3]⟩ [2]',
        ]
        for claim in claims:
            with self.subTest(claim=claim):
                self.prove(f'example : {claim} := by decide_cbv', FALSE)
        self.prove('''
def forged : AtomicPublication.Update Nat Nat Nat Nat (List Nat) :=
  ⟨owner.cut, ⟨⟨11,22,31⟩,32⟩,41,42,[9],by decide_cbv⟩
''', FALSE)


if __name__ == '__main__': unittest.main()
