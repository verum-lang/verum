"""Resource transfer laws, exact foundations and semantic counterexamples."""
from pathlib import Path
import re
import tempfile
import unittest
from check_typed_programs import ROOT, check, require_success, run

HEADER='import VerumExternalReplay.ResourceTransfer\nopen ResourceTransfer\nset_option warningAsError true\n'


class ResourceTransferTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake','build','VerumExternalReplay.ResourceTransfer'))

    def test_universal_foundations_and_actual_composition_constructor(self):
        names=re.findall(r'^theorem (\w+)', (ROOT/'VerumExternalReplay/ResourceTransfer.lean').read_text(),re.M)
        self.assertEqual(len(names),15)
        with tempfile.TemporaryDirectory(prefix='verum-resource-laws-') as directory:
            path=Path(directory)/'check.lean'
            path.write_text(HEADER+'\n'.join('#print axioms ResourceTransfer.'+name for name in names)+'''
 def lang : Language Nat Nat Nat := ⟨fun p x => some (p+x), fun p x => totalCost (p+x) 1 2 3⟩
 def unchanged : Transfer lang lang where
   translate := id
   input := id
   output := id
   budget := id
   monotone := fun _ _ h => h
   exact := fun _ _ => rfl
   bounded := fun _ _ => Nat.le_refl _
 example : (compose unchanged unchanged).budget 12 = 12 := rfl
 example : (compose unchanged unchanged).translate 7 = 7 := rfl
 example : lang.cost 2 3 = 11 := rfl
 example : totalCost 0 1 0 0 > (fun _ : Nat => 0) ((fun _ : Nat => 1) 0) := by decide
 example : (1 : Nat) ≤ (fun _ : Nat => 1) 0 ∧ (0 : Nat) ≤ (fun _ : Nat => 0) 0 := by decide
 example : ∀ m n : Nat, m ≤ n → (fun _ : Nat => 0) m ≤ (fun _ : Nat => 0) n := by intros; exact Nat.le_refl _
''')
            result=check(path)
        roster=[d['data'] for d in result['diagnostics'] if d['severity']=='information']
        self.assertEqual(len(roster),len(names))
        for row in roster:
            allowed = ('depends on axioms: [Quot.sound]',) if 'fixed_aggregate_risk_transport' in row else ('does not depend on any axioms','depends on axioms: [propext]')
            self.assertTrue(row.endswith(allowed),row)

    def test_false_claims_reach_semantic_rejection(self):
        statements={
            'translation_overhead_erased':'totalCost 1 1 0 0 ≤ 1',
            'nonmonotone_budget_assumed':'(if (0 : Nat) = 0 then 1 else 0) ≤ (if (1 : Nat) = 0 then 1 else 0)',
            'paid_component_erased':'advancePaid 0 (fun _ => 0) (advancePaid 1 (fun _ => 1) (0, 0)) = (0, 0)',
            'incurred_overhead_discounted':'totalCost 0 1 0 0 ≤ (fun _ : Nat => 0) ((fun _ : Nat => 1) 0)',
            'semantic_none_replaced':'(none : Option Bool).map id = some true',
            'noninjective_value_reflection':'(some false : Option Bool).map (fun _ => ()) = (some true).map (fun _ => ()) → (some false : Option Bool) = some true',
        }
        with tempfile.TemporaryDirectory(prefix='verum-resource-controls-') as directory:
            for name,statement in statements.items():
                with self.subTest(name=name):
                    path=Path(directory)/(name+'.lean'); path.write_text(HEADER+f'example : {statement} := by decide\n')
                    result=check(path,'Tactic `decide` proved that the proposition')
                    self.assertEqual(sum(d['severity']=='error' for d in result['diagnostics']),1)


if __name__=='__main__': unittest.main()
