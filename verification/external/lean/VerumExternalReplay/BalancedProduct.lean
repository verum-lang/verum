import Lean

/-! Finite product maximization for a positive sequence with decreasing adjacent
ratios. The hypotheses are local algebraic properties, not an assumed maximum.
A probability application must prove those properties for its own factors. -/
namespace BalancedProduct
set_option warningAsError true

def product (w : Nat → Rat) : List Nat → Rat
  | [] => 1
  | n :: ns => w n * product w ns

def ratio (w : Nat → Rat) (q : Nat) : Rat := w (q+1) / w q

theorem pow_add (a : Rat) (m n : Nat) : a^(m+n) = a^m*a^n := by
  induction n with
  | zero => simp
  | succ n ih => rw [Nat.add_succ, Rat.pow_succ, ih, Rat.pow_succ]; grind

variable (w : Nat → Rat) (positive : ∀ n, 0 < w n)
variable (slope : ∀ a b, a ≤ b → w (b+1)*w a ≤ w b*w (a+1))
include positive

theorem ratio_pos (q : Nat) : 0 < ratio w q := by
  have h := Rat.mul_pos (positive (q+1)) (Rat.inv_pos.mpr (positive q))
  exact h

include slope
theorem lower_growth (q m d : Nat) (h : m+d ≤ q) :
    w m * (ratio w q)^d ≤ w (m+d) := by
  induction d with
  | zero => simp
  | succ d ih =>
    have hprev := ih (by omega)
    have hs := slope (m+d) q (by omega)
    have hq := positive q
    have ht := ratio_pos w positive q
    have hstep : w (m+d)*ratio w q ≤ w (m+d+1) := by
      apply Rat.le_of_mul_le_mul_right (c := w q) ?_ hq
      simp only [ratio, Rat.mul_assoc, Rat.div_mul_cancel (Rat.ne_of_gt hq)]
      simpa only [Rat.mul_comm] using hs
    have hm := Rat.mul_le_mul_of_nonneg_right hprev (Rat.le_of_lt ht)
    rw [Rat.pow_succ]
    have hc := Rat.le_trans hm hstep
    simpa only [Nat.add_succ, Rat.mul_assoc] using hc

theorem upper_growth (q d : Nat) :
    w (q+d) ≤ w q * (ratio w q)^d := by
  induction d with
  | zero => simp
  | succ d ih =>
    have hs := slope q (q+d) (by omega)
    have hq := positive q
    have ht := ratio_pos w positive q
    have hstep : w (q+d+1) ≤ w (q+d)*ratio w q := by
      apply Rat.le_of_mul_le_mul_right (c := w q) ?_ hq
      simp only [ratio, Rat.mul_assoc, Rat.div_mul_cancel (Rat.ne_of_gt hq)]
      simpa only [Rat.mul_comm] using hs
    have hm := Rat.mul_le_mul_of_nonneg_right ih (Rat.le_of_lt ht)
    have hc := Rat.le_trans hstep hm
    rw [Rat.pow_succ]
    simpa only [Nat.add_succ, Rat.mul_assoc] using hc

/-- A supporting geometric sequence bounds every factor, on either side of q. -/
theorem envelope (q m : Nat) :
    w m * (ratio w q)^q ≤ w q * (ratio w q)^m := by
  by_cases h : m ≤ q
  · have hg := lower_growth w positive slope q m (q-m) (by omega)
    have hp := Rat.pow_pos (n := m) (ratio_pos w positive q)
    have hm := Rat.mul_le_mul_of_nonneg_right hg (Rat.le_of_lt hp)
    have he : m + (q-m) = q := by omega
    rw [he] at hm
    have he' : (q-m)+m = q := by omega
    rw [Rat.mul_assoc, ← pow_add, he'] at hm
    exact hm
  · have hg := upper_growth w positive slope q (m-q)
    have hp := Rat.pow_pos (n := q) (ratio_pos w positive q)
    have hm := Rat.mul_le_mul_of_nonneg_right hg (Rat.le_of_lt hp)
    have he : q + (m-q) = m := by omega
    rw [he] at hm
    have he' : (m-q)+q = m := by omega
    rw [Rat.mul_assoc, ← pow_add, he'] at hm
    exact hm

omit slope in
theorem product_pos (ns : List Nat) : 0 < product w ns := by
  induction ns with
  | nil => exact (by decide : (0 : Rat) < 1)
  | cons n ns ih => exact Rat.mul_pos (positive n) ih

theorem product_envelope (q : Nat) (ns : List Nat) :
    product w ns * (ratio w q)^(q*ns.length) ≤
      (w q)^ns.length * (ratio w q)^ns.sum := by
  induction ns with
  | nil => simp [product]
  | cons n ns ih =>
    have he := envelope w positive slope q n
    have hl := Rat.mul_nonneg (Rat.le_of_lt (product_pos w positive ns))
      (Rat.le_of_lt (Rat.pow_pos (n := q*ns.length) (ratio_pos w positive q)))
    have hr := Rat.mul_nonneg (Rat.le_of_lt (positive q))
      (Rat.le_of_lt (Rat.pow_pos (n := n) (ratio_pos w positive q)))
    have h1 := Rat.mul_le_mul_of_nonneg_right he hl
    have h2 := Rat.mul_le_mul_of_nonneg_left ih hr
    have hc := Rat.le_trans h1 h2
    simp only [product, List.length_cons, List.sum_cons, Nat.mul_add, Nat.mul_one,
      pow_add, Rat.pow_succ] at *
    grind only

/-- Closed balanced product, using q=floor(n/K), r=n mod K. -/
def bound (w : Nat → Rat) (K n : Nat) : Rat :=
  (w (n/K))^K * (ratio w (n/K))^(n%K)

/-- Global product maximum follows from local adjacent-ratio inequalities. -/
theorem maximum (ns : List Nat) :
    product w ns ≤ bound w ns.length ns.sum := by
  let q := ns.sum / ns.length
  let r := ns.sum % ns.length
  have he : ns.sum = q*ns.length+r := by
    have h := Nat.div_add_mod ns.sum ns.length
    simpa only [Nat.mul_comm] using h.symm
  have hp := product_envelope w positive slope q ns
  rw [he, pow_add] at hp
  have hpos := Rat.pow_pos (n := q*ns.length) (ratio_pos w positive q)
  apply Rat.le_of_mul_le_mul_right (c := (ratio w q)^(q*ns.length)) ?_ hpos
  change product w ns * (ratio w q)^(q*ns.length) ≤
    ((w q)^ns.length * (ratio w q)^r) * (ratio w q)^(q*ns.length)
  simpa only [Rat.mul_assoc, Rat.mul_comm] using hp

omit positive slope in
theorem product_append (xs ys : List Nat) :
    product w (xs ++ ys) = product w xs * product w ys := by
  induction xs with
  | nil => simp [product]
  | cons x xs ih => simp only [List.cons_append, product, ih, Rat.mul_assoc]

omit positive slope in
theorem product_replicate (k n : Nat) : product w (List.replicate k n) = (w n)^k := by
  induction k with
  | zero => simp [product]
  | succ k ih => simp only [List.replicate_succ, product, ih, Rat.pow_succ, Rat.mul_comm]

omit positive slope in
theorem pow_mul (a b : Rat) (n : Nat) : (a*b)^n = a^n*b^n := by
  induction n with
  | zero => simp
  | succ n ih => simp only [Rat.pow_succ, ih]; grind

def balancedCounts (K n : Nat) : List Nat :=
  List.replicate (n%K) (n/K+1) ++ List.replicate (K-n%K) (n/K)

omit positive slope in
theorem balanced_length (K n : Nat) (hK : 0 < K) :
    (balancedCounts K n).length = K := by
  have hr := Nat.mod_lt n hK
  simp only [balancedCounts, List.length_append, List.length_replicate]
  omega

omit positive slope in
theorem balanced_sum (K n : Nat) (hK : 0 < K) :
    (balancedCounts K n).sum = n := by
  have hr := Nat.mod_lt n hK
  have hk : n%K + (K-n%K) = K := by omega
  have hn := Nat.div_add_mod n K
  simp only [balancedCounts, List.sum_append, List.sum_replicate_nat]
  grind only

omit slope in
/-- The upper bound is attained by counts differing by at most one. -/
theorem balanced_attains (K n : Nat) (hK : 0 < K) :
    product w (balancedCounts K n) = bound w K n := by
  let q := n/K
  let r := n%K
  have hr : r < K := Nat.mod_lt n hK
  have hk : r + (K-r) = K := by omega
  have hc : w (q+1) = w q * ratio w q := by
    unfold ratio
    rw [Rat.mul_comm, Rat.div_mul_cancel (Rat.ne_of_gt (positive q))]
  change product w (List.replicate r (q+1) ++ List.replicate (K-r) q) =
    (w q)^K * (ratio w q)^r
  rw [product_append, product_replicate, product_replicate, hc, pow_mul]
  have hp := pow_add (w q) r (K-r)
  rw [hk] at hp
  grind only

end BalancedProduct
