import VerumExternalReplay.ReadEvidence
import VerumExternalReplay.FiniteExecutionControls

namespace ReadEvidenceControls
open CheckedPairs FiniteExecution ReadEvidence
set_option warningAsError true

def enough : Limits := ⟨some 20, 1000⟩
def copyInput : Code := .mk
  [⟨.primitive .checkedAdd, [.input 0, .literal (.nat 0)], none⟩] none []
def nested : Code := .mk [⟨.call 0, [.input 1], none⟩] none [copyInput]
def constant : Code := .mk
  [⟨.primitive .equal, [.literal (.nat 1), .literal (.nat 1)], none⟩] none []
def unusedRegister : Code := .mk
  [⟨.primitive .checkedAdd, [.input 0, .literal (.nat 0)], none⟩,
   ⟨.primitive .equal, [.literal (.nat 1), .literal (.nat 1)], none⟩] none []
def unusedFailure : Code := .mk
  [⟨.primitive .rem, [.literal (.nat 1), .input 0], none⟩,
   ⟨.primitive .equal, [.literal (.nat 1), .literal (.nat 1)], none⟩] none []
def eagerCall : Code := .mk [⟨.call 0, [.input 1], none⟩] none [constant]

-- Universal across values, limits, starting work and semantic failures.
theorem nested_unused_input (left right value : Value) (limits : Limits) (used : Nat) :
    execute apply nested limits [left, value] used =
      execute apply nested limits [right, value] used := by
  apply execute_agreement
  rfl

theorem unused_registers_are_observable :
    selectRoot unusedRegister (execute apply unusedRegister enough [.nat 1]) =
      selectRoot unusedRegister (execute apply unusedRegister enough [.nat 2]) ∧
    execute apply unusedRegister enough [.nat 1] ≠
      execute apply unusedRegister enough [.nat 2] := by decide_cbv

theorem unused_failure_is_observable :
    (execute apply unusedFailure enough [.nat 0]).stop = .undefined ∧
    (execute apply unusedFailure enough [.nat 1]).stop = .complete := by decide_cbv

theorem eager_callee_input_is_observable :
    (execute apply eagerCall enough [.nat 0]).halt = some ⟨[0], .missingInput 0 1⟩ ∧
    (execute apply eagerCall enough [.nat 0, .nat 1]).stop = .complete := by decide_cbv

theorem missing_is_not_zero : key copyInput [] ≠ key copyInput [.nat 0] := by decide_cbv

theorem limits_are_observable :
    (execute apply copyInput enough [.nat 0]).stop = .complete ∧
    (execute apply copyInput ⟨some 0, 1000⟩ [.nat 0]).stop = .budget := by decide_cbv

theorem initial_work_is_observable :
    (execute apply copyInput enough [.nat 0] 0).observation =
      (execute apply copyInput enough [.nat 0] 1).observation ∧
    execute apply copyInput enough [.nat 0] 0 ≠
      execute apply copyInput enough [.nat 0] 1 := by decide_cbv

-- Frozen code is not enough when primitive semantics change.
def changedApply (_ : Op) (_ : List Value) : Except Refusal Value := .ok (.nat 9)
theorem semantics_are_observable :
    execute apply copyInput enough [.nat 0] ≠
      execute changedApply copyInput enough [.nat 0] := by
  simp only [execute, copyInput, frame, List.attach_nil, List.map_nil]
  decide_cbv

-- Reuse existing admission controls: full body completion is not root-read
-- success, and equal effective input values do not authorize an interface.
-- `FiniteExecutionControls.interface_refusal` and
-- `FiniteExecutionControls.root_work_is_separate` are imported above.

-- A report extension that retains the whole environment invalidates the
-- footprint-only contract. Native observed invocation traces do retain it.
theorem captured_inputs_refuse_projection :
    key constant [.nat 1] = key constant [.nat 2] ∧
    ([Value.nat 1], execute apply constant enough [.nat 1]) ≠
      ([Value.nat 2], execute apply constant enough [.nat 2]) := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : execute apply unusedRegister enough [.nat 1] =
    execute apply unusedRegister enough [.nat 2] := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : execute apply unusedFailure enough [.nat 0] =
    execute apply unusedFailure enough [.nat 1] := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : execute apply eagerCall enough [.nat 0] =
    execute apply eagerCall enough [.nat 0, .nat 1] := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : execute apply copyInput enough [.nat 0] =
    execute apply copyInput ⟨some 0, 1000⟩ [.nat 0] := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : execute apply copyInput enough [.nat 0] 0 =
    execute apply copyInput enough [.nat 0] 1 := by decide_cbv

-- Pin the foundation of every named law; no admitted or native oracle proofs.
/-- info: 'ReadEvidence.operand_agreement' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ReadEvidence.operand_agreement

/-- info: 'ReadEvidence.arguments_agreement' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ReadEvidence.arguments_agreement

/-- info: 'ReadEvidence.steps_agreement' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ReadEvidence.steps_agreement

/-- info: 'ReadEvidence.key_agreement' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ReadEvidence.key_agreement

/-- info: 'ReadEvidence.frame_agreement' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ReadEvidence.frame_agreement

/-- info: 'ReadEvidence.execute_agreement' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ReadEvidence.execute_agreement

/-- info: 'ReadEvidence.selected_agreement' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ReadEvidence.selected_agreement

/-- info: 'ReadEvidenceControls.nested_unused_input' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ReadEvidenceControls.nested_unused_input

/-- info: 'ReadEvidenceControls.unused_registers_are_observable' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ReadEvidenceControls.unused_registers_are_observable

/-- info: 'ReadEvidenceControls.unused_failure_is_observable' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ReadEvidenceControls.unused_failure_is_observable

/-- info: 'ReadEvidenceControls.eager_callee_input_is_observable' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ReadEvidenceControls.eager_callee_input_is_observable

/-- info: 'ReadEvidenceControls.missing_is_not_zero' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ReadEvidenceControls.missing_is_not_zero

/-- info: 'ReadEvidenceControls.limits_are_observable' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ReadEvidenceControls.limits_are_observable

/-- info: 'ReadEvidenceControls.initial_work_is_observable' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ReadEvidenceControls.initial_work_is_observable

/-- info: 'ReadEvidenceControls.semantics_are_observable' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ReadEvidenceControls.semantics_are_observable

/-- info: 'ReadEvidenceControls.captured_inputs_refuse_projection' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ReadEvidenceControls.captured_inputs_refuse_projection

end ReadEvidenceControls
