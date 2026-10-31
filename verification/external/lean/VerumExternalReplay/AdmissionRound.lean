import Std.Tactic

/-!
Accounting for a frozen round of service visits. The queued count and the
active-slot bit are stored separately; their mathematical sum may exceed the
machine-word range by one. A checked diagnostic sum is not the control state.

These laws count calls to `visited`. A caller must separately establish that
each such call accompanies one paid FIFO visit, that no snapshot owner can be
overtaken, and that the empty-roster bypass is truthful. Neither primitive
progress, successful admission, fairness nor native refinement follows here.
-/
namespace AdmissionRound
set_option autoImplicit false
set_option warningAsError true

structure State where
  queuedDue : Nat
  activeDue : Bool
  deriving DecidableEq

def issued (queued : Nat) (active : Bool) : State := ⟨queued, active⟩

def State.debt (state : State) : Nat :=
  state.queuedDue + if state.activeDue then 1 else 0

def State.ready (state : State) (empty : Bool) : Bool :=
  empty || (!state.activeDue && decide (state.queuedDue = 0))

def State.visited (state : State) : State :=
  if state.activeDue then { state with activeDue := false }
  else if state.queuedDue = 0 then state
  else { state with queuedDue := state.queuedDue - 1 }

def State.visits (state : State) : Nat → State
  | 0 => state
  | count + 1 => state.visited.visits count

def State.Bounded (state : State) (wordMax : Nat) : Prop :=
  state.queuedDue ≤ wordMax

def State.remaining (state : State) (wordMax : Nat) : Option Nat :=
  if state.debt ≤ wordMax then some state.debt else none

theorem issued_debt (queued : Nat) (active : Bool) :
    (issued queued active).debt = queued + if active then 1 else 0 := rfl

theorem ready_iff (state : State) (empty : Bool) :
    state.ready empty = true ↔ empty = true ∨ state.debt = 0 := by
  cases state with
  | mk queued active => cases active <;> cases empty <;> simp [State.ready, State.debt]

theorem visited_debt (state : State) : state.visited.debt = state.debt - 1 := by
  cases state with
  | mk queued active =>
    cases active <;> simp [State.visited, State.debt]
    split <;> simp_all

theorem visited_bounded (state : State) (wordMax : Nat) (bounded : state.Bounded wordMax) :
    state.visited.Bounded wordMax := by
  unfold State.visited
  split
  · exact bounded
  · split
    · exact bounded
    · exact Nat.le_trans (Nat.sub_le _ _) bounded

theorem active_visit_preserves_queue (queued : Nat) :
    (issued queued true).visited = issued queued false := rfl

theorem zero_visit_stable : (issued 0 false).visited = issued 0 false := rfl

theorem visits_debt (state : State) (count : Nat) :
    (state.visits count).debt = state.debt - count := by
  induction count generalizing state with
  | zero => rfl
  | succ count ih =>
    simp only [State.visits, ih, visited_debt]
    omega

theorem visits_bounded (state : State) (wordMax count : Nat)
    (bounded : state.Bounded wordMax) : (state.visits count).Bounded wordMax := by
  induction count generalizing state with
  | zero => exact bounded
  | succ count ih => exact ih state.visited (visited_bounded state wordMax bounded)

theorem ready_after_visits_iff (state : State) (count : Nat) :
    (state.visits count).ready false = true ↔ state.debt ≤ count := by
  rw [ready_iff, visits_debt]
  simp
  exact Nat.sub_eq_zero_iff_le

theorem bounded_debt (state : State) (wordMax : Nat) (bounded : state.Bounded wordMax) :
    state.debt ≤ wordMax + 1 := by
  unfold State.debt
  unfold State.Bounded at bounded
  split <;> omega

theorem remaining_some_iff (state : State) (wordMax value : Nat) :
    state.remaining wordMax = some value ↔ value = state.debt ∧ state.debt ≤ wordMax := by
  unfold State.remaining
  split <;> simp_all [eq_comm]

theorem remaining_none_iff (state : State) (wordMax : Nat) :
    state.remaining wordMax = none ↔ wordMax < state.debt := by
  unfold State.remaining
  split <;> simp_all

theorem bounded_overflow_iff (state : State) (wordMax : Nat)
    (bounded : state.Bounded wordMax) :
    state.remaining wordMax = none ↔ state.queuedDue = wordMax ∧ state.activeDue = true := by
  rw [remaining_none_iff]
  cases state with
  | mk queued active =>
    cases active with
    | false =>
      simp [State.debt]
      exact bounded
    | true =>
      simp [State.debt]
      constructor
      · intro above
        exact Nat.le_antisymm bounded (Nat.lt_succ_iff.mp above)
      · intro same
        rw [same]
        exact Nat.lt_succ_self _

theorem full_width_visit (wordMax : Nat) :
    (issued wordMax true).remaining wordMax = none ∧
    (issued wordMax true).visited.remaining wordMax = some wordMax := by
  simp [issued, State.visited, State.remaining, State.debt]

end AdmissionRound
