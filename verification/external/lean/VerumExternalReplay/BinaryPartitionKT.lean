import VerumExternalReplay.ExactMass

/-! Fixed binary KT probabilities for a finite partition of counted rows.
The bound counts both outcomes. It is neither an anytime guarantee nor a claim
about a native acquisition prefix whose actual census has not been exported. -/
namespace BinaryPartitionKT
open ExactMass
set_option warningAsError true

def factorial : Nat → Nat
  | 0 => 1
  | n+1 => (n+1)*factorial n

def oddProduct : Nat → Nat
  | 0 => 1
  | n+1 => oddProduct n*(2*n+1)

theorem factorial_positive (n : Nat) : 0 < factorial n := by
  induction n with
  | zero => decide
  | succ n ih => exact Nat.mul_pos (by omega) ih

theorem oddProduct_positive (n : Nat) : 0 < oddProduct n := by
  induction n with
  | zero => decide
  | succ n ih => exact Nat.mul_pos ih (by omega)

def kt (a b : Nat) : Fraction :=
  ⟨oddProduct a * oddProduct b, 2^(a+b)*factorial (a+b),
    Nat.mul_pos (Nat.pow_pos (by decide)) (factorial_positive _)⟩

def pure (n : Nat) : Fraction := kt n 0

theorem kt_positive (a b : Nat) : 0 < (kt a b).numerator :=
  Nat.mul_pos (oddProduct_positive a) (oddProduct_positive b)

theorem kt_empty : kt 0 0 = one := rfl

theorem kt_symmetric (a b : Nat) : kt a b = kt b a := by
  simp [kt, Nat.add_comm, Nat.mul_comm]

/-- The closed form follows the fixed binary KT one-step prediction rule. -/
theorem kt_successor (a b : Nat) :
    kt (a+1) b = mul (kt a b) ⟨2*a+1, 2*(a+b)+2, by omega⟩ := by
  simp only [kt, mul, oddProduct, show a+1+b=(a+b)+1 by omega, Nat.pow_succ, factorial]
  simp [Nat.mul_add, Nat.add_mul, Nat.mul_assoc, Nat.mul_comm, Nat.mul_left_comm]

theorem kt_negative_successor (a b : Nat) :
    kt a (b+1) = mul (kt a b) ⟨2*b+1, 2*(a+b)+2, by omega⟩ := by
  rw [kt_symmetric a (b+1), kt_successor b a, kt_symmetric b a]
  simp [Nat.add_comm]

def factor (n : Nat) : Fraction := ⟨2*n+1,2*n+2,by omega⟩

/-- Factorial ratio representing C(a+b,a), without rounding or floating point. -/
def binomial (a b : Nat) : Fraction :=
  ⟨factorial (a+b),factorial a * factorial b,
    Nat.mul_pos (factorial_positive _) (factorial_positive _)⟩

theorem pure_zero : pure 0 = one := rfl

theorem pure_successor (n : Nat) : pure (n+1) = mul (pure n) (factor n) := by
  simp [pure,kt,oddProduct,factorial,factor,mul,Nat.pow_succ,Nat.mul_add,Nat.add_mul,
    Nat.mul_assoc,Nat.mul_comm,Nat.mul_left_comm]

theorem factor_monotone (a b : Nat) (ordered : a ≤ b) : AtMost (factor a) (factor b) := by
  simp only [AtMost,factor,Nat.mul_add,Nat.add_mul,Nat.one_mul]
  simp only [Nat.mul_comm,Nat.mul_left_comm]
  omega

theorem pure_supermultiplicative (a b : Nat) : AtMost (mul (pure a) (pure b)) (pure (a+b)) := by
  induction b with
  | zero => simpa only [Nat.add_zero,pure_zero] using eq_le (mul_one (pure a))
  | succ b ih =>
    rw [show a+(b+1)=(a+b)+1 by omega, pure_successor (a+b), pure_successor b]
    rw [← mul_assoc_exact]
    exact le_trans (mul_mono_right _ ih)
      (mul_mono_left _ (factor_monotone b (a+b) (by omega)))

theorem factorial_supermultiplicative (a b : Nat) : factorial a * factorial b ≤ factorial (a+b) := by
  induction b with
  | zero => simp [factorial]
  | succ b ih =>
    calc
      factorial a * factorial (b+1) = (b+1)*(factorial a * factorial b) := by
        simp [factorial,Nat.mul_assoc,Nat.mul_comm]
      _ ≤ (a+b+1)*factorial (a+b) := Nat.mul_le_mul (by omega) ih
      _ = factorial (a+(b+1)) := by
        rw [show a+(b+1)=(a+b)+1 by omega]; rfl

theorem split_mixed_cell (a b : Nat) : AtMost (kt a b) (mul (pure a) (pure b)) := by
  have bound := Nat.mul_le_mul_left (oddProduct a * oddProduct b * 2^(a+b))
    (factorial_supermultiplicative a b)
  simpa [AtMost,kt,pure,mul,oddProduct,Nat.pow_add,Nat.mul_assoc,Nat.mul_comm,Nat.mul_left_comm] using bound

theorem pure_partition_ratio (a b : Nat) :
    Equal (mul (pure a) (pure b)) (mul (binomial a b) (kt a b)) := by
  simp [Equal,pure,kt,binomial,mul,oddProduct,Nat.pow_add,
    Nat.mul_assoc,Nat.mul_comm,Nat.mul_left_comm]

def totals : List (Nat × Nat) → Nat × Nat
  | [] => (0,0)
  | (a,b)::tail => (a+(totals tail).1,b+(totals tail).2)

def partition : List (Nat × Nat) → Fraction
  | [] => one
  | (a,b)::tail => mul (kt a b) (partition tail)

theorem partition_below_pure (cells : List (Nat × Nat)) :
    AtMost (partition cells) (mul (pure (totals cells).1) (pure (totals cells).2)) := by
  induction cells with
  | nil => exact eq_le (eq_symm (mul_one one))
  | cons cell tail ih =>
    rcases cell with ⟨a,b⟩
    apply le_trans (mul_mono_right _ (split_mixed_cell a b))
    apply le_trans (mul_mono_left _ ih)
    have rearrange : mul (mul (pure a) (pure b))
        (mul (pure (totals tail).1) (pure (totals tail).2)) =
        mul (mul (pure a) (pure (totals tail).1)) (mul (pure b) (pure (totals tail).2)) := by
      simp [mul,Nat.mul_comm,Nat.mul_left_comm]
    rw [rearrange]
    exact le_trans (mul_mono_right _ (pure_supermultiplicative a _))
      (mul_mono_left _ (pure_supermultiplicative b _))

/-- Q_child / Q_parent ≤ C(n,k), expressed without an unproved division rule. -/
theorem partition_binomial_bound (cells : List (Nat × Nat)) :
    AtMost (partition cells)
      (mul (binomial (totals cells).1 (totals cells).2) (kt (totals cells).1 (totals cells).2)) :=
  le_trans (partition_below_pure cells) (eq_le (pure_partition_ratio _ _))

def ratio (cells : List (Nat × Nat)) : Fraction :=
  divide (partition cells) (kt (totals cells).1 (totals cells).2) (kt_positive _ _)

theorem ratio_binomial_bound (cells : List (Nat × Nat)) :
    AtMost (ratio cells) (binomial (totals cells).1 (totals cells).2) :=
  (divide_atMost_iff _ _ _ _).mpr (partition_binomial_bound cells)

/-- Pure cells attain the bound, so a smaller uniform bound is impossible. -/
theorem pure_partition_attains_bound (a b : Nat) :
    Equal (ratio [(a,0),(0,b)]) (binomial a b) := by
  simp [Equal, ratio, partition, totals, kt, binomial, oddProduct,
    divide, mul, one, Nat.pow_add, Nat.mul_assoc, Nat.mul_comm, Nat.mul_left_comm]

end BinaryPartitionKT
