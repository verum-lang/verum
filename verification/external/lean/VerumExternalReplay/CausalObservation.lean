import VerumExternalReplay.OutcomeTransitions
import VerumExternalReplay.PredictiveObservation
import VerumExternalReplay.ActionTargetWire

/-! Declared scalar callback semantics for outcome-aware native snapshots.
The complete wire supplies one extra external input; the original query stays
pre-outcome. This reuses the typed native evaluator and the completed-row
contract. Native callback purity, allocation/refusal and physical capture
authority remain correspondence premises, not consequences of this decoder.
-/
namespace CausalObservation
set_option warningAsError true

abbrev External := Nat × List Nat
structure Input where
  query : External
  event : External

def decode (target : List Nat) : Option Nat :=
  if target.length = 8 ∧ target.all (fun byte => byte < 256) then
    some (ActionTargetWire.readLittleEndian (target.map UInt8.ofNat))
  else none

def appendOutcome (event : External) (target : List Nat) : Option External :=
  OutcomeTransitions.appendOutcome decode event target

theorem append_exact (event : External) (target : List Nat) (completed : External) :
    appendOutcome event target = some completed ↔
    ∃ value, decode target = some value ∧ completed = (event.1,event.2++[value]) := by
  exact OutcomeTransitions.append_outcome_exact decode event target completed

theorem wrong_width_refused (event : External) (target : List Nat) (h : target.length ≠ 8) :
    appendOutcome event target = none := by
  simp [appendOutcome,OutcomeTransitions.appendOutcome,decode,h]

def update (code : FiniteCode.Code RetainedNative.Primitive Nat)
    (binding : ObserverAdmission.Binding Nat) (state : Nat) (event : External)
    (target : List Nat) : Option Nat := do
  let completed ← appendOutcome event target
  PredictiveObservation.evaluate code binding state completed.1 completed.2

def protocol (readCode updateCode : FiniteCode.Code RetainedNative.Primitive Nat)
    (readBinding updateBinding : ObserverAdmission.Binding Nat) (context : List Nat) :
    OutcomeTransitions.Protocol Nat PredictiveReplay.Bank Input PredictiveReplay.Prediction (List Nat) where
  predict state _ input :=
    (PredictiveObservation.evaluate readCode readBinding state input.query.1 input.query.2).map
      (fun reading => ⟨context,PredictiveObservation.wire 8 0 reading⟩)
  update state input target := update updateCode updateBinding state input.event target
  learn bank prediction target := some (PredictiveReplay.learn bank prediction target)

end CausalObservation
