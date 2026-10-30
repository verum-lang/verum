import VerumExternalReplay.ExactMass
import Init.Data.Nat.Coprime

/-! Cross-cancelled exact products reuse the existing nonnegative rational model.
Canonical input evidence justifies omitting a final gcd. Zero numerators are
allowed; division requires a strictly positive divisor. Native ownership,
poisoning, allocation and performance remain separate correspondence obligations. -/
namespace CanonicalProduct
open ExactMass
set_option warningAsError true

def Canonical (a : Fraction) : Prop := Nat.Coprime a.numerator a.denominator
instance (a : Fraction) : Decidable (Canonical a) := inferInstanceAs (Decidable (Nat.Coprime _ _))

def crossMul (a b : Fraction) : Fraction :=
  let g := Nat.gcd a.numerator b.denominator
  let h := Nat.gcd b.numerator a.denominator
  ⟨(a.numerator / g) * (b.numerator / h),
   (a.denominator / h) * (b.denominator / g),
   Nat.mul_pos (Nat.div_gcd_pos_of_pos_right b.numerator a.positive)
     (Nat.div_gcd_pos_of_pos_right a.numerator b.positive)⟩

theorem cross_mul_exact (a b : Fraction) : Equal (crossMul a b) (mul a b) := by
  let g := Nat.gcd a.numerator b.denominator
  let h := Nat.gcd b.numerator a.denominator
  have an : a.numerator / g * g = a.numerator := Nat.div_mul_cancel (Nat.gcd_dvd_left _ _)
  have bd : b.denominator / g * g = b.denominator := Nat.div_mul_cancel (Nat.gcd_dvd_right _ _)
  have bn : b.numerator / h * h = b.numerator := Nat.div_mul_cancel (Nat.gcd_dvd_left _ _)
  have ad : a.denominator / h * h = a.denominator := Nat.div_mul_cancel (Nat.gcd_dvd_right _ _)
  change (a.numerator / g * (b.numerator / h)) * (a.denominator * b.denominator) =
    (a.numerator * b.numerator) * (a.denominator / h * (b.denominator / g))
  calc
    _ = (a.numerator / g * (b.numerator / h)) *
        ((a.denominator / h * h) * (b.denominator / g * g)) := by rw [ad, bd]
    _ = ((a.numerator / g * g) * (b.numerator / h * h)) *
        (a.denominator / h * (b.denominator / g)) := by ac_rfl
    _ = _ := by rw [an, bn]

theorem cross_mul_strict_order (a b threshold : Fraction) :
    Less (crossMul a b) threshold ↔ Less (mul a b) threshold :=
  ⟨fun h => le_less_trans (eq_le (eq_symm (cross_mul_exact a b))) h,
   fun h => le_less_trans (eq_le (cross_mul_exact a b)) h⟩

theorem cross_mul_canonical (a b : Fraction) (ha : Canonical a) (hb : Canonical b) :
    Canonical (crossMul a b) := by
  have ab := (ha.coprime_div_left (Nat.gcd_dvd_left a.numerator b.denominator)).coprime_div_right
    (Nat.gcd_dvd_right b.numerator a.denominator)
  have ad := Nat.gcd_div_gcd_div_gcd_of_pos_right (n := a.numerator) b.positive
  have cb := Nat.gcd_div_gcd_div_gcd_of_pos_right (n := b.numerator) a.positive
  have cd := (hb.coprime_div_left (Nat.gcd_dvd_left b.numerator a.denominator)).coprime_div_right
    (Nat.gcd_dvd_right a.numerator b.denominator)
  exact (ab.mul_right ad).mul_left (Nat.Coprime.mul_right cb cd)

theorem cross_mul_positive (a b : Fraction) (ha : 0 < a.numerator) (hb : 0 < b.numerator) :
    0 < (crossMul a b).numerator :=
  Nat.mul_pos (Nat.div_gcd_pos_of_pos_left b.denominator ha)
    (Nat.div_gcd_pos_of_pos_left a.denominator hb)

def inverse (a : Fraction) (positive : 0 < a.numerator) : Fraction :=
  ⟨a.denominator, a.numerator, positive⟩
def crossDiv (a b : Fraction) (positive : 0 < b.numerator) : Fraction :=
  crossMul a (inverse b positive)

theorem inverse_canonical (a : Fraction) (positive : 0 < a.numerator) (ha : Canonical a) :
    Canonical (inverse a positive) := ha.symm

theorem cross_div_exact (a b : Fraction) (positive : 0 < b.numerator) :
    Equal (crossDiv a b positive) (mul a (inverse b positive)) := cross_mul_exact _ _

theorem cross_div_canonical (a b : Fraction) (positive : 0 < b.numerator)
    (ha : Canonical a) (hb : Canonical b) : Canonical (crossDiv a b positive) :=
  cross_mul_canonical a _ ha (inverse_canonical b positive hb)

theorem cross_div_positive (a b : Fraction) (ha : 0 < a.numerator) (hb : 0 < b.numerator) :
    0 < (crossDiv a b hb).numerator := cross_mul_positive a _ ha b.positive

/-- Canonical value equality is exact numerator/denominator identity, including zero. -/
theorem canonical_unique (a b : Fraction) (ha : Canonical a) (hb : Canonical b)
    (same : Equal a b) : a = b := by
  have ad : a.denominator ∣ b.denominator := ha.symm.dvd_of_dvd_mul_left
    (same ▸ Nat.dvd_mul_left a.denominator b.numerator)
  have bd : b.denominator ∣ a.denominator := hb.symm.dvd_of_dvd_mul_left
    (same.symm ▸ Nat.dvd_mul_left b.denominator a.numerator)
  have den := Nat.dvd_antisymm ad bd
  have num : a.numerator = b.numerator := by
    unfold Equal at same
    rw [← den] at same
    exact Nat.mul_right_cancel a.positive same
  cases a; cases b
  cases den; cases num
  rfl

def normalize (a : Fraction) : Fraction :=
  ⟨a.numerator / Nat.gcd a.numerator a.denominator,
   a.denominator / Nat.gcd a.numerator a.denominator,
   Nat.div_gcd_pos_of_pos_right a.numerator a.positive⟩

theorem normalize_canonical (a : Fraction) : Canonical (normalize a) :=
  Nat.gcd_div_gcd_div_gcd_of_pos_right a.positive

theorem normalize_exact (a : Fraction) : Equal (normalize a) a := by
  let g := Nat.gcd a.numerator a.denominator
  have n : a.numerator / g * g = a.numerator := Nat.div_mul_cancel (Nat.gcd_dvd_left _ _)
  have d : a.denominator / g * g = a.denominator := Nat.div_mul_cancel (Nat.gcd_dvd_right _ _)
  change (a.numerator / g) * a.denominator = a.numerator * (a.denominator / g)
  calc
    _ = (a.numerator / g) * ((a.denominator / g) * g) := by rw [d]
    _ = ((a.numerator / g) * g) * (a.denominator / g) := by ac_rfl
    _ = _ := by rw [n]

theorem cross_mul_normalized (a b : Fraction) (ha : Canonical a) (hb : Canonical b) :
    crossMul a b = normalize (mul a b) :=
  canonical_unique _ _ (cross_mul_canonical a b ha hb) (normalize_canonical _)
    (eq_trans (cross_mul_exact a b) (eq_symm (normalize_exact _)))

theorem cross_div_normalized (a b : Fraction) (positive : 0 < b.numerator)
    (ha : Canonical a) (hb : Canonical b) :
    crossDiv a b positive = normalize (mul a (inverse b positive)) :=
  cross_mul_normalized a _ ha (inverse_canonical b positive hb)

theorem zero_canonical : Canonical zero := by decide
theorem one_canonical : Canonical one := by decide

/-- The invariant is carried with the accumulator; arbitrary raw fractions
    cannot acquire canonical evidence merely by calling the optimized operation. -/
structure Reduced where
  value : Fraction
  canonical : Canonical value

def Reduced.ofFraction (a : Fraction) : Reduced := ⟨normalize a, normalize_canonical a⟩
def Reduced.one : Reduced := ⟨ExactMass.one, one_canonical⟩
def Reduced.mul (a b : Reduced) : Reduced :=
  ⟨crossMul a.value b.value, cross_mul_canonical _ _ a.canonical b.canonical⟩
def Reduced.div (a b : Reduced) (positive : 0 < b.value.numerator) : Reduced :=
  ⟨crossDiv a.value b.value positive, cross_div_canonical _ _ positive a.canonical b.canonical⟩

def product (start : Reduced) : List Reduced → Reduced
  | [] => start
  | a :: rest => product (start.mul a) rest
def ordinary (start : Fraction) : List Reduced → Fraction
  | [] => start
  | a :: rest => ordinary (mul start a.value) rest

theorem product_exact (start : Reduced) (xs : List Reduced) :
    Equal (product start xs).value (ordinary start.value xs) := by
  have congruent : ∀ xs a b, Equal a b → Equal (ordinary a xs) (ordinary b xs) := by
    intro xs
    induction xs with
    | nil => intro a b same; exact same
    | cons x xs ih => intro a b same; exact ih _ _ (mul_congr_right x.value same)
  induction xs generalizing start with
  | nil => exact eq_refl _
  | cons x xs ih =>
    exact eq_trans (ih (start.mul x)) (congruent xs _ _ (cross_mul_exact start.value x.value))

theorem product_normalized (start : Reduced) (xs : List Reduced) :
    (product start xs).value = normalize (ordinary start.value xs) :=
  canonical_unique _ _ (product start xs).canonical (normalize_canonical _)
    (eq_trans (product_exact start xs) (eq_symm (normalize_exact _)))

end CanonicalProduct
