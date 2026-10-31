"""Successful value reuse without merging observer domains or capabilities."""
import re
import unittest
from lean_test_fixture import IsolatedLeanTest

HEADER = '''import VerumExternalReplay.ReadReuse
import VerumExternalReplay.ReadEvidenceControls
open CheckedPairs FiniteExecution ObserverAdmission ReadEvidence ReadReuse
set_option warningAsError true
def allowance : Limits := ⟨some 20, 1000⟩
def queryRead : Code := .mk
  [⟨.primitive .checkedAdd, [.input 1, .literal (.nat 0)], none⟩] none []
def queryBinding : Binding Nat := ⟨7, 1, [.state, .external 0]⟩
def goodUpdate : Code := .mk
  [⟨.primitive .checkedAdd, [.input 0, .literal (.nat 1)], none⟩] none []
def lateFailure : Code := .mk
  [⟨.primitive .checkedAdd, [.input 0, .literal (.nat 1)], none⟩,
   ⟨.primitive .rem, [.literal (.nat 1), .input 0], none⟩] (some 0) []
def pureApply := FiniteExecutionProjection.partialApply apply
def firstCut : ObserverOwnership.Cut Nat Nat Nat := ⟨1, 10, 0⟩
def secondCut : ObserverOwnership.Cut Nat Nat Nat := ⟨2, 20, 0⟩
def updateFirst : ObserverOwnership.Cut Nat Nat Value := ⟨1, 10, .nat 0⟩
def updateSecond : ObserverOwnership.Cut Nat Nat Value := ⟨2, 20, .nat 0⟩
def prepareUpdate (cut : ObserverOwnership.Cut Nat Nat Value) (code : Code) :=
  ObserverOwnership.prepare cut
    ((runBody apply .update code [cut.state] ⟨some 20, 1000, 0, 0, false⟩).result.map Prod.fst)
'''


class ReadReuseTests(IsolatedLeanTest):
    MODULES = ('ReadReuse', 'ReadEvidenceControls')

    def test_constructive_axiom_roster(self):
        names = [namespace + name for module, namespace in
                 [('ReadEvidence', 'ReadEvidence.'), ('ReadReuse', 'ReadReuse.')]
                 for name in re.findall(r'^theorem (\w+)', self.sources[module], re.M)]
        names += ['ReadReuse.excludesState']
        self.assertEqual(len(names), 14)
        self.assert_axioms(self.prove(HEADER + '\n'.join('#print axioms ' + n for n in names)), names)

    def test_binding_certificate_and_successful_value_reuse(self):
        self.prove(HEADER + '''
example : excludesState queryRead queryBinding.slots = true := by rfl
example : excludesState ReadEvidenceControls.nested queryBinding.slots = true := by rfl
example : excludesState ReadEvidenceControls.unusedFailure queryBinding.slots = false := by rfl
example : excludesState lateFailure queryBinding.slots = false := by rfl
example : excludesState queryRead [.external 0, .state] = false := by rfl
example (state : Value) :
    FiniteCode.run pureApply queryRead [state, .nat 42] = some (.nat 42) := by
  apply state_excluded_reuse apply queryRead queryBinding (.nat 0) state
    [.nat 42] [.nat 0, .nat 42] [state, .nat 42] (by rfl) (by rfl) (by rfl)
    allowance 0 (.nat 42)
  · decide_cbv
  · decide_cbv
example (left right : Value) :
    key queryRead [left, .nat 42] = key queryRead [right, .nat 42] :=
  state_key_agreement queryRead queryBinding left right [.nat 42]
    [left, .nat 42] [right, .nat 42] (by rfl) (by rfl) (by rfl)
example : bindValues 1 (Value.nat 0) [] [.state, .external 0] 0 =
    .error (.invalidSlot 1 0 1) := by rfl
''')

    def test_unused_eager_work_changes_definedness(self):
        self.prove(HEADER + '''
example : (execute apply lateFailure allowance [.nat 0]).registers = [.nat 1] := by decide_cbv
example : (execute apply lateFailure allowance [.nat 0]).stop = .undefined := by decide_cbv
example : (execute apply lateFailure allowance [.nat 1]).stop = .complete := by decide_cbv
example : FiniteCode.run pureApply lateFailure [.nat 0] = none := by decide_cbv
example : key lateFailure [.nat 0] ≠ key lateFailure [.nat 1] := by decide_cbv
example : excludesState ReadEvidenceControls.eagerCall [.external 0, .state] = false := by rfl
''')

    def test_update_failure_and_read_capabilities_stay_local(self):
        self.prove(HEADER + '''
example : FiniteCode.run pureApply queryRead [.nat 0, .nat 42] = some (.nat 42) ∧
    FiniteCode.run pureApply queryRead [.nat 9, .nat 42] = some (.nat 42) ∧
    FiniteCode.run pureApply lateFailure [.nat 0] = none ∧
    FiniteCode.run pureApply goodUpdate [.nat 0] = some (.nat 1) := by decide_cbv
example : ObserverOwnership.read (E := Nat) firstCut (.ok (42 : Nat)) ≠
    ObserverOwnership.read secondCut (.ok 42) :=
  reading_cuts_distinct firstCut secondCut 42 (by decide)
example : ObserverOwnership.prepare (E := Nat) firstCut (.error 17) = .error 17 ∧
    ObserverOwnership.prepare (E := Nat) secondCut (.ok 1) = .ok ⟨secondCut, 1⟩ ∧
    (ObserverOwnership.commit secondCut ⟨secondCut, 1⟩).current = ⟨2, 20, 1⟩ := ⟨rfl, rfl, rfl⟩
example : (ObserverOwnership.commit secondCut ⟨firstCut, 1⟩).current = secondCut := by decide_cbv
example : (prepareUpdate updateFirst lateFailure).toOption = none ∧
    (prepareUpdate updateSecond goodUpdate).toOption = some ⟨updateSecond, .nat 1⟩ := by decide_cbv
example : ((prepareUpdate updateSecond goodUpdate).toOption.map
    (fun prepared => (ObserverOwnership.commit updateSecond prepared).current)) =
      some ⟨2, 20, .nat 1⟩ := by decide_cbv
''')

    def test_resource_and_interface_admission_remain_independent(self):
        self.prove(HEADER + '''
example : (execute apply queryRead ⟨some 0, 1000⟩ [.nat 0, .nat 42]).stop = .budget ∧
    FiniteCode.run pureApply queryRead [.nat 0, .nat 42] = some (.nat 42) := by decide_cbv
example : (evaluate apply .read queryRead queryBinding (.nat 0) 8 [.nat 42]
    ⟨some 20, 1000, 0, 0, false⟩).result = .error (.binding .read .interface) := by rfl
example : (evaluate apply .read queryRead queryBinding (.nat 0) 7 []
    ⟨some 20, 1000, 0, 0, false⟩).result = .error (.binding .read (.width 1 0)) := by rfl
example : selectRoot queryRead (execute apply queryRead ⟨some 0, 1000⟩ [.nat 0, .nat 42]) =
    .unattempted := by decide_cbv
''')

    def test_false_reuse_and_ownership_claims_rejected(self):
        claims = {
            'unused_eager_state_is_independent': 'excludesState lateFailure queryBinding.slots = true',
            'selected_value_implies_completion': 'selectRoot lateFailure (execute apply lateFailure allowance [.nat 0]) = .selected (.nat 1)',
            'read_equality_merges_updates': 'FiniteCode.run pureApply lateFailure [.nat 0] = FiniteCode.run pureApply goodUpdate [.nat 0]',
            'failed_neighbor_poisons_update': '(prepareUpdate updateSecond goodUpdate).toOption = none',
            'equal_values_transfer_reading': 'ObserverOwnership.read (E := Nat) firstCut (.ok (42 : Nat)) = ObserverOwnership.read secondCut (.ok 42)',
            'foreign_update_commits': '(ObserverOwnership.commit secondCut ⟨firstCut, 1⟩).current.state = 1',
            'resource_refusal_is_semantic_failure': 'FiniteCode.run pureApply queryRead [.nat 0, .nat 42] = none',
            'changed_semantics_reuses_value': 'selectRoot queryRead (execute ReadEvidenceControls.changedApply queryRead allowance [.nat 0, .nat 42]) = .selected (.nat 42)',
            'changed_binding_reuses_value': 'FiniteCode.run pureApply queryRead [.nat 42, .nat 0] = some (.nat 42)',
            'changed_relevant_input_reuses_value': 'FiniteCode.run pureApply queryRead [.nat 0, .nat 43] = some (.nat 42)',
            'missing_input_is_zero': 'key queryRead [.nat 0] = key queryRead [.nat 0, .nat 0]',
            'eager_callee_state_can_be_ignored': 'excludesState ReadEvidenceControls.eagerCall [.external 0, .state] = true',
        }
        for name, claim in claims.items():
            with self.subTest(claim=name):
                tactic = 'decide_cbv'
                if name == 'equal_values_transfer_reading':
                    tactic = 'simp only [ObserverOwnership.read, Except.map, Except.ok.injEq]; decide_cbv'
                self.prove(HEADER + 'example : ' + claim + ' := by ' + tactic, failure=True)


if __name__ == '__main__':
    unittest.main()
