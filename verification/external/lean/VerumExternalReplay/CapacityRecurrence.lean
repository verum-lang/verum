import VerumExternalReplay.RefinementCapacity

/-! Exact one-event recurrence of the proved capacity. The quotient/remainder
rollover is part of the proof. This is algebraic correspondence to the native
power factors, not a theorem about native execution or its resource accounting.
-/
namespace RefinementCapacity
open RefinementFeasibility
set_option warningAsError true

/-- Incrementing the total changes exactly one balanced count from q to q+1. -/
theorem balanced_bound_succ (w : Nat → Rat) (positive : ∀ i, 0 < w i)
    {K : Nat} (hK : 0 < K) (n : Nat) :
    BalancedProduct.bound w K (n+1) =
      BalancedProduct.bound w K n * BalancedProduct.ratio w (n/K) := by
  have hr := Nat.mod_lt n hK
  have hm : (n+1)%K = (n%K+1)%K := (Nat.mod_add_mod n K 1).symm
  by_cases rollover : n%K+1 = K
  · have hm0 : (n+1)%K = 0 := by rw [hm, rollover, Nat.mod_self]
    have hq := Nat.succ_div_of_mod_eq_zero hm0
    have hw : w (n/K+1) = w (n/K)*BalancedProduct.ratio w (n/K) := by
      unfold BalancedProduct.ratio
      rw [Rat.mul_comm, Rat.div_mul_cancel (Rat.ne_of_gt (positive (n/K)))]
    have hp : (BalancedProduct.ratio w (n/K))^K =
        (BalancedProduct.ratio w (n/K))^(n%K)*BalancedProduct.ratio w (n/K) := by
      simpa only [rollover] using Rat.pow_succ (BalancedProduct.ratio w (n/K)) (n%K)
    simp only [BalancedProduct.bound, hq, hm0, Rat.pow_zero, Rat.mul_one]
    rw [hw, BalancedProduct.pow_mul, hp, Rat.mul_assoc]
  · have hr' : n%K+1 < K := by omega
    have hm' : (n+1)%K = n%K+1 := by rw [hm, Nat.mod_eq_of_lt hr']
    have hq := Nat.succ_div_of_mod_ne_zero (show (n+1)%K ≠ 0 by omega)
    simp only [BalancedProduct.bound, hq, hm', Rat.pow_succ, Rat.mul_assoc]

theorem weight_ratio_eq_step {K : Nat} (hK : 0 < K) (q : Nat) :
    BalancedProduct.ratio (weight K) q = weightStep K q := by
  unfold BalancedProduct.ratio
  rw [weight_succ hK, Rat.mul_comm (weight K q),
    Rat.mul_div_cancel (Rat.ne_of_gt (weight_pos hK q))]

private theorem scale_step (p b q r : Rat) (hp : p ≠ 0) :
    q*(b*r) = (p*b)*(q/p*r) := by grind

theorem capacity_succ_step {K : Nat} (hK : 0 < K) (n : Nat) :
    capacity K (n+1) = capacity K n *
      ((((n+1 : Nat) : Rat)^(n+1)/(n : Rat)^n) * weightStep K (n/K)) := by
  unfold capacity
  rw [balanced_bound_succ (weight K) (weight_pos hK) hK, weight_ratio_eq_step hK]
  exact scale_step _ _ _ _ (Rat.ne_of_gt (self_power_pos n))

private theorem quotient_factors (a b c d e f : Rat)
    (hb : b ≠ 0) (he : e ≠ 0) (hf : f ≠ 0) :
    a/b*((c*d)/e/f) = (a*d*c)/(b*e*f) := by grind

/-- Native update factors with q=floor(n/K), including n=0 and remainder rollover. -/
theorem capacity_recurrence {K : Nat} (hK : 0 < K) (n : Nat) :
    let q := n/K
    capacity K (n+1) = capacity K n *
      ((((n+1 : Nat) : Rat)^(n+1)*(q : Rat)^q*(2*(q : Rat)+1)) /
        ((n : Rat)^n*((q : Rat)+1)^(q+1)*(2*(q : Rat)+(K : Rat)))) := by
  dsimp only
  rw [capacity_succ_step hK]
  have hq := Rat.natCast_nonneg (a := n/K)
  have hk : 0 < (K : Rat) := Rat.natCast_pos.mpr hK
  have hn := self_power_pos n
  have hnext : 0 < ((n/K : Nat) : Rat)+1 := by grind
  have hd : 0 < 2*((n/K : Nat) : Rat)+(K : Rat) := by grind
  unfold weightStep baseStep
  exact congrArg (fun x => capacity K n*x)
    (quotient_factors _ _ _ _ _ _ (Rat.ne_of_gt hn)
      (Rat.ne_of_gt (Rat.pow_pos hnext)) (Rat.ne_of_gt hd))

/-- One event in a declared K-outcome context has capacity exactly 1/K. -/
theorem capacity_first {K : Nat} (hK : 0 < K) : capacity K 1 = 1/(K : Rat) := by
  have h := capacity_recurrence hK 0
  simp only [Nat.zero_add, Nat.zero_div, capacity_zero, Rat.pow_zero, Rat.pow_one,
    Rat.natCast_ofNat, Rat.zero_add, Rat.mul_zero, Rat.one_mul, Rat.mul_one] at h
  exact h

end RefinementCapacity
