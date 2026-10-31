import Std.Tactic

/-!
Checked observations of paid work. Unknown observations and overflow propagate;
they are not zero. Conservation requires a complete, contiguous sequence of
nonincreasing budget observations, including the final returned boundary.

This is a model of diagnostic arithmetic and result-preserving instrumentation.
Native callers must establish interval/phase coverage, separation of semantic
state from diagnostic counters, and the absence of hidden budget mutation.
Returned errors are ordinary results here; unwinding executions are not modeled.
-/
namespace WorkObservation
set_option autoImplicit false
set_option warningAsError true

def checked (maximum value : Nat) : Option Nat :=
  if value ≤ maximum then some value else none

def add (maximum : Nat) (left right : Option Nat) : Option Nat :=
  left.bind fun a => right.bind fun b => checked maximum (a + b)

def delta (before after : Nat) : Option Nat :=
  if after ≤ before then some (before - after) else none

def credit (maximum : Nat) (totals : Nat → Option Nat) (phase : Nat)
    (amount : Option Nat) : Nat → Option Nat :=
  fun index => if index = phase then add maximum (totals phase) amount else totals index

def measure (maximum before : Nat) : List Nat → Option Nat
  | [] => some 0
  | after :: rest => add maximum (delta before after) (measure maximum after rest)

/-- The final endpoint is explicit: omitting the tail does not cover it. -/
inductive Covered : Nat → List Nat → Nat → Prop where
  | nil (remaining : Nat) : Covered remaining [] remaining
  | cons {before after final : Nat} {rest : List Nat}
      (monotone : after ≤ before) (tail : Covered after rest final) :
      Covered before (after :: rest) final

theorem checked_exact (maximum value : Nat) (fits : value ≤ maximum) :
    checked maximum value = some value := by simp [checked, fits]

theorem checked_overflow (maximum value : Nat) (overflow : maximum < value) :
    checked maximum value = none := by simp [checked, Nat.not_le_of_gt overflow]

theorem unknown_left (maximum : Nat) (right : Option Nat) :
    add maximum none right = none := rfl

theorem unknown_right (maximum : Nat) (left : Option Nat) :
    add maximum left none = none := by cases left <;> rfl

theorem known_add (maximum a b : Nat) (fits : a + b ≤ maximum) :
    add maximum (some a) (some b) = some (a + b) := checked_exact maximum (a + b) fits

theorem delta_conservation (before after : Nat) (monotone : after ≤ before) :
    delta before after = some (before - after) ∧ (before - after) + after = before := by
  exact ⟨by simp [delta, monotone], Nat.sub_add_cancel monotone⟩

theorem increased_budget_unknown (before after : Nat) (increased : before < after) :
    delta before after = none := by simp [delta, Nat.not_le_of_gt increased]

theorem credit_selected (maximum phase : Nat) (totals : Nat → Option Nat) (amount : Option Nat) :
    credit maximum totals phase amount phase = add maximum (totals phase) amount := by
  simp [credit]

theorem credit_other (maximum phase other : Nat) (totals : Nat → Option Nat)
    (amount : Option Nat) (different : other ≠ phase) :
    credit maximum totals phase amount other = totals other := by
  simp [credit, different]

theorem covered_monotone {before final : Nat} {samples : List Nat}
    (covered : Covered before samples final) : final ≤ before := by
  induction covered with
  | nil => exact Nat.le_refl _
  | cons monotone _ ih => exact Nat.le_trans ih monotone

theorem covered_conservation (maximum : Nat) {before final : Nat} {samples : List Nat}
    (covered : Covered before samples final) (fits : before - final ≤ maximum) :
    measure maximum before samples = some (before - final) := by
  induction covered with
  | nil => simp [measure]
  | @cons before after final rest monotone tail ih =>
    have final_le := covered_monotone tail
    have tail_fits : after - final ≤ maximum :=
      Nat.le_trans (Nat.sub_le_sub_right monotone final) fits
    have telescope : (before - after) + (after - final) = before - final := by omega
    simp only [measure, (delta_conservation before after monotone).1, ih tail_fits]
    have combined : (before - after) + (after - final) ≤ maximum := by rw [telescope]; exact fits
    rw [known_add maximum _ _ combined, telescope]

/-- Semantic state/result are separate from telemetry in this model. -/
def instrument {State Result : Type} (remaining : State → Nat)
    (action : State → State × Result) (state : State) : (State × Result) × Option Nat :=
  let outcome := action state
  (outcome, delta (remaining state) (remaining outcome.1))

theorem result_noninterference {State Result : Type} (remaining : State → Nat)
    (action : State → State × Result) (state : State) :
    (instrument remaining action state).1 = action state := rfl

end WorkObservation
