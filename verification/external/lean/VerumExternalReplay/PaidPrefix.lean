import Std.Tactic

/-!
Paid service coverage over a frozen prefix of append-only identities. The cursor
advances only when its exact next identity receives positive paid work. Repeated
visits to other identities cannot discharge the prefix, even if their total
number exceeds its length. The endpoint is frozen until the caller issues a new
prefix; later appends cannot silently increase or discharge this obligation.

The event stream must represent actual paid dispatches to stable identities.
These laws do not establish that a native caller supplies that stream, that a
paid visit makes semantic progress, or that scheduling eventually visits anyone.
-/
namespace PaidPrefix
set_option autoImplicit false
set_option warningAsError true

structure State where
  next : Nat
  stop : Nat
  deriving DecidableEq

structure Event where
  identity : Nat
  paid : Nat
  deriving DecidableEq

def issued (stop : Nat) : State := ⟨0, stop⟩

def State.ready (state : State) : Bool := decide (state.next = state.stop)

def State.Bounded (state : State) (wordMax : Nat) : Prop :=
  state.next ≤ state.stop ∧ state.stop ≤ wordMax

def State.visit (state : State) (event : Event) : State :=
  if state.next < state.stop ∧ event.identity = state.next ∧ 0 < event.paid
  then { state with next := state.next + 1 }
  else state

def State.run (state : State) : List Event → State
  | [] => state
  | event :: events => (state.visit event).run events

def Witness (events : List Event) (identity : Nat) : Prop :=
  ∃ event ∈ events, event.identity = identity ∧ 0 < event.paid

theorem visit_stop (state : State) (event : Event) :
    (state.visit event).stop = state.stop := by
  unfold State.visit
  split <;> rfl

theorem visit_monotone (state : State) (event : Event) :
    state.next ≤ (state.visit event).next := by
  unfold State.visit
  split <;> simp

theorem visit_bounded (state : State) (event : Event) (wordMax : Nat)
    (bounded : state.Bounded wordMax) : (state.visit event).Bounded wordMax := by
  unfold State.visit
  split
  · rename_i eligible
    exact ⟨Nat.succ_le_of_lt eligible.1, bounded.2⟩
  · exact bounded

theorem unpaid_stable (state : State) (identity : Nat) :
    state.visit ⟨identity, 0⟩ = state := by
  simp [State.visit]

theorem wrong_identity_stable (state : State) (event : Event)
    (different : event.identity ≠ state.next) : state.visit event = state := by
  simp [State.visit, different]

theorem exact_paid_advance (state : State) (paid : Nat)
    (pending : state.next < state.stop) (positive : 0 < paid) :
    state.visit ⟨state.next, paid⟩ = ⟨state.next + 1, state.stop⟩ := by
  simp [State.visit, pending, positive]

theorem ready_visit_stable (state : State) (event : Event)
    (ready : state.ready = true) : state.visit event = state := by
  have same : state.next = state.stop := of_decide_eq_true ready
  simp [State.visit, same]

theorem outside_prefix_stable (state : State) (event : Event)
    (outside : state.stop ≤ event.identity) : state.visit event = state := by
  unfold State.visit
  split
  · rename_i eligible
    have : False := by omega
    exact this.elim
  · rfl

theorem run_stop (state : State) (events : List Event) :
    (state.run events).stop = state.stop := by
  induction events generalizing state with
  | nil => rfl
  | cons event events ih => rw [State.run, ih, visit_stop]

theorem run_monotone (state : State) (events : List Event) :
    state.next ≤ (state.run events).next := by
  induction events generalizing state with
  | nil => exact Nat.le_refl _
  | cons event events ih =>
    exact Nat.le_trans (visit_monotone state event) (ih (state.visit event))

theorem run_bounded (state : State) (events : List Event) (wordMax : Nat)
    (bounded : state.Bounded wordMax) : (state.run events).Bounded wordMax := by
  induction events generalizing state with
  | nil => exact bounded
  | cons event events ih =>
    exact ih (state.visit event) (visit_bounded state event wordMax bounded)

theorem visit_coverage (state : State) (event : Event) (identity : Nat)
    (lower : state.next ≤ identity) (upper : identity < (state.visit event).next) :
    event.identity = identity ∧ 0 < event.paid := by
  unfold State.visit at upper
  split at upper
  · rename_i eligible
    change identity < state.next + 1 at upper
    have same : identity = state.next := Nat.le_antisymm (Nat.le_of_lt_succ upper) lower
    exact ⟨eligible.2.1.trans same.symm, eligible.2.2⟩
  · omega

theorem run_coverage (state : State) (events : List Event) (identity : Nat)
    (lower : state.next ≤ identity) (upper : identity < (state.run events).next) :
    Witness events identity := by
  induction events generalizing state with
  | nil => exact (Nat.not_lt_of_ge lower upper).elim
  | cons event events ih =>
    cases (inferInstance : Decidable (identity < (state.visit event).next)) with
    | isTrue first =>
      exact ⟨event, List.mem_cons_self, visit_coverage state event identity lower first⟩
    | isFalse first =>
      obtain ⟨observed, member, paid⟩ := ih (state.visit event) (Nat.le_of_not_gt first) upper
      exact ⟨observed, List.mem_cons_of_mem event member, paid⟩

theorem completed_prefix_coverage (stop : Nat) (events : List Event)
    (ready : ((issued stop).run events).ready = true) (identity : Nat)
    (inside : identity < stop) : Witness events identity := by
  have same : ((issued stop).run events).next = stop := by
    have h := of_decide_eq_true ready
    exact h.trans (run_stop (issued stop) events)
  exact run_coverage (issued stop) events identity (Nat.zero_le _) (by rw [same]; exact inside)

end PaidPrefix
