import VerumExternalReplay.CanonicalProduct

/-! Exact multiplicity aggregation of factors from one frozen model. These
arithmetic laws do not merge observation keys, update a count bank, establish a
native enumeration, or authorize partial publication after failure. -/
namespace ExactPackets
open ExactMass
set_option warningAsError true

def power (q : Fraction) (n : Nat) : Fraction :=
  ⟨q.numerator^n, q.denominator^n, Nat.pow_pos q.positive⟩
def product : List Fraction → Fraction
  | [] => one
  | q :: qs => mul q (product qs)
abbrev Packet := Fraction × Nat
def expand (packets : List Packet) : List Fraction :=
  packets.flatMap (fun p => List.replicate p.2 p.1)
def aggregate (packets : List Packet) : Fraction :=
  product (packets.map (fun p => power p.1 p.2))

theorem power_positive (q : Fraction) (positive : 0 < q.numerator) (n : Nat) :
    0 < (power q n).numerator := Nat.pow_pos positive

theorem product_append (xs ys : List Fraction) :
    product (xs ++ ys) = mul (product xs) (product ys) := by
  induction xs with
  | nil => simp [product, mul, one]
  | cons x xs ih => simp only [List.cons_append, product, ih, mul_assoc_exact]

theorem product_replicate (q : Fraction) (n : Nat) :
    product (List.replicate n q) = power q n := by
  induction n with
  | zero => rfl
  | succ n ih =>
    simp only [List.replicate_succ, product, ih, power, mul, Nat.pow_succ]
    congr 1 <;> ac_rfl

theorem aggregate_exact (packets : List Packet) :
    aggregate packets = product (expand packets) := by
  induction packets with
  | nil => rfl
  | cons p ps ih =>
    simp only [aggregate, List.map_cons, product] at *
    simp only [expand, List.flatMap_cons, product_append, product_replicate]
    rw [ih]
    rfl

theorem product_permutation (xs ys : List Fraction) (same : xs.Perm ys) :
    product xs = product ys := by
  induction same with
  | nil => rfl
  | cons x _ ih => simp only [product, ih]
  | swap x y xs => simp only [product, mul]; congr 1 <;> ac_rfl
  | trans _ _ ih₁ ih₂ => exact ih₁.trans ih₂

/-- The producer must establish this full multiplicity correspondence. Equal
final probabilities alone do not establish complete factor enumeration. -/
theorem regroup_exact (observations : List Fraction) (packets : List Packet)
    (complete : (expand packets).Perm observations) :
    aggregate packets = product observations :=
  (aggregate_exact packets).trans (product_permutation _ _ complete)

theorem power_add (q : Fraction) (m n : Nat) :
    power q (m+n) = mul (power q m) (power q n) := by
  simp [power, mul, Nat.pow_add]

theorem power_congr (a b : Fraction) (same : Equal a b) (n : Nat) :
    Equal (power a n) (power b n) := by
  change a.numerator^n * b.denominator^n = b.numerator^n * a.denominator^n
  rw [← Nat.mul_pow, ← Nat.mul_pow, same]

def quotient (a b : Fraction) (positive : 0 < b.numerator) : Fraction :=
  mul a (CanonicalProduct.inverse b positive)

theorem cancel_common (a b common : Fraction)
    (hb : 0 < b.numerator) (hc : 0 < common.numerator) :
    Equal (quotient (mul a common) (mul b common) (Nat.mul_pos hb hc)) (quotient a b hb) := by
  simp only [quotient, CanonicalProduct.inverse, Equal, mul]
  ac_rfl

theorem cancel_powers (q : Fraction) (positive : 0 < q.numerator) (m n k : Nat) :
    Equal (quotient (power q (m+k)) (power q (n+k)) (power_positive q positive _))
      (quotient (power q m) (power q n) (power_positive q positive _)) := by
  simpa only [power_add] using cancel_common (power q m) (power q n) (power q k)
    (power_positive q positive n) (power_positive q positive k)

/-- Remove exactly the common exponent. Absolute masses remain separate;
this theorem concerns a quotient with a strictly positive denominator. -/
theorem cancel_minimum (q : Fraction) (positive : 0 < q.numerator) (m n : Nat) :
    Equal (quotient (power q m) (power q n) (power_positive q positive _))
      (quotient (power q (m - min m n)) (power q (n - min m n)) (power_positive q positive _)) := by
  simpa only [Nat.sub_add_cancel (Nat.min_le_left m n),
    Nat.sub_add_cancel (Nat.min_le_right m n)] using
    cancel_powers q positive (m - min m n) (n - min m n) (min m n)

end ExactPackets
