"""Check exhaustion semantics and the reducer's independently stated laws."""
import unittest
from lean_test_fixture import IsolatedLeanTest


class ReferenceReductionTests(IsolatedLeanTest):
    MODULES = ('ReductionScope', 'UniverseSemantics')

    def test_well_typed_terms_can_exhaust_without_becoming_invalid(self):
        self.prove('''
import VerumExternalReplay.ReductionScope
open VerumKernel
/-- info: true -/
#guard_msgs in
#eval! [0, 1, 999, 4294967293].all fun level =>
  let value := Term.universe (.concrete level)
  let domain := Term.universe (.concrete (level + 1))
  let term := Term.app (.lam domain (.var 0)) value
  let typed := match verifyCertificate term domain with
    | .ok () => true
    | _ => false
  let exhausted := [0, 1].all fun fuel => match whnfFuel fuel term with
    | .error .fuel_exhausted => true
    | _ => false
  let completed := match whnfFuel 2 term with
    | .ok (.universe (.concrete result)) => result == level
    | _ => false
  typed && exhausted && completed
''')

    def test_exhaustion_cannot_be_claimed_as_a_completed_reduction(self):
        self.prove('''
import VerumExternalReplay.ReductionScope
open VerumKernel
example : (whnfFuel 0 (.universe (.concrete 0))).isOk = true := by decide_cbv
''', failure=True)

    def test_exact_universe_successor_laws_and_no_wrap(self):
        rows = self.prove("""
import VerumExternalReplay.UniverseSemantics
open VerumKernel
#print axioms Level.successor_exact
#print axioms Level.successor_strict
""")
        self.assert_axioms(rows, ['VerumKernel.Level.successor_exact',
                                 'VerumKernel.Level.successor_strict'])
        self.prove("""
import VerumExternalReplay.UniverseSemantics
open VerumKernel
example : (Level.successor (.concrete Level.wireMax)).denote (fun _ => 0) = 0 := by decide_cbv
""", failure=True)

    def test_completed_mismatch_and_law_axioms(self):
        rows = self.prove('''
import VerumExternalReplay.ReductionScope
open VerumKernel
/-- info: true -/
#guard_msgs in
#eval! match defEq (.universe (.concrete 0)) (.universe (.concrete 1)) with
  | .ok false => true
  | _ => false
#print axioms whnfFuel_reduces
#print axioms HeadStep.preserves_scope
#print axioms scope_whnfFuel
''')
        self.assert_axioms(rows, ['VerumKernel.whnfFuel_reduces',
                                 'VerumKernel.HeadStep.preserves_scope',
                                 'VerumKernel.scope_whnfFuel'])


if __name__ == '__main__':
    unittest.main()
