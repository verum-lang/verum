import VerumExternalReplay.CausalCoding
import VerumExternalReplay.EnumerableMass

/-! Proper distributions on finite wire bodies, using exact rational finite cuts.
Conservation alone allows mass on infinite bodies. A proper length law also
provides a vanishing tail. Width 1 denotes bit lengths; width 8 denotes byte
lengths. Typed serialization is a separate restriction of this complete carrier.
These are probability-mass laws, not native or statistical-stopping theorems. -/
namespace FiniteWire
open ExactMass EnumerableMass CausalCoding
set_option warningAsError true

structure LengthLaw where
  weight : Nat → Fraction
  remaining : Nat → Fraction
  conserved : ∀ n, Equal (add (partialSum weight n) (remaining n)) one
  vanishes : ∀ epsilon : Fraction, 0 < epsilon.numerator →
    ∃ cutoff, ∀ n, cutoff ≤ n → Less (remaining n) epsilon

theorem length_prefix_bounded (law : LengthLaw) (n : Nat) :
    AtMost (partialSum law.weight n) one :=
  le_trans (le_add _ _) (eq_le (law.conserved n))

/-- No positive mass may remain outside every finite cut. -/
theorem no_persistent_tail (law : LengthLaw) (epsilon : Fraction) (positive : 0 < epsilon.numerator) :
    ¬ ∀ n, AtMost epsilon (law.remaining n) := by
  obtain ⟨cutoff, small⟩ := law.vanishes epsilon positive
  intro lower
  have h := small cutoff (Nat.le_refl _)
  have k := lower cutoff
  simp only [Less, AtMost] at h k
  omega

/-- A fully explicit bound; no real limits or classical choice are required. -/
theorem geometric_tail (epsilon : Fraction) (positive : 0 < epsilon.numerator)
    (n : Nat) (large : epsilon.denominator ≤ n) : Less (half n) epsilon := by
  have scale : 2^n ≤ epsilon.numerator * 2^n := by
    simpa only [Nat.one_mul] using Nat.mul_le_mul_right (2^n) positive
  simp only [Less, half, unit, Nat.one_mul]
  exact Nat.lt_of_le_of_lt large (Nat.lt_of_lt_of_le Nat.lt_two_pow_self scale)

def geometric : LengthLaw where
  weight n := half (n+1)
  remaining := half
  conserved := prefix_exact dyadic
  vanishes epsilon positive := ⟨epsilon.denominator, geometric_tail epsilon positive⟩

/-- Length is drawn first. Kernel, retained state and mask may depend on that
length and prior observations, but not on the body that is about to be scored. -/
structure Model (State : Type) where
  unitBits : Nat
  positiveWidth : 0 < unitBits
  length : LengthLaw
  kernel : Nat → Kernel State
  initial : Nat → State
  prediction : Nat → List Bool

variable {State : Type}

def bodyMass (model : Model State) (n : Nat) (body : List Bool) : Fraction :=
  mul (model.length.weight n)
    (likelihood (model.kernel n) (model.initial n) (residual (model.prediction n) body))

/-- Derive length from the complete body; refuse partial units. -/
def score (model : Model State) (body : List Bool) : Option Fraction :=
  if body.length % model.unitBits = 0 then
    some (bodyMass model (body.length / model.unitBits) body)
  else none

theorem score_word (model : Model State) {body : List Bool} {n : Nat}
    (valid : body ∈ words (model.unitBits*n)) : score model body = some (bodyMass model n body) := by
  simp [score, word_length valid, Nat.mul_div_cancel_left n model.positiveWidth]

def sliceMass (model : Model State) (n : Nat) : Fraction :=
  sum ((words (model.unitBits * n)).map (bodyMass model n))

/-- A wire body cannot be counted in two different length slices. -/
theorem length_unique (model : Model State) {body : List Bool} {n m : Nat}
    (first : body ∈ words (model.unitBits*n)) (second : body ∈ words (model.unitBits*m)) : n = m := by
  have equal := (word_length first).symm.trans (word_length second)
  exact Nat.eq_of_mul_eq_mul_left model.positiveWidth equal

/-- Summing every complete body of a chosen length recovers its length mass. -/
theorem slice_exact (model : Model State) (n : Nat) :
    Equal (sliceMass model n) (model.length.weight n) := by
  apply eq_trans (sum_scale _ _ _)
  exact eq_trans (mul_congr_left _ (residual_normalized _ _ _ _)) (mul_one _)

theorem wire_prefix_exact (model : Model State) (n : Nat) :
    Equal (add (partialSum (sliceMass model) n) (model.length.remaining n)) one :=
  eq_trans (add_congr_right _ (prefix_congr _ _ (slice_exact model) n)) (model.length.conserved n)

/-- The induced distribution retains both conservation and the same vanishing
tail. Bounded partial sums by themselves would not establish this result. -/
def inducedLengthLaw (model : Model State) : LengthLaw where
  weight := sliceMass model
  remaining := model.length.remaining
  conserved := wire_prefix_exact model
  vanishes := model.length.vanishes

theorem wire_prefix_bounded (model : Model State) (n : Nat) :
    AtMost (partialSum (sliceMass model) n) one := length_prefix_bounded (inducedLengthLaw model) n

theorem wire_tail_vanishes (model : Model State) (epsilon : Fraction) (positive : 0 < epsilon.numerator) :
    ∃ cutoff, ∀ n, cutoff ≤ n → Less (model.length.remaining n) epsilon :=
  (inducedLengthLaw model).vanishes epsilon positive

/-- Invalid or noncanonical words remain unallocated rather than being silently
renormalized. This applies to every finite cut, including empty support slices. -/
def selectedSlice (model : Model State) (keep : List Bool → Bool) (n : Nat) : Fraction :=
  mul (model.length.weight n)
    (selectedMass (model.kernel n) (model.initial n) (model.prediction n) (model.unitBits*n) keep)

theorem selected_slice_bounded (model : Model State) (keep : List Bool → Bool) (n : Nat) :
    AtMost (selectedSlice model keep n) (model.length.weight n) :=
  le_trans (mul_mono_left _ (selected_bounded _ _ _ _ _)) (eq_le (mul_one _))

theorem selected_prefix_bounded (model : Model State) (keep : List Bool → Bool) (n : Nat) :
    AtMost (partialSum (selectedSlice model keep) n) one :=
  le_trans (prefix_mono _ _ (selected_slice_bounded model keep) n) (length_prefix_bounded model.length n)

end FiniteWire
