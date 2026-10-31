import Std.Tactic

/-! Exact delayed membership transport over a closed deterministic lasso.
The bounded accumulator denotes a transition function, not a unique history or
commit count. Native owner/dispatch guards must separately ensure one increment
per actual successful commit. No source law or finite world period follows.
-/
namespace BoundedLasso
set_option warningAsError true

def next (size closing index : Nat) : Nat :=
  if index + 1 < size then index + 1 else closing

def iterate (size closing : Nat) : Nat → Nat → Nat
  | 0, index => index
  | count + 1, index => next size closing (iterate size closing count index)

/-- Subtract two already reduced residues without machine-width addition. -/
def subResidue (period left right : Nat) : Nat :=
  if right ≤ left then left - right else period - (right - left)

structure Summary where
  capped : Nat
  residue : Nat
  deriving DecidableEq

def elapsed (size closing count : Nat) : Summary :=
  ⟨min count size, count % (size - closing)⟩

/-- Both additions are guarded by strict bounds. Neither forms size+1 or
period+1, even when a machine implementation stores its maximum integer. -/
def push (size closing : Nat) (summary : Summary) : Summary :=
  ⟨if summary.capped < size then summary.capped + 1 else summary.capped,
   if summary.residue + 1 < size - closing then summary.residue + 1 else 0⟩

def transport (size closing : Nat) (summary : Summary) (index : Nat) : Nat :=
  let distance := size - index
  if summary.capped < distance then index + summary.capped
  else closing + subResidue (size - closing) summary.residue (distance % (size - closing))

theorem subResidue_lt (period left right : Nat) (hl : left < period) :
    subResidue period left right < period := by
  unfold subResidue
  split <;> omega

/-- A machine implementation can establish the unique residue through a
bounded lift, without asking its solver to prove a second variable-width
division. The carry equation determines the mathematical result; the separate
range theorem supplies its bound when the left operand is also reduced. -/
theorem subResidue_unique (period left right result : Nat)
    (hr : right < period)
    (lift : result + (if right ≤ left then period else 0) = left + period - right) :
    result = subResidue period left right := by
  by_cases noBorrow : right ≤ left
  · simp only [subResidue, ite_eq_left noBorrow] at *
    omega
  · simp only [subResidue, ite_eq_right noBorrow] at *
    omega

/-- A source checker can establish these branch-local equations using widened
machine arithmetic. They determine the exact abstract transport result; no
solver certificate or implementation behavior is assumed by this theorem. -/
theorem transport_of_lift (size closing index value : Nat) (summary : Summary)
    (closed : closing < size)
    (beforeWrap : summary.capped < size - index → value = index + summary.capped)
    (cycle : size - index ≤ summary.capped → closing ≤ value ∧
      value - closing + (if (size - index) % (size - closing) ≤ summary.residue
        then size - closing else 0) =
      summary.residue + (size - closing) - (size - index) % (size - closing)) :
    value = transport size closing summary index := by
  unfold transport
  dsimp only
  by_cases before : summary.capped < size - index
  · rw [ite_eq_left before]
    exact beforeWrap before
  · rw [ite_eq_right before]
    obtain ⟨inside, lift⟩ := cycle (by omega)
    have exactResidue := subResidue_unique (size - closing) summary.residue
      ((size - index) % (size - closing)) (value - closing)
      (Nat.mod_lt _ (by omega)) lift
    omega

theorem subResidue_exact (period count distance : Nat) (positive : 0 < period)
    (enough : distance ≤ count) :
    subResidue period (count % period) (distance % period) = (count - distance) % period := by
  have hs := Nat.mod_lt distance positive
  have hx := Nat.mod_lt (count - distance) positive
  have sum : (distance % period + (count - distance) % period) % period = count % period := by
    rw [← Nat.add_mod, Nat.add_sub_of_le enough]
  by_cases nowrap : distance % period + (count - distance) % period < period
  · rw [Nat.mod_eq_of_lt nowrap] at sum
    unfold subResidue
    rw [ite_eq_left (by omega)]
    omega
  · have wraps : period ≤ distance % period + (count - distance) % period := by omega
    rw [Nat.mod_eq_sub_mod wraps, Nat.mod_eq_of_lt (by omega)] at sum
    unfold subResidue
    rw [ite_eq_right (by omega)]
    omega

theorem next_bounded (size closing index : Nat) (closed : closing < size) :
    next size closing index < size := by
  unfold next
  split <;> omega

theorem iterate_bounded (size closing count index : Nat)
    (closed : closing < size) (inside : index < size) :
    iterate size closing count index < size := by
  induction count with
  | zero => exact inside
  | succ count _ => exact next_bounded size closing _ closed

theorem next_in_cycle (size closing offset : Nat) (closed : closing < size)
    (inside : offset < size - closing) :
    next size closing (closing + offset) = closing + ((offset + 1) % (size - closing)) := by
  by_cases room : offset + 1 < size - closing
  · rw [Nat.mod_eq_of_lt room]
    simp only [next, ite_eq_left (show closing + offset + 1 < size by omega)]
    omega
  · have last : offset + 1 = size - closing := by omega
    rw [last, Nat.mod_self, Nat.add_zero]
    simp only [next, ite_eq_right (show ¬ closing + offset + 1 < size by omega)]

/-- The transient lasts exactly size-index steps; only then does the cycle's
modulus apply. The origin need not be the beginning of the carrier. -/
theorem iterate_exact (size closing count index : Nat)
    (closed : closing < size) (inside : index < size) :
    iterate size closing count index =
      if count < size - index then index + count
      else closing + ((count - (size - index)) % (size - closing)) := by
  have positive : 0 < size - closing := by omega
  induction count with
  | zero => simp [iterate, show 0 < size - index by omega]
  | succ count ih =>
    rw [iterate, ih]
    by_cases before : count < size - index
    · rw [ite_eq_left before]
      by_cases stays : count + 1 < size - index
      · rw [ite_eq_left stays]
        simp only [next, ite_eq_left (show index + count + 1 < size by omega)]
        omega
      · rw [ite_eq_right stays]
        have ends : count + 1 - (size - index) = 0 := by omega
        rw [ends, Nat.zero_mod, Nat.add_zero]
        simp only [next, ite_eq_right (show ¬ index + count + 1 < size by omega)]
    · rw [ite_eq_right before, ite_eq_right (show ¬ count + 1 < size - index by omega)]
      rw [next_in_cycle size closing _ closed (Nat.mod_lt _ positive), Nat.mod_add_mod]
      congr 2
      omega

theorem elapsed_bounded (size closing count : Nat) (closed : closing < size) :
    (elapsed size closing count).capped ≤ size ∧
    (elapsed size closing count).residue < size - closing :=
  ⟨Nat.min_le_right _ _, Nat.mod_lt _ (by omega)⟩

theorem push_exact (size closing count : Nat) (closed : closing < size) :
    push size closing (elapsed size closing count) = elapsed size closing (count + 1) := by
  have hr := Nat.mod_lt count (show 0 < size - closing by omega)
  have cap : (if min count size < size then min count size + 1 else min count size) =
      min (count + 1) size := by split <;> omega
  have residue : (if count % (size - closing) + 1 < size - closing
      then count % (size - closing) + 1 else 0) = (count + 1) % (size - closing) := by
    rw [← Nat.mod_add_mod]
    split
    · exact (Nat.mod_eq_of_lt ‹_›).symm
    · have last : count % (size - closing) + 1 = size - closing := by omega
      rw [last, Nat.mod_self]
  unfold push elapsed
  dsimp only
  rw [cap, residue]

theorem push_bounded (size closing count : Nat) (closed : closing < size) :
    (push size closing (elapsed size closing count)).capped ≤ size ∧
    (push size closing (elapsed size closing count)).residue < size - closing := by
  rw [push_exact size closing count closed]
  exact elapsed_bounded size closing (count + 1) closed

theorem push_intermediates_bounded (size closing count : Nat) (closed : closing < size) :
    ((elapsed size closing count).capped < size →
      (elapsed size closing count).capped + 1 ≤ size) ∧
    (elapsed size closing count).residue + 1 ≤ size - closing := by
  have bounds := elapsed_bounded size closing count closed
  constructor
  · intro room; omega
  · omega

theorem transport_exact (size closing count index : Nat)
    (closed : closing < size) (inside : index < size) :
    transport size closing (elapsed size closing count) index = iterate size closing count index := by
  rw [iterate_exact size closing count index closed inside]
  unfold transport elapsed
  dsimp only
  have cap : min count size < size - index ↔ count < size - index := by
    constructor <;> intro h <;> omega
  by_cases before : count < size - index
  · rw [ite_eq_left before, ite_eq_left (cap.mpr before)]
    rw [Nat.min_eq_left (by omega)]
  · rw [ite_eq_right before, ite_eq_right (fun h => before (cap.mp h))]
    rw [subResidue_exact (size - closing) count (size - index) (by omega) (by omega)]

theorem transport_push (size closing count index : Nat)
    (closed : closing < size) (inside : index < size) :
    transport size closing (push size closing (elapsed size closing count)) index =
      next size closing (transport size closing (elapsed size closing count) index) := by
  rw [push_exact size closing count closed,
    transport_exact size closing (count+1) index closed inside,
    transport_exact size closing count index closed inside]
  rfl

theorem transport_bounded (size closing count index : Nat)
    (closed : closing < size) (inside : index < size) :
    transport size closing (elapsed size closing count) index < size := by
  rw [transport_exact size closing count index closed inside]
  exact iterate_bounded size closing count index closed inside

end BoundedLasso
