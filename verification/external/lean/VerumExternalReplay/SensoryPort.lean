import VerumExternalReplay.AcquiredObserver
import VerumExternalReplay.ObserverInstallation

/-! An immutable sensory snapshot is part of the complete captured observation.
The schema is opaque data, not an inferred source identity or interpreted pixel
layout. These laws constrain the adapter and history model; native capture time,
Arc ownership and external device fidelity still require correspondence evidence. -/
namespace SensoryPort
open AcquiredObserver
set_option warningAsError true

def eventSource (e : Event) : Source :=
  ⟨e.epoch, e.rawSpace, e.owner, .event, e.sensory.isSome⟩

def querySource (q : Query) : Source :=
  ⟨q.epoch, q.rawSpace, q.owner, .query, q.sensory.isSome⟩

/-- Both projections retain one captured input. The action can be absent or a
candidate action; changing it cannot replace the captured sensory snapshot. -/
def queryAt (e : Event) (action : Option UInt8) : Query :=
  ⟨e.epoch, e.rawSpace, e.owner, e.raw, e.input, e.episode, e.lifeAge, e.pose, action, e.sensory⟩

theorem encoding_injective (a b : Sensory) : sensoryValue a = sensoryValue b ↔ a = b := by
  cases a; cases b
  simp [sensoryValue]

theorem event_query_read_same (limit : Nat) (e : Event) (action : Option UInt8) :
    apply limit .eventSensory [.event e] = apply limit .querySensory [.query (queryAt e action)] := by
  simp [apply, arity, queryAt]

theorem missing_is_undefined (limit : Nat) (e : Event) (missing : e.sensory = none) :
    apply limit .eventSensory [.event e] = .error .missing := by
  simp [apply, arity, missing]

theorem present_is_exact (limit : Nat) (e : Event) (s : Sensory) (present : e.sensory = some s) :
    apply limit .eventSensory [.event e] = .ok (.pair (.bytes s.schema) (.bytes s.body)) := by
  simp [apply, arity, present, sensoryValue]

theorem payload_is_not_source_identity (e : Event) (a b : Sensory) :
    eventSource {e with sensory := some a} = eventSource {e with sensory := some b} := rfl

theorem captured_presence_is_source_identity (e : Event) (s : Sensory) :
    eventSource {e with sensory := some s} ≠ eventSource {e with sensory := none} := by
  simp [eventSource]

theorem mismatched_port_refuses_before_execution (limit : Nat) (code : FiniteCode.Code Op Value)
    (binding : ObserverAdmission.Binding Source) (state : Value) (e : Event) (values : List Value)
    (different : e.sensory.isSome ≠ binding.interface.sensory) :
    run limit code binding state (eventSource e) values = none := by
  have mismatch : eventSource e ≠ binding.interface := by
    intro equal
    exact different (congrArg Source.sensory equal)
  simp [run, mismatch]

theorem changed_capture_distinct (e : Event) (a b : Option Sensory) (different : a ≠ b) :
    ({e with sensory := a} : Event) ≠ {e with sensory := b} := by
  intro equal
  exact different (congrArg Event.sensory equal)

def history (origin : Origin) (e : Event) : ObserverInstallation.History Origin Event :=
  ⟨origin, [some e]⟩

/-- Retained-history compatibility compares presence, schema and every body
byte, even if raw keys and all older scalar features are unchanged. -/
theorem changed_capture_refuses_install [DecidableEq Origin]
    (step : State → Event → Option State) (initial : State) (origin : Origin)
    (e : Event) (a b : Option Sensory) (different : a ≠ b) :
    ObserverInstallation.install step initial [history origin {e with sensory := a}]
      (history origin {e with sensory := b}) 1 = none := by
  apply ObserverInstallation.conflicting_origin_refused
  · rfl
  · have changed := changed_capture_distinct e a b different
    simpa [history] using changed

def retainedPort (events : List Event) (index : Nat) : Option Sensory :=
  events[index]?.bind Event.sensory

theorem future_append_preserves_capture (before after : List Event) (index : Nat)
    (retained : index < before.length) :
    retainedPort (before ++ after) index = retainedPort before index := by
  simp only [retainedPort, List.getElem?_append_left retained]

/-- Online execution and replay consume the same complete events, without
projecting them to raw keys or reconstructing their sensory ports afterward. -/
theorem live_replay (step : State → Event → Option State) (events : List Event) (initial : State) :
    PartialObservers.online step events (some initial) = PartialObservers.fold step events initial := by
  simpa using (PartialObservers.fold_online step events (some initial)).symm

end SensoryPort
