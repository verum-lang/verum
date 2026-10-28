import VerumExternalReplay.OutcomeTransitions
import VerumExternalReplay.AcquiredObserver

/-! Outcome-aware execution over the full acquired value vocabulary. The
prediction decoder remains an explicit callback premise. Native allocation,
pointer identity and retained VM service costs require separate evidence. -/
namespace AcquiredCausalObservation
open AcquiredObserver
set_option warningAsError true

abbrev External := Source × List Value
structure Input where
  query : External
  event : External

/-- Byte values retain the entire wire, including headers and empty payloads.
Out-of-range diagnostic bytes are refused before conversion to UInt8. -/
def decode (target : List Nat) : Option Value :=
  if target.all (fun byte => byte < 256) then
    some (.bytes (target.map UInt8.ofNat))
  else none

theorem decode_roundtrip (bytes : List UInt8) :
    decode (bytes.map UInt8.toNat) = some (.bytes bytes) := by
  have valid : (bytes.map UInt8.toNat).all (fun byte => byte < 256) = true := by
    rw [List.all_map,List.all_eq_true]
    intro byte _
    exact decide_eq_true (UInt8.toNat_lt byte)
  simp [decode,valid,List.map_map,Function.comp_def]

theorem decode_exact (target : List Nat) (bytes : List UInt8)
    (success : decode target = some (.bytes bytes)) : bytes.map UInt8.toNat = target := by
  unfold decode at success
  split at success
  · rename_i valid
    have eq := Value.bytes.inj (Option.some.inj success)
    rw [←eq,List.map_map]
    conv => rhs; rw [←List.map_id target]
    apply List.map_congr_left
    intro n hn
    have bound : n < 256 := of_decide_eq_true ((List.all_eq_true.mp valid) n hn)
    simp [Nat.mod_eq_of_lt bound]
  · cases success

theorem decode_injective (left right : List Nat) (bytes : List UInt8)
    (a : decode left = some (.bytes bytes)) (b : decode right = some (.bytes bytes)) :
    left = right := (decode_exact left bytes a).symm.trans (decode_exact right bytes b)

def appendOutcome (event : External) (target : List Nat) : Option External :=
  OutcomeTransitions.appendOutcome decode event target

theorem completion_retains_input (event : External) (target : List Nat)
    (completed : External) (success : appendOutcome event target = some completed) :
    completed.1 = event.1 ∧ completed.2.take event.2.length = event.2 ∧
      completed.2.length = event.2.length+1 :=
  OutcomeTransitions.append_outcome_retains_input decode event target completed success

def update (maxTrace : Nat) (code : FiniteCode.Code Op Value)
    (binding : ObserverAdmission.Binding Source) (state : Value) (event : External)
    (target : List Nat) : Option Value := do
  let completed ← appendOutcome event target
  AcquiredObserver.run maxTrace code binding state completed.1 completed.2

def protocol (maxTrace : Nat) (readCode updateCode : FiniteCode.Code Op Value)
    (readBinding updateBinding : ObserverAdmission.Binding Source)
    (prediction : Value → Option PredictiveReplay.Prediction) :
    OutcomeTransitions.Protocol Value PredictiveReplay.Bank Input PredictiveReplay.Prediction (List Nat) where
  predict state _ input :=
    (AcquiredObserver.run maxTrace readCode readBinding state input.query.1 input.query.2).bind prediction
  update state input target := update maxTrace updateCode updateBinding state input.event target
  learn bank forecast target := some (PredictiveReplay.learn bank forecast target)

/-- The native receiver derives a query from every field of the pre-event
record. Outcome bytes are appended only when executing the update. -/
def eventSource (event : Event) : Source :=
  ⟨event.epoch,event.rawSpace,event.owner,.event,event.sensory.isSome⟩

def queryOf (event : Event) : Query :=
  ⟨event.epoch,event.rawSpace,event.owner,event.raw,event.input,event.episode,
    event.lifeAge,event.pose,some event.action,event.sensory⟩

def queryRow (event : Event) : External :=
  ({eventSource event with shape := .query},[.query (queryOf event)])

/-- Admission checks the complete supplied rows, including source and context;
an equal scalar raw key alone grants no authority. -/
def validRow (source : Source) (event query : External) (target : Value)
    (context : Nat × UInt64 × UInt8) : Bool :=
  match event.2,target with
  | [.event e],.bytes _ =>
    decide (event.1 = source ∧ event.1 = eventSource e ∧ query = queryRow e ∧
      context = (source.epoch,e.input,e.action))
  | _,_ => false

theorem admitted_query_exact (source : Source) (event query : External)
    (target : Value) (context : Nat × UInt64 × UInt8)
    (admitted : validRow source event query target context = true) :
    ∃ e bytes, event = (source,[.event e]) ∧ source = eventSource e ∧
      query = queryRow e ∧ target = .bytes bytes ∧ context = (source.epoch,e.input,e.action) := by
  unfold validRow at admitted
  split at admitted
  · rename_i e bytes he
    have h := of_decide_eq_true admitted
    exact ⟨e,bytes,Prod.ext h.1 he,h.1.symm.trans h.2.1,h.2.2.1,rfl,h.2.2.2⟩
  · cases admitted

/-- Before prediction, a retained row must agree with the physical journal's
pre-event input and complete outcome. Valid row shape alone is insufficient. -/
def matchesCompleted (event : External) (target : Value)
    (row : OutcomeTransitions.Row External (List Nat)) : Bool :=
  decide (event = row.input ∧ decode row.outcome = some target)

theorem completed_target_exact (event : External) (bytes : List UInt8)
    (row : OutcomeTransitions.Row External (List Nat))
    (agrees : matchesCompleted event (.bytes bytes) row = true) :
    event = row.input ∧ bytes.map UInt8.toNat = row.outcome := by
  have h := of_decide_eq_true agrees
  exact ⟨h.1,decode_exact row.outcome bytes h.2⟩

end AcquiredCausalObservation
