import VerumExternalReplay.CheckedPairs
import VerumExternalReplay.ObserverAdmission

namespace FiniteExecutionControls
open CheckedPairs FiniteExecution ObserverAdmission
set_option warningAsError true

def add (a b : UInt64) : Code := .mk
  [⟨.primitive .checkedAdd, [.literal (.nat a), .literal (.nat b)], none⟩] none []
def lexical : Code := .mk [⟨.call 0, [], none⟩] none
  [.mk [⟨.call 0, [], none⟩] none [add 2 3], add 40 2]
def precedence : Code := .mk [⟨.call 9, [.input 7], none⟩] none []
def hole : Code := .mk [⟨.primitive .checkedAdd, [.literal (.nat 1)], some 2⟩] none []
def noCalleeOutput : Code := .mk [⟨.call 0, [], none⟩] none [.mk [] none []]

theorem lexical_receipt : execute apply lexical ⟨some 8, 1000⟩ [] =
    ⟨.complete, [.nat 5], ⟨13, 3, 1, 2⟩, none, none⟩ := by
  unfold lexical add; reduce_execution
theorem argument_before_callee : execute apply precedence ⟨none, 1000⟩ [] =
    ⟨.undefined, [], ⟨6, 1, 0, 1⟩, some ⟨[0], .missingInput 0 7⟩, none⟩ := by
  unfold precedence; reduce_execution
theorem declared_operand_hole : execute apply hole ⟨none, 1000⟩ [] =
    ⟨.undefined, [], ⟨5, 1, 1, 0⟩, some ⟨[0], .missingOperand 1⟩, none⟩ := by
  unfold hole; reduce_execution
theorem child_output_position : execute apply noCalleeOutput ⟨none, 1000⟩ [] =
    ⟨.undefined, [], ⟨5, 1, 0, 1⟩, some ⟨[0, 0], .missingCalleeOutput⟩, none⟩ := by
  unfold noCalleeOutput; reduce_execution
theorem semantic_refusal : execute apply (add 18446744073709551615 1) ⟨some 1, 1000⟩ [] =
    ⟨.undefined, [], ⟨5, 1, 1, 0⟩, some ⟨[0], .primitive .overflow⟩, none⟩ := by
  unfold add; reduce_execution
theorem dispatch_refusal : execute apply (add 18446744073709551615 1) ⟨some 0, 1000⟩ [] =
    ⟨.budget, [], ⟨1, 0, 0, 0⟩, some ⟨[0], .allowance⟩, none⟩ := by
  unfold add; reduce_execution
theorem work_refusal : execute apply (add 18446744073709551615 1) ⟨none, 0⟩ [] =
    ⟨.budget, [], ⟨0, 0, 0, 0⟩, some ⟨[0], .allowance⟩, some ⟨0, 0, 1⟩⟩ := by
  unfold add; reduce_execution

def pack : Code := .mk [⟨.primitive .packByte, [.input 0, .input 1], none⟩] none []
def binding : Binding Nat := ⟨7, 2, [.external 0, .external 1]⟩
def allowance : Budget := ⟨some 1, 1000, 0, 0, false⟩
def firstRead := evaluate apply .read pack binding (.nat 0) 7 [.nat 1, .nat 2] allowance
def secondUpdate := evaluate apply .update pack binding (.nat 0) 7 [.nat 1, .nat 2] firstRead.budget

macro "reduce_observer" : tactic => `(tactic|
  reduce_admission)

theorem read_result : firstRead.result = .ok (.nat 258,
    ⟨.complete, [.nat 258], ⟨10, 1, 1, 0⟩, none, none⟩) := by
  unfold firstRead pack binding allowance; reduce_observer
theorem shared_dispatch_allowance : secondUpdate.result = .error (.execution .update
    ⟨.budget, [], ⟨17, 0, 0, 0⟩, some ⟨[0], .allowance⟩, none⟩ none false) := by
  unfold secondUpdate firstRead pack binding allowance; reduce_observer
theorem cumulative_accounting : secondUpdate.budget = ⟨some 1, 1000, 17, 1, false⟩ := by
  unfold secondUpdate firstRead pack binding allowance; reduce_observer
theorem root_work_is_separate :
    (evaluate apply .read pack binding (.nat 0) 7 [.nat 1, .nat 2] ⟨none, 10, 0, 0, false⟩).result =
      .error (.work .read .rootOutput ⟨10, 10, 1⟩ (some
        ⟨.complete, [.nat 258], ⟨10, 1, 1, 0⟩, none, none⟩)) := by
  unfold pack binding; reduce_observer
theorem interface_refusal :
    (evaluate apply .read pack binding (.nat 0) 8 [.nat 1, .nat 2] allowance).result =
      .error (.binding .read .interface) := by
  unfold pack binding allowance; reduce_observer
theorem work_before_interface :
    (evaluate apply .read pack binding (.nat 0) 8 [.nat 1, .nat 2] ⟨none, 0, 0, 0, false⟩).result =
      .error (.work .read .inputBinding ⟨0, 0, 5⟩ none) := by
  unfold pack binding; reduce_observer
theorem invalid_slot : validateSlots 2 [.external 2] 0 = .error (.invalidSlot 0 2 2) := rfl

/-- info: 'FiniteExecutionControls.cumulative_accounting' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms cumulative_accounting
end FiniteExecutionControls
