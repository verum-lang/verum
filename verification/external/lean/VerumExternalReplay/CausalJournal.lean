import VerumExternalReplay.ActionTargetWire
import VerumExternalReplay.OutcomeTransitions

/-! Publication of a completed physical event is separate from obtaining its
target bytes. A reservation retains a whole revision anchor and a nominal slot;
equal actions or payloads do not identify either owner. The native adapter must
establish that anchor/slot equality reflects the actual retained allocations.
This model does not prove affine typing, allocator behavior, callback purity,
internal tree cursors or the truth of a physical observation.
-/
namespace CausalJournal
open ActionCapture AcquiredObserver
set_option warningAsError true

structure Completed where
  observed : Observed
  observation : Option Sensory
  deriving DecidableEq

/-- Known terminal failure is a stopped action outcome. Unknown delivery is
absent; known delivery without sensor bytes is still a completed event. -/
def captured [DecidableEq A] (capture : Capture A D) : Option Completed :=
  match capture.result with
  | .ok observed => some ⟨observed,capture.observation⟩
  | .error failure =>
      if observationEligible capture.action capture.result then
        failure.terminal.map (fun kind => ⟨.terminal kind .stopped,capture.observation⟩)
      else none

def target (completed : Completed) : Except TargetRefusal Target :=
  match completed.observation with
  | none => .error .missing
  | some observation => .ok ⟨observedKind completed.observed,observation⟩

theorem target_agrees [DecidableEq A] (capture : Capture A D) :
    (match captured capture with
      | none => .error .unobserved
      | some completed => target completed) = semanticTarget capture := by
  cases h : capture.result with
  | ok observed => simp [captured,target,semanticTarget,targetKind,h] <;> rfl
  | error failure =>
      cases eligible : observationEligible capture.action (.error failure) <;>
        cases ht : failure.terminal <;>
        simp [captured,target,semanticTarget,targetKind,h,eligible,ht,observedKind] <;> rfl

theorem completed_ignores_restart [DecidableEq A] (capture : Capture A D)
    (restart : Option Sensory) :
    captured {capture with resumed := restart} = captured capture := rfl

theorem known_without_bytes [DecidableEq A] (action : A) (observed : Observed) :
    captured (D := D) ⟨action,.ok observed,none,none⟩ = some ⟨observed,none⟩ := rfl

structure Journal (Anchor Input : Type) where
  anchor : Anchor
  complete : Bool
  events : List (Input × Option Completed)
  deriving DecidableEq

structure Reservation (Anchor Slot Input Action : Type) where
  before : Anchor
  after : Anchor
  slot : Slot
  input : Input
  action : Action
  deriving DecidableEq

structure Pending (Anchor Slot Input Action : Type) where
  journal : Journal Anchor Input
  reservation : Reservation Anchor Slot Input Action

structure Invocation (Anchor Slot Input Action Detail : Type) where
  pending : Pending Anchor Slot Input Action
  capture : Capture Action Detail

structure Publication (Anchor Slot Input Action Detail : Type) where
  journal : Journal Anchor Input
  slot : Slot
  capture : Capture Action Detail

variable {Anchor Slot Input Action Detail : Type}

/-- The anchor represents all native same-owner comparisons, including roots;
it must not be reduced to an action, an ordinal or equality of state values. -/
def bind [DecidableEq Anchor] (journal : Journal Anchor Input)
    (reservation : Reservation Anchor Slot Input Action) :
    Option (Pending Anchor Slot Input Action) :=
  if journal.complete && decide (journal.anchor = reservation.before) then
    some ⟨journal,reservation⟩
  else none

def dispatch (pending : Pending Anchor Slot Input Action) : Pending Anchor Slot Input Action :=
  {pending with journal := {pending.journal with complete := false}}

/-- The requested action comes from the owned reservation, never a separate
caller-supplied action. The callback's physical fidelity remains a premise. -/
def invoke (pending : Pending Anchor Slot Input Action)
    (callback : Action → Capture Action Detail) : Invocation Anchor Slot Input Action Detail :=
  ⟨dispatch pending,callback pending.reservation.action⟩

def publish (pending : Pending Anchor Slot Input Action) (outcome : Option Completed) :
    Journal Anchor Input :=
  ⟨pending.reservation.after,true,pending.journal.events++[(pending.reservation.input,outcome)]⟩

/-- Refusal returns the exact invocation and leaves its dispatched journal
closed. Success fills only its reserved slot and publishes the completed row. -/
def observe [DecidableEq Action] (invocation : Invocation Anchor Slot Input Action Detail) :
    Except (Invocation Anchor Slot Input Action Detail) (Publication Anchor Slot Input Action Detail) :=
  if invocation.capture.action = invocation.pending.reservation.action then
    match captured invocation.capture with
    | none => .error invocation
    | some outcome => .ok ⟨publish invocation.pending (some outcome),
        invocation.pending.reservation.slot,invocation.capture⟩
  else .error invocation

theorem bind_exact [DecidableEq Anchor] (journal : Journal Anchor Input)
    (reservation : Reservation Anchor Slot Input Action) (pending : Pending Anchor Slot Input Action) :
    bind journal reservation = some pending ↔
      journal.complete = true ∧ journal.anchor = reservation.before ∧ pending = ⟨journal,reservation⟩ := by
  cases hc : journal.complete <;> by_cases same : journal.anchor = reservation.before <;>
    simp only [bind,hc,same] <;> simp [eq_comm]

theorem stale_refused [DecidableEq Anchor] (journal : Journal Anchor Input)
    (reservation : Reservation Anchor Slot Input Action) (stale : journal.anchor ≠ reservation.before) :
    bind journal reservation = none := by simp [bind,stale]

theorem incomplete_refused [DecidableEq Anchor] (journal : Journal Anchor Input)
    (reservation : Reservation Anchor Slot Input Action) (closed : journal.complete = false) :
    bind journal reservation = none := by simp [bind,closed]

theorem dispatch_preserves_history (pending : Pending Anchor Slot Input Action) :
    (dispatch pending).journal.events = pending.journal.events ∧
    (dispatch pending).journal.anchor = pending.journal.anchor ∧
    (dispatch pending).journal.complete = false := ⟨rfl,rfl,rfl⟩

theorem foreign_action_retains_invocation [DecidableEq Action]
    (invocation : Invocation Anchor Slot Input Action Detail)
    (foreign : invocation.capture.action ≠ invocation.pending.reservation.action) :
    observe invocation = .error invocation := by simp [observe,foreign]

theorem unknown_delivery_retains_invocation [DecidableEq Action]
    (invocation : Invocation Anchor Slot Input Action Detail)
    (unknown : captured invocation.capture = none) :
    observe invocation = .error invocation := by simp [observe,unknown]

theorem publication_exact [DecidableEq Action]
    (invocation : Invocation Anchor Slot Input Action Detail)
    (result : Publication Anchor Slot Input Action Detail) :
    observe invocation = .ok result ↔
      invocation.capture.action = invocation.pending.reservation.action ∧
      ∃ outcome, captured invocation.capture = some outcome ∧
        result = ⟨publish invocation.pending (some outcome),
          invocation.pending.reservation.slot,invocation.capture⟩ := by
  by_cases same : invocation.capture.action = invocation.pending.reservation.action
  · cases h : captured invocation.capture <;> simp [observe,same,h,eq_comm]
  · simp [observe,same]

theorem published_prefix (pending : Pending Anchor Slot Input Action) (outcome : Option Completed) :
    (publish pending outcome).events.take pending.journal.events.length = pending.journal.events := by
  simp [publish]

/-- Legacy confirmation publishes an event but supplies no completed outcome.
The history remains available as pre-event data; causal replay must refuse it. -/
def completedRows (events : List (Input × Option Completed)) :
    Option (List (OutcomeTransitions.Row Input Completed)) :=
  events.mapM (fun (input,outcome) => outcome.map (fun value => ⟨input,value⟩))

theorem missing_outcome_refuses (input : Input) (tail : List (Input × Option Completed)) :
    completedRows ((input,none)::tail) = none := rfl

theorem completed_rows_append (first second : List (Input × Option Completed)) :
    completedRows (first++second) = (do
      let a ← completedRows first
      let b ← completedRows second
      pure (a++b)) := List.mapM_append

theorem legacy_publication_has_no_complete_replay (pending : Pending Anchor Slot Input Action) :
    completedRows (publish pending none).events = none := by
  rw [publish,completed_rows_append]
  cases completedRows pending.journal.events <;> rfl

end CausalJournal
