import VerumExternalReplay.BinaryPrediction

/-! Finite KT banks indexed by a causal position schedule. A frozen forecast
matches incremental learning only when later positions use different cells.
Keys retain their complete values; no digest or quotient equality is used. -/
namespace PositionPrediction
open ExactMass CausalCoding
set_option warningAsError true

abbrev Bank (Key : Type) := List (Key × BinaryPrediction.State)
variable {Key : Type} [DecidableEq Key]

def lookup (bank : Bank Key) (key : Key) : BinaryPrediction.State :=
  match bank with
  | [] => BinaryPrediction.fromCounts 0 0
  | (k, counts) :: rest => if key = k then counts else lookup rest key

def observe (bank : Bank Key) (key : Key) (bit : Bool) : Bank Key :=
  match bank with
  | [] => [(key, BinaryPrediction.next (BinaryPrediction.fromCounts 0 0) bit)]
  | (k, counts) :: rest => if key = k then (k, BinaryPrediction.next counts bit) :: rest
      else (k, counts) :: observe rest key bit

theorem observed (bank : Bank Key) (key : Key) (bit : Bool) :
    lookup (observe bank key bit) key = BinaryPrediction.next (lookup bank key) bit := by
  induction bank with
  | nil => simp [lookup, observe]
  | cons entry rest ih =>
    rcases entry with ⟨k, counts⟩
    by_cases h : key = k <;> simp [observe, lookup, h, ih]

theorem other_unchanged (bank : Bank Key) (key other : Key) (bit : Bool) (different : other ≠ key) :
    lookup (observe bank key bit) other = lookup bank other := by
  induction bank with
  | nil => simp [lookup, observe, different]
  | cons entry rest ih =>
    rcases entry with ⟨k, counts⟩
    by_cases h : key = k
    · subst k; simp [observe, lookup, different]
    · by_cases ho : other = k <;> simp [observe, lookup, h, ho, ih]

structure Cursor (Key : Type) where
  position : Nat
  bank : Bank Key

def kernel (key : Nat → Key) : Kernel (Cursor Key) where
  probability s bit := BinaryPrediction.kernel.probability (lookup s.bank (key s.position)) bit
  next s bit := ⟨s.position+1, observe s.bank (key s.position) bit⟩
  normalized _ := BinaryPrediction.probabilities_normalized _

def frozen (key : Nat → Key) (bank : Bank Key) (position : Nat) : List Bool → Fraction
  | [] => one
  | bit :: rest => mul (BinaryPrediction.kernel.probability (lookup bank (key position)) bit)
      (frozen key bank (position+1) rest)

theorem frozen_congr (key : Nat → Key) (a b : Bank Key) (position : Nat) (body : List Bool)
    (agree : ∀ i, position ≤ i → lookup a (key i) = lookup b (key i)) :
    frozen key a position body = frozen key b position body := by
  induction body generalizing position with
  | nil => rfl
  | cons bit rest ih =>
    simp only [frozen, agree position (Nat.le_refl _)]
    rw [ih (position+1) (fun i hi => agree i (by omega))]

/-- No later position can see an earlier position's increment. -/
theorem frozen_after_update (key : Nat → Key) (unique : Function.Injective key)
    (bank : Bank Key) (position : Nat) (bit : Bool) (rest : List Bool) :
    frozen key (observe bank (key position) bit) (position+1) rest = frozen key bank (position+1) rest := by
  apply frozen_congr
  intro i hi
  apply other_unchanged
  intro same
  have := unique same
  omega

theorem incremental_eq_frozen (key : Nat → Key) (unique : Function.Injective key)
    (bank : Bank Key) (position : Nat) (body : List Bool) :
    likelihood (kernel key) ⟨position, bank⟩ body = frozen key bank position body := by
  induction body generalizing bank position with
  | nil => rfl
  | cons bit rest ih =>
    change mul _ (likelihood (kernel key) ⟨position+1, observe bank (key position) bit⟩ rest) = _
    rw [ih, frozen_after_update key unique]
    rfl

theorem frozen_normalized (key : Nat → Key) (unique : Function.Injective key)
    (bank : Bank Key) (position n : Nat) :
    Equal (sum ((words n).map (frozen key bank position))) one := by
  apply eq_trans (sum_congr (words n) _ _ ?_) (body_normalized (kernel key) ⟨position, bank⟩ n)
  intro body _
  rw [incremental_eq_frozen key unique]
  exact eq_refl _

end PositionPrediction
