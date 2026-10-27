import Lean

/- Nonnegative rational model using positive natural denominators. Comparisons
   use exact cross multiplication; normalization is unnecessary for these laws.
   This is proof data, not a replacement for a native arbitrary-precision library. -/
namespace ExactMass
set_option warningAsError true

structure Fraction where
  numerator : Nat
  denominator : Nat
  positive : 0 < denominator
  deriving DecidableEq

def zero : Fraction := ⟨0, 1, by decide⟩
def one : Fraction := ⟨1, 1, by decide⟩
def unit (denominator : Nat) (positive : 0 < denominator) : Fraction :=
  ⟨1, denominator, positive⟩
def Equal (a b : Fraction) : Prop := a.numerator * b.denominator = b.numerator * a.denominator
def AtMost (a b : Fraction) : Prop := a.numerator * b.denominator ≤ b.numerator * a.denominator
def Less (a b : Fraction) : Prop := a.numerator * b.denominator < b.numerator * a.denominator
instance (a b : Fraction) : Decidable (Equal a b) := inferInstanceAs (Decidable (_ = _))
instance (a b : Fraction) : Decidable (AtMost a b) := inferInstanceAs (Decidable (_ ≤ _))
instance (a b : Fraction) : Decidable (Less a b) := inferInstanceAs (Decidable (_ < _))
def add (a b : Fraction) : Fraction :=
  ⟨a.numerator * b.denominator + b.numerator * a.denominator,
    a.denominator * b.denominator, Nat.mul_pos a.positive b.positive⟩
def mul (a b : Fraction) : Fraction :=
  ⟨a.numerator * b.numerator, a.denominator * b.denominator, Nat.mul_pos a.positive b.positive⟩

theorem le_refl (a : Fraction) : AtMost a a := Nat.le_refl _
theorem eq_refl (a : Fraction) : Equal a a := rfl
theorem eq_symm {a b : Fraction} (h : Equal a b) : Equal b a := h.symm
theorem eq_le {a b : Fraction} (h : Equal a b) : AtMost a b := Nat.le_of_eq h
theorem le_antisymm {a b : Fraction} (h : AtMost a b) (k : AtMost b a) : Equal a b :=
  Nat.le_antisymm h k
theorem zero_le (a : Fraction) : AtMost zero a := by simp [AtMost, zero]
theorem less_le {a b : Fraction} (h : Less a b) : AtMost a b := Nat.le_of_lt h

theorem le_trans {a b c : Fraction} (h : AtMost a b) (k : AtMost b c) : AtMost a c := by
  apply Nat.le_of_mul_le_mul_right (c := b.denominator) _ b.positive
  have h' := Nat.mul_le_mul_right c.denominator h
  have k' := Nat.mul_le_mul_right a.denominator k
  simp only [Nat.mul_comm, Nat.mul_left_comm] at h' k' ⊢
  exact Nat.le_trans h' k'

theorem eq_trans {a b c : Fraction} (h : Equal a b) (k : Equal b c) : Equal a c :=
  le_antisymm (le_trans (eq_le h) (eq_le k)) (le_trans (eq_le (eq_symm k)) (eq_le (eq_symm h)))

theorem add_comm (a b : Fraction) : Equal (add a b) (add b a) := by
  simp [Equal, add, Nat.mul_comm, Nat.add_comm]

theorem add_assoc (a b c : Fraction) : Equal (add (add a b) c) (add a (add b c)) := by
  simp [Equal, add, Nat.mul_add, Nat.mul_comm, Nat.mul_left_comm, Nat.add_assoc, Nat.add_comm]

theorem add_zero (a : Fraction) : Equal (add a zero) a := by simp [Equal, add, zero]

theorem add_mono_left (c : Fraction) {a b : Fraction} (h : AtMost a b) : AtMost (add c a) (add c b) := by
  have scaled := Nat.mul_le_mul_right (c.denominator * c.denominator) h
  have result := Nat.add_le_add_left scaled (c.numerator * a.denominator * c.denominator * b.denominator)
  simpa only [AtMost, add, Nat.add_mul, Nat.mul_add, Nat.mul_assoc,
    Nat.mul_comm, Nat.mul_left_comm] using result

theorem add_mono_right (c : Fraction) {a b : Fraction} (h : AtMost a b) : AtMost (add a c) (add b c) :=
  le_trans (eq_le (add_comm a c)) (le_trans (add_mono_left c h) (eq_le (add_comm c b)))

theorem add_congr_left (c : Fraction) {a b : Fraction} (h : Equal a b) : Equal (add c a) (add c b) :=
  le_antisymm (add_mono_left c (eq_le h)) (add_mono_left c (eq_le (eq_symm h)))

theorem add_congr_right (c : Fraction) {a b : Fraction} (h : Equal a b) : Equal (add a c) (add b c) :=
  le_antisymm (add_mono_right c (eq_le h)) (add_mono_right c (eq_le (eq_symm h)))

theorem le_add (a b : Fraction) : AtMost a (add a b) :=
  le_trans (eq_le (eq_symm (add_zero a))) (add_mono_left a (zero_le b))

theorem mul_comm (a b : Fraction) : Equal (mul a b) (mul b a) := by simp [Equal, mul, Nat.mul_comm]
theorem mul_one (a : Fraction) : Equal (mul a one) a := by simp [Equal, mul, one]
theorem mul_assoc_exact (a b c : Fraction) : mul (mul a b) c = mul a (mul b c) := by
  simp [mul, Nat.mul_assoc]
theorem mul_zero (a : Fraction) : Equal (mul a zero) zero := by simp [Equal, mul, zero]

theorem mul_add (a b c : Fraction) : Equal (mul a (add b c)) (add (mul a b) (mul a c)) := by
  simp [Equal, add, mul, Nat.mul_add, Nat.mul_comm, Nat.mul_left_comm]

theorem mul_mono_right (c : Fraction) {a b : Fraction} (h : AtMost a b) : AtMost (mul a c) (mul b c) := by
  have result := Nat.mul_le_mul_right (c.numerator * c.denominator) h
  simpa only [AtMost, mul, Nat.mul_assoc, Nat.mul_comm, Nat.mul_left_comm] using result

theorem mul_congr_right (c : Fraction) {a b : Fraction} (h : Equal a b) : Equal (mul a c) (mul b c) :=
  le_antisymm (mul_mono_right c (eq_le h)) (mul_mono_right c (eq_le (eq_symm h)))

theorem mul_congr_left (c : Fraction) {a b : Fraction} (h : Equal a b) : Equal (mul c a) (mul c b) :=
  eq_trans (mul_comm c a) (eq_trans (mul_congr_right c h) (mul_comm b c))

theorem mul_mono_left (c : Fraction) {a b : Fraction} (h : AtMost a b) : AtMost (mul c a) (mul c b) :=
  le_trans (eq_le (mul_comm c a)) (le_trans (mul_mono_right c h) (eq_le (mul_comm b c)))

def sum : List Fraction → Fraction
  | [] => zero
  | a :: tail => add a (sum tail)

theorem sum_congr (xs : List α) (f g : α → Fraction) (h : ∀ x ∈ xs, Equal (f x) (g x)) :
    Equal (sum (xs.map f)) (sum (xs.map g)) := by
  induction xs with
  | nil => exact eq_refl _
  | cons x xs ih =>
    exact eq_trans (add_congr_right _ (h x (by simp)))
      (add_congr_left _ (ih (fun y hy => h y (by simp [hy]))))

theorem sum_mono (xs : List α) (f g : α → Fraction) (h : ∀ x ∈ xs, AtMost (f x) (g x)) :
    AtMost (sum (xs.map f)) (sum (xs.map g)) := by
  induction xs with
  | nil => exact le_refl _
  | cons x xs ih =>
    exact le_trans (add_mono_right _ (h x (by simp)))
      (add_mono_left _ (ih (fun y hy => h y (by simp [hy]))))

theorem sum_scale (xs : List α) (f : α → Fraction) (c : Fraction) :
    Equal (sum (xs.map (fun x => mul c (f x)))) (mul c (sum (xs.map f))) := by
  induction xs with
  | nil => exact eq_symm (mul_zero _)
  | cons x xs ih => exact eq_trans (add_congr_left _ ih) (eq_symm (mul_add _ _ _))

theorem sum_add (xs : List α) (f g : α → Fraction) :
    Equal (sum (xs.map (fun x => add (f x) (g x))))
      (add (sum (xs.map f)) (sum (xs.map g))) := by
  induction xs with
  | nil => exact eq_symm (add_zero _)
  | cons x xs ih =>
    apply eq_trans (add_congr_left _ ih)
    apply eq_trans (add_assoc _ _ _)
    apply eq_trans (add_congr_left _ (eq_symm (add_assoc _ _ _)))
    apply eq_trans (add_congr_left _ (add_congr_right _ (add_comm _ _)))
    apply eq_trans (add_congr_left _ (add_assoc _ _ _))
    exact eq_symm (add_assoc _ _ _)

theorem sum_common_denominator (ns : List Nat) (d : Nat) (hd : 0 < d) :
    Equal (sum (ns.map (fun n => ⟨n, d, hd⟩))) ⟨ns.sum, d, hd⟩ := by
  induction ns with
  | nil => simp [sum, Equal, zero]
  | cons n ns ih =>
    apply eq_trans (add_congr_left _ ih)
    simp [Equal, add, Nat.add_mul, Nat.mul_assoc]

def divide (a b : Fraction) (hb : 0 < b.numerator) : Fraction :=
  ⟨a.numerator * b.denominator, a.denominator * b.numerator, Nat.mul_pos a.positive hb⟩

-- Exact threshold comparison: no division, logarithm or floating point needed.
theorem ratio_threshold (q p weight : Fraction) (hp : 0 < p.numerator)
    (hw : 0 < weight.numerator) :
    Less (divide one weight hw) (divide q p hp) ↔ Less p (mul weight q) := by
  simp only [Less, divide, one, mul, Nat.one_mul]
  constructor <;> intro h
  · simpa only [Nat.mul_assoc, Nat.mul_comm, Nat.mul_left_comm] using h
  · simpa only [Nat.mul_assoc, Nat.mul_comm, Nat.mul_left_comm] using h

end ExactMass
