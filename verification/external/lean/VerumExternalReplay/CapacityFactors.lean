import VerumExternalReplay.RefinementFeasibility

/-! Exact KT capacity weights and the scalar transfer inequality. -/
namespace RefinementFeasibility
set_option warningAsError true

private theorem mul_left_comm (a b c : Rat) : a*(b*c) = b*(a*c) := by
  rw [← Rat.mul_assoc, ← Rat.mul_assoc, Rat.mul_comm a b]

def rising (a : Rat) : Nat → Rat
  | 0 => 1
  | n + 1 => rising a n * (a + (n : Rat))

def weight (K n : Nat) : Rat :=
  rising (1/2) n / (rising ((K : Rat)/2) n * (n : Rat)^n)

@[simp] theorem rising_zero (a : Rat) : rising a 0 = 1 := rfl
@[simp] theorem rising_succ (a : Rat) (n : Nat) :
    rising a (n+1) = rising a n * (a + (n : Rat)) := rfl

theorem rising_pos {a : Rat} (ha : 0 < a) (n : Nat) : 0 < rising a n := by
  induction n with
  | zero => change (0 : Rat) < 1; decide
  | succ n ih =>
    have hn := Rat.natCast_nonneg (a := n)
    exact Rat.mul_pos ih (by grind)

theorem self_power_pos (n : Nat) : 0 < (n : Rat)^n := by
  cases n with
  | zero => simp only [Rat.pow_zero]; decide
  | succ n =>
    apply Rat.pow_pos
    exact Rat.natCast_pos.mpr (by omega)

@[simp] theorem weight_zero (K : Nat) : weight K 0 = 1 := by
  simp only [weight, rising_zero, Rat.pow_zero]
  grind

theorem weight_pos {K : Nat} (hK : 0 < K) (n : Nat) : 0 < weight K n := by
  have hk : 0 < (K : Rat) := Rat.natCast_pos.mpr hK
  have hhalf : (0 : Rat) < 1/2 := by grind
  have hkhalf : 0 < (K : Rat)/2 := by grind
  exact Rat.mul_pos (rising_pos hhalf n)
    (Rat.inv_pos.mpr (Rat.mul_pos (rising_pos hkhalf n) (self_power_pos n)))

private theorem normalized_power_identity (m : Rat) (hm : m ≠ 0) (n : Nat) :
    (1+1/m)^2*(1-1/m^2)^n*(m^(n+1)*m^(n+1)) =
      (m+1)^(n+2)*(m-1)^n := by
  induction n with
  | zero => simp only [Rat.pow_zero, Rat.pow_succ]; grind
  | succ n ih =>
    have he : (1-1/m^2)*m*m = (m+1)*(m-1) := by grind
    rw [Rat.pow_succ (1-1/m^2) n, Rat.pow_succ m (n+1),
      Rat.pow_succ (m+1) (n+2), Rat.pow_succ (m-1) n]
    calc
      _ = ((1+1/m)^2*(1-1/m^2)^n*(m^(n+1)*m^(n+1))) *
          ((1-1/m^2)*m*m) := by simp only [Rat.mul_assoc, Rat.mul_comm, mul_left_comm]
      _ = ((m+1)^(n+2)*(m-1)^n) * ((m+1)*(m-1)) := by rw [ih, he]
      _ = _ := by simp only [Rat.mul_assoc, Rat.mul_comm, mul_left_comm]

theorem power_gap (n : Nat) :
    (2*(n : Rat)+3) * (((n : Rat)+1)^(n+1)*((n : Rat)+1)^(n+1)) <
      (2*(n : Rat)+1) * (((n : Rat)+2)^(n+2)*(n : Rat)^n) := by
  have hn := Rat.natCast_nonneg (a := n)
  have hm : 0 < (n : Rat)+1 := by grind
  have hp := Rat.mul_pos (Rat.pow_pos (n := n+1) hm) (Rat.pow_pos (n := n+1) hm)
  have hg := Rat.mul_lt_mul_of_pos_right (normalized_gap n) hp
  have he := normalized_power_identity ((n : Rat)+1) (Rat.ne_of_gt hm) n
  have hs : (n : Rat)+1-1 = n := by grind
  have ht : (n : Rat)+1+1 = (n : Rat)+2 := by grind
  rw [hs, ht] at he
  have hrearrange :
      (2*((n : Rat)+1)-1)*(1+1/((n : Rat)+1))^2*(1-1/((n : Rat)+1)^2)^n *
        (((n : Rat)+1)^(n+1)*((n : Rat)+1)^(n+1)) =
      (2*((n : Rat)+1)-1) * (((n : Rat)+2)^(n+2)*(n : Rat)^n) := by
    rw [← he]
    simp only [Rat.mul_assoc]
  rw [hrearrange] at hg
  have hl : 2*((n : Rat)+1)+1 = 2*(n : Rat)+3 := by grind
  have hr : 2*((n : Rat)+1)-1 = 2*(n : Rat)+1 := by grind
  simpa only [hl, hr] using hg

private theorem div_lt_div_of_cross {a b c d : Rat} (hb : 0 < b) (hd : 0 < d)
    (h : a*d < c*b) : a/b < c/d := by
  rw [← Rat.mul_lt_mul_right (Rat.mul_pos hb hd)]
  have hl : a/b*(b*d) = a*d := by
    rw [← Rat.mul_assoc, Rat.div_mul_cancel (Rat.ne_of_gt hb)]
  have hr : c/d*(b*d) = c*b := by
    rw [Rat.mul_comm b d, ← Rat.mul_assoc, Rat.div_mul_cancel (Rat.ne_of_gt hd)]
  simpa only [hl, hr] using h

def baseStep (n : Nat) : Rat :=
  (2*(n : Rat)+1)*(n : Rat)^n / ((n : Rat)+1)^(n+1)

def weightStep (K n : Nat) : Rat := baseStep n / (2*(n : Rat)+(K : Rat))

theorem baseStep_pos (n : Nat) : 0 < baseStep n := by
  have hn := Rat.natCast_nonneg (a := n)
  exact Rat.mul_pos (Rat.mul_pos (by grind) (self_power_pos n))
    (Rat.inv_pos.mpr (Rat.pow_pos (by grind)))

theorem baseStep_next_lt (n : Nat) : baseStep (n+1) < baseStep n := by
  have hn := Rat.natCast_nonneg (a := n)
  unfold baseStep
  apply div_lt_div_of_cross (Rat.pow_pos (by grind)) (Rat.pow_pos (by grind))
  have h1 : ((n+1 : Nat) : Rat) = (n : Rat)+1 := by simp
  have h2 : 2*((n : Rat)+1)+1 = 2*(n : Rat)+3 := by grind
  have h3 : (n : Rat)+1+1 = (n : Rat)+2 := by grind
  rw [h1, h2, h3]
  simpa only [Rat.mul_assoc, Rat.mul_comm, mul_left_comm] using power_gap n

theorem weightStep_pos {K : Nat} (hK : 0 < K) (n : Nat) : 0 < weightStep K n := by
  have hn := Rat.natCast_nonneg (a := n)
  have hk : 0 < (K : Rat) := Rat.natCast_pos.mpr hK
  exact Rat.mul_pos (baseStep_pos n) (Rat.inv_pos.mpr (by grind))

theorem weightStep_next_lt {K : Nat} (hK : 0 < K) (n : Nat) :
    weightStep K (n+1) < weightStep K n := by
  have hn := Rat.natCast_nonneg (a := n)
  have hk : 0 < (K : Rat) := Rat.natCast_pos.mpr hK
  have hd : 0 < 2*(n : Rat)+(K : Rat) := by grind
  have hds : 0 < 2*((n+1 : Nat) : Rat)+(K : Rat) := by grind
  unfold weightStep
  apply div_lt_div_of_cross hds hd
  have hl := Rat.mul_lt_mul_of_pos_right (baseStep_next_lt n) hd
  have hr := Rat.mul_le_mul_of_nonneg_left
    (show 2*(n : Rat)+(K : Rat) ≤ 2*((n+1 : Nat) : Rat)+(K : Rat) by grind)
    (Rat.le_of_lt (baseStep_pos n))
  grind only

private theorem quotient_step (r s p q t u : Rat)
    (hs : s ≠ 0) (hp : p ≠ 0) (hq : q ≠ 0) (hu : u ≠ 0) :
    (r*t)/(s*u*q) = r/(s*p) * ((2*t)*p/q/(2*u)) := by
  grind

theorem weight_succ {K : Nat} (hK : 0 < K) (n : Nat) :
    weight K (n+1) = weight K n * weightStep K n := by
  have hn := Rat.natCast_nonneg (a := n)
  have hk : 0 < (K : Rat) := Rat.natCast_pos.mpr hK
  have hkhalf : 0 < (K : Rat)/2 := by grind
  have hs := rising_pos hkhalf n
  have hp := self_power_pos n
  have hq := self_power_pos (n+1)
  have hu : 0 < (K : Rat)/2+(n : Rat) := by grind
  have he := quotient_step (rising (1/2) n) (rising ((K : Rat)/2) n)
    ((n : Rat)^n) (((n+1 : Nat) : Rat)^(n+1)) (1/2+(n : Rat)) ((K : Rat)/2+(n : Rat))
    (Rat.ne_of_gt hs) (Rat.ne_of_gt hp) (Rat.ne_of_gt hq) (Rat.ne_of_gt hu)
  have h1 : 2*((1 : Rat)/2+(n : Rat)) = 2*(n : Rat)+1 := by grind
  have h2 : 2*((K : Rat)/2+(n : Rat)) = 2*(n : Rat)+(K : Rat) := by grind
  simp only [weight, rising_succ, weightStep, baseStep]
  simpa only [h1, h2, Rat.natCast_add, Rat.natCast_ofNat] using he

theorem weightStep_antitone {K a b : Nat} (hK : 0 < K) (h : a ≤ b) :
    weightStep K b ≤ weightStep K a := by
  induction h with
  | refl => exact Rat.le_refl
  | step h ih =>
    exact Rat.le_trans (Rat.le_of_lt (weightStep_next_lt hK _)) ih

theorem weight_slope_antitone {K a b : Nat} (hK : 0 < K) (h : a ≤ b) :
    weight K (b+1)*weight K a ≤ weight K b*weight K (a+1) := by
  rw [weight_succ hK b, weight_succ hK a]
  have hm := Rat.mul_le_mul_of_nonneg_left (weightStep_antitone hK h)
    (Rat.le_of_lt (Rat.mul_pos (weight_pos hK b) (weight_pos hK a)))
  simpa only [Rat.mul_assoc, Rat.mul_comm, mul_left_comm] using hm

end RefinementFeasibility
