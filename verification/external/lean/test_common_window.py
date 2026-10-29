"""Common-support and positive-reference laws with strict kernel controls."""
from pathlib import Path
import re
import tempfile
import unittest
from check_typed_programs import ROOT, check, require_success, run

HEADER = '''import VerumExternalReplay.CommonWindow
open ExactMass CategoricalPrediction CommonWindow
set_option warningAsError true
def binary : Histogram := ⟨[0,0],by decide⟩
def singleton : Histogram := ⟨[0],by decide⟩
def alphabet : Alphabet Nat := ⟨[7,11],by decide,by decide⟩
def reversed : Alphabet Nat := ⟨[11,7],by decide,by decide⟩
def wider : Alphabet Nat := ⟨[7,11,13],by decide,by decide⟩
def oneCell : Partition alphabet Unit := ⟨fun _ => binary,by intro c; rfl,fun _ => ()⟩
def splitCells : Partition alphabet Bool :=
  ⟨fun _ => binary,by intro c; rfl,fun past => past.length % 2 == 1⟩
def trained : Partition alphabet Unit :=
  ⟨fun _ => ⟨[3,1],by decide⟩,by intro c; rfl,fun _ => ()⟩
def price (name look : Nat) (hn : 0 < name) (hl : 0 < look) : Price :=
  ⟨unit name hn,unit look hl,Nat.zero_lt_one,Nat.zero_lt_one⟩
def old : Assessment Nat Nat :=
  ⟨4,alphabet,unit 8 (by decide),unit 8 (by decide),price 4 5 (by decide) (by decide),by decide,by decide⟩
def new : Assessment Nat Nat :=
  ⟨4,alphabet,unit 4 (by decide),⟨2,16,by decide⟩,price 8 5 (by decide) (by decide),by decide,by decide⟩
def wrongWindow : Assessment Nat Nat := { new with window := 9 }
def wrongSupport : Assessment Nat Nat := { new with alphabet := reversed }
def wrongReference : Assessment Nat Nat :=
  { new with reference := unit 4 (by decide), referencePositive := by decide }
def noLookPrice : Assessment Nat Nat :=
  { new with price := price 8 1 (by decide) (by decide) }
def checkMass (p : Partition alphabet C) [DecidableEq C] (word : List Nat) (q : Fraction) : Bool :=
  match likelihood p [] word with | some actual => decide (Equal actual q) | none => false
def matched (result : Option Fraction) (expected : Fraction) : Bool :=
  match result with | some actual => decide (Equal actual expected) | none => false
def hugeCompression (a : Fraction) : Fraction := divide a (unit (2^80+1) (by decide)) (by decide)
'''
FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'


class CommonWindowTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake','build','VerumExternalReplay.CommonWindow'))

    def prove(self,source,failure=False):
        with tempfile.TemporaryDirectory(prefix='verum-common-window-') as directory:
            path=Path(directory)/'check.lean';path.write_text(HEADER+source)
            result=check(path,FALSE if failure else None)
        if failure:
            errors=[d for d in result['diagnostics'] if d['severity']=='error']
            self.assertEqual(len(errors),1,errors)
            self.assertTrue(errors[0]['data'].startswith(FALSE),errors)
        return result

    def test_same_support_different_causal_partitions(self):
        self.prove('''
example : checkMass oneCell [0,1] (unit 8 (by decide)) = true := by decide_cbv
example : checkMass splitCells [0,1] (unit 4 (by decide)) = true := by decide_cbv
example : checkMass trained [1,0] ⟨7,40,by decide⟩ = true := by decide_cbv
example : likelihood oneCell [] [2] = none := by decide_cbv
example : likelihood splitCells [] [0,2,1] = none := by decide_cbv
example : Equal (contextFutureMass oneCell.choose oneCell.counts [] 3) one := by decide_cbv
example : Equal (contextFutureMass splitCells.choose splitCells.counts [] 3) one := by decide_cbv
example (past labels : List Nat) :
    (likelihood oneCell past labels).isSome = (likelihood splitCells past labels).isSome :=
  shared_domain _ _ _ _
example (past : List Nat) (n : Nat) :
    Equal (contextFutureMass oneCell.choose oneCell.counts past n) one ∧
    Equal (contextFutureMass splitCells.choose splitCells.counts past n) one := both_normalized _ _ _ _
example (past : List Nat) (n : Nat) :
    let space := CategoricalPrediction.words 2 n
    Equal (sum (space.map (contextMass oneCell.choose oneCell.counts past))) one ∧
    Equal (sum (space.map (contextMass splitCells.choose splitCells.counts past))) one :=
  shared_words_normalized oneCell splitCells past n
example : Equal (sum ((CategoricalPrediction.words 2 3).map
    (contextMass splitCells.choose splitCells.counts []))) one := by decide_cbv
''')

    def test_reference_cancellation_and_original_price(self):
        self.prove('''
example : compatible old new = true := by decide_cbv
example : matched (CommonWindow.compare old new) one = true := by decide_cbv
example : Equal (ratio old noLookPrice) ⟨5,1,by decide⟩ := by decide_cbv
example : CommonWindow.compare old wrongWindow = none := by decide_cbv
example : CommonWindow.compare old wrongSupport = none := by decide_cbv
example : CommonWindow.compare old wrongReference = none := by decide_cbv
example : Equal (ratio old wrongReference) (unit 2 (by decide)) := by decide_cbv
example : Equal (divide (hugeCompression (unit 3 (by decide)))
    (hugeCompression (unit 7 (by decide))) (by decide)) ⟨7,3,by decide⟩ := by decide_cbv
example : Equal (add (predict singleton ⟨0,by decide⟩) (predict singleton ⟨0,by decide⟩))
    ⟨2,1,by decide⟩ := by decide_cbv
''')

    def test_invalid_comparisons_and_shortcuts_are_rejected(self):
        claims={
            'foreign_window':'(CommonWindow.compare old wrongWindow).isSome = true',
            'renamed_alphabet':'(CommonWindow.compare old wrongSupport).isSome = true',
            'expanded_support':'(CommonWindow.compare old { new with alphabet := wider }).isSome = true',
            'different_reference':'(CommonWindow.compare old wrongReference).isSome = true',
            'drop_name_price':'Equal (ratio old new) ⟨2,1,by decide⟩',
            'reset_paid_look':'Equal (ratio old noLookPrice) (ratio old new)',
            'cancel_different_parent':'Equal (ratio old wrongReference) (ratio old new)',
            'join_partitions':'checkMass oneCell [0,1] (unit 4 (by decide)) = true',
            'reset_training':'checkMass trained [1,0] (unit 8 (by decide)) = true',
            'discard_later_foreign_label':'(likelihood splitCells [] [0,2,1]).isSome = true',
            'predict_after_observing':'checkMass oneCell [0] ⟨3,4,by decide⟩ = true',
            'zero_reference_is_positive':'0 < zero.numerator',
            'outcome_selected_support':'Equal (add (predict singleton ⟨0,by decide⟩) (predict singleton ⟨0,by decide⟩)) one',
            'drop_unobserved_words':'Equal (sum ([[0,0],[1,1]].map (contextMass oneCell.choose oneCell.counts []))) one',
        }
        for name,claim in claims.items():
            with self.subTest(control=name):self.prove(f'example : {claim} := by decide_cbv\n',True)

    def test_constructive_axiom_roster(self):
        names=[]
        for module in ['ExactMass','CategoricalPrediction','CommonWindow']:
            source=(ROOT/'VerumExternalReplay'/(module+'.lean')).read_text()
            names += [module+'.'+n for n in re.findall(r'^theorem ([\w.]+)',source,re.M)]
        names += ['CategoricalPrediction.contextPath','CommonWindow.compatible','CommonWindow.ratio']
        result=self.prove('\n'.join('#print axioms '+n for n in names)+'\n')
        rows=[d['data'] for d in result['diagnostics'] if d['severity']=='information']
        self.assertEqual(len(rows),len(names))
        for row in rows:
            if 'depends on axioms: [' in row:
                self.assertLessEqual(set(row.split('depends on axioms: [')[1].removesuffix(']').split(', ')),
                    {'propext','Quot.sound'},row)
            else:self.assertTrue(row.endswith('does not depend on any axioms'),row)


if __name__=='__main__':unittest.main()
