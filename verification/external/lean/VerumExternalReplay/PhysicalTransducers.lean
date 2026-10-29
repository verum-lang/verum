import Init

/-! Exact ordered physical histories for total state transducers. Service is a
stutter by definition. Runtime correspondence must establish that premise,
including any refusal, ownership and bounded-arithmetic behavior. -/
namespace PhysicalTransducers
set_option autoImplicit false
set_option warningAsError true
universe u v w

inductive Event (X : Type u) (G : Type v) where
  | observation : X → Event X G
  | service : G → Event X G

def step {X : Type u} {G : Type v} {S : Type w}
    (observe : S → X → S) (state : S) : Event X G → S
  | .observation x => observe state x
  | .service _ => state

def run {X : Type u} {G : Type v} {S : Type w}
    (observe : S → X → S) : List (Event X G) → S → S
  | [], state => state
  | event :: events, state => run observe events (step observe state event)

def physical {X : Type u} {G : Type v} : List (Event X G) → List X
  | [] => []
  | .observation x :: events => x :: physical events
  | .service _ :: events => physical events

def fold {X : Type u} {S : Type w} (observe : S → X → S) : List X → S → S
  | [], state => state
  | x :: xs, state => fold observe xs (observe state x)

theorem service_stutters {X : Type u} {G : Type v} {S : Type w}
    (observe : S → X → S) (state : S) (grant : G) :
    step observe state (.service grant) = state := rfl

theorem observation_updates {X : Type u} {G : Type v} {S : Type w}
    (observe : S → X → S) (state : S) (x : X) :
    step (G := G) observe state (.observation x) = observe state x := rfl

theorem service_erasure {X : Type u} {G : Type v} {S : Type w}
    (observe : S → X → S) (events : List (Event X G)) (state : S) :
    run observe events state = fold observe (physical events) state := by
  induction events generalizing state with
  | nil => rfl
  | cons event events ih =>
    cases event with
    | observation x => exact ih (observe state x)
    | service _ => exact ih state

theorem same_physical_history_same_state {X : Type u} {G : Type v} {S : Type w}
    (observe : S → X → S) (a b : List (Event X G)) (state : S)
    (h : physical a = physical b) : run observe a state = run observe b state := by
  rw [service_erasure observe a state, service_erasure observe b state, h]

end PhysicalTransducers
