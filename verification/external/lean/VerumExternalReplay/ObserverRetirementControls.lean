import VerumExternalReplay.ObserverRetirement

namespace ObserverRetirementControls
open ObserverRetirement
set_option warningAsError true

-- Synthetic controls. The physical record here identifies an already committed
-- event, and retained observer states differ from their prior language IDs.
def before : State Nat Nat Nat Nat :=
  ⟨7, 5, 40, [⟨10, 100⟩, ⟨20, 200⟩, ⟨30, 300⟩], 901, 8, [], 2⟩
def receipt : Receipt Nat := ⟨7, 5, 1, .undefined, 7⟩
def after : State Nat Nat Nat Nat :=
  ⟨7, 6, 20, [⟨10, 100⟩], 901, 7, [receipt], 2⟩
def second : Receipt Nat := ⟨7, 6, 0, .undefined, 6⟩
def finalState : State Nat Nat Nat Nat :=
  ⟨7, 7, 10, [], 901, 6, [receipt, second], 2⟩

def input (epoch original : Nat) : Option Nat :=
  if epoch = 20 then some (original + epoch) else none
def readNext (_ : Nat) (layers : List (Layer Nat Nat)) (physical : Nat) : Option Nat :=
  match layers with
  | [layer] => some (layer.observer + physical)
  | _ => none

theorem exact_retirement : retire before receipt = some after := by rfl
theorem exact_chain : replay before [receipt, second] = some finalState := by rfl
theorem conserved_commit : finalState.physical = before.physical := by rfl
theorem retained_current_state : after.layers = [⟨10, 100⟩] := by rfl
theorem exact_edge : edge after 5 input readNext = some (25, 1001) := by rfl
theorem unrecovered_language_has_no_edge : edge before 5 input readNext = none := by rfl
theorem resource_keeps_its_class : retire before { receipt with reason := .resource } = none := by rfl
theorem foreign_keeps_its_class : retire before { receipt with reason := .foreignSource } = none := by rfl
theorem no_repeat_retirement : retire after receipt = none := by rfl
theorem capacity_refusal : retire { before with capacity := 0 } receipt = none := by rfl
theorem cannot_refund_work : retire before { receipt with remaining := 8 } = none := by rfl
theorem invalid_layer_refusal : retire before { receipt with failed := 3 } = none := by rfl

end ObserverRetirementControls
