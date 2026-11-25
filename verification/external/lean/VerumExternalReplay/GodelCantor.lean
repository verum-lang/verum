import Init
import Init.Data.Nat.Sqrt.Lemmas

namespace Verum.GodelCantor
set_option warningAsError true

def triangle : Nat → Nat
  | 0 => 0
  | n + 1 => triangle n + n + 1

theorem twice_triangle (n : Nat) : 2 * triangle n = n * (n + 1) := by
  induction n with
  | zero => rfl
  | succ n ih => simp only [triangle]; grind

theorem triangle_formula (n : Nat) : triangle n = n * (n + 1) / 2 := by
  rw [← twice_triangle]; omega

theorem triangle_mono {i j : Nat} (h : i ≤ j) : triangle i ≤ triangle j := by
  induction h with
  | refl => exact Nat.le_refl _
  | step _ ih => simp only [triangle]; omega

theorem le_triangle (n : Nat) : n ≤ triangle n := by
  cases n with
  | zero => decide
  | succ n => simp only [triangle]; omega

def pair (a b : Nat) := triangle (a + b) + b

def diagonal (z : Nat) := ((8 * z + 1).sqrt - 1) / 2

private theorem square_boundary (n : Nat) :
    (2*n+1) * (2*n+1) = 8*triangle n+1 := by
  have h := twice_triangle n
  grind

theorem diagonal_interval (z : Nat) : triangle (diagonal z) ≤ z ∧ z < triangle (diagonal z + 1) := by
  let s := (8*z+1).sqrt
  have lower : s*s ≤ 8*z+1 := Nat.sqrt_le _
  have upper : 8*z+1 < (s+1)*(s+1) := Nat.lt_succ_sqrt _
  have positive : 0 < s := by
    by_cases hs : s = 0
    · rw [hs] at upper
      omega
    · omega
  have low : 2*diagonal z+1 ≤ s := by change 2*((s-1)/2)+1 ≤ s; omega
  have high : s+1 ≤ 2*(diagonal z+1)+1 := by change s+1 ≤ 2*((s-1)/2+1)+1; omega
  have left := Nat.le_trans (Nat.mul_le_mul low low) lower
  have right := Nat.lt_of_lt_of_le upper (Nat.mul_le_mul high high)
  rw [square_boundary] at left right
  omega

private theorem interval_unique {i j z : Nat}
    (hi : triangle i ≤ z ∧ z < triangle (i+1))
    (hj : triangle j ≤ z ∧ z < triangle (j+1)) : i = j := by
  rcases Nat.lt_trichotomy i j with h | h | h
  · have := triangle_mono (show i+1 ≤ j from h)
    omega
  · exact h
  · have := triangle_mono (show j+1 ≤ i from h)
    omega

def unpair (z : Nat) : Nat × Nat :=
  let d := diagonal z
  let b := z - triangle d
  (d - b, b)

theorem unpair_pair (a b : Nat) : unpair (pair a b) = (a,b) := by
  have interval : triangle (a+b) ≤ pair a b ∧ pair a b < triangle (a+b+1) := by
    simp only [pair, triangle]; omega
  have d := interval_unique (diagonal_interval (pair a b)) interval
  unfold unpair
  rw [d]
  unfold pair
  apply Prod.ext <;> dsimp <;> omega

theorem pair_unpair (z : Nat) : pair (unpair z).1 (unpair z).2 = z := by
  obtain ⟨lo, hi⟩ := diagonal_interval z
  simp only [triangle] at hi
  have sum : diagonal z - (z - triangle (diagonal z)) + (z - triangle (diagonal z)) = diagonal z := by omega
  simp only [unpair, pair, sum]
  omega

/-- Natural model of the two checked conversions in the native word encoder. -/
def wordMax := 2^64-1
def pairWord (a b : Nat) : Option Nat :=
  if a+b ≤ wordMax then
    let code := (a+b)*(a+b+1)/2+b
    if code ≤ wordMax then some code else none
  else none

theorem pairWord_exact (a b : Nat) :
    pairWord a b = if pair a b ≤ wordMax then some (pair a b) else none := by
  unfold pairWord
  rw [← triangle_formula]
  change (if a+b ≤ wordMax then (if pair a b ≤ wordMax then some (pair a b) else none) else none) = _
  by_cases hab : a+b ≤ wordMax
  · simp [hab]
  · have h := le_triangle (a+b)
    have bad : ¬pair a b ≤ wordMax := by unfold pair; omega
    simp [hab,bad]

theorem word_roundtrip {z : Nat} (hz : z ≤ wordMax) :
    pairWord (unpair z).1 (unpair z).2 = some z := by
  rw [pairWord_exact, pair_unpair]
  simp [hz]

theorem intermediate_bounds {z d b : Nat} (hz : z ≤ wordMax) (hd : d ≤ wordMax) (hb : b ≤ d) :
    8*z+1 < 2^128 ∧ d*(d+1)+b < 2^128 := by
  have h := Nat.mul_le_mul hd (Nat.succ_le_succ hd)
  dsimp [wordMax] at hz hd h
  omega

theorem decoded_components_fit {z : Nat} (hz : z ≤ wordMax) :
    (unpair z).1 ≤ wordMax ∧ (unpair z).2 ≤ wordMax := by
  obtain ⟨lo, hi⟩ := diagonal_interval z
  have d := le_triangle (diagonal z)
  simp only [triangle] at hi
  unfold unpair
  dsimp
  omega

theorem pair_injective {a b c d : Nat} (h : pair a b = pair c d) : (a,b) = (c,d) := by
  have eq := congrArg unpair h
  simpa only [unpair_pair] using eq

/-- The native list stores its length in the surrounding tag and requires
    zero residual payload at that exact length. -/
def encodeList : List Nat → Option Nat
  | [] => some 0
  | x :: xs => do return ← pairWord x (← encodeList xs)

def decodeList : Nat → Nat → Option (List Nat)
  | 0, z => if z = 0 then some [] else none
  | n+1, z => do return (unpair z).1 :: (← decodeList n (unpair z).2)

theorem list_roundtrip (xs : List Nat) {z : Nat} (encoded : encodeList xs = some z) :
    decodeList xs.length z = some xs := by
  induction xs generalizing z with
  | nil =>
    have h : z = 0 := by simpa [encodeList] using encoded.symm
    subst z; rfl
  | cons x xs ih =>
    cases ht : encodeList xs with
    | none => simp [encodeList, ht] at encoded
    | some tail =>
      have h : pairWord x tail = some z := by simpa [encodeList, ht] using encoded
      rw [pairWord_exact] at h
      split at h
      · have hz : pair x tail = z := Option.some.inj h
        subst z
        simp only [List.length_cons, decodeList, unpair_pair, ih ht]
        rfl
      · contradiction

theorem list_injective {xs ys : List Nat} {z : Nat}
    (sameLength : xs.length = ys.length)
    (hx : encodeList xs = some z) (hy : encodeList ys = some z) : xs = ys := by
  have h := list_roundtrip xs hx
  rw [sameLength, list_roundtrip ys hy] at h
  exact (Option.some.inj h).symm

#print axioms unpair_pair
#print axioms pair_unpair
#print axioms pairWord_exact
#print axioms word_roundtrip
#print axioms intermediate_bounds
#print axioms decoded_components_fit
#print axioms list_roundtrip
#print axioms list_injective
end Verum.GodelCantor
