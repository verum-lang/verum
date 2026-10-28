import VerumExternalReplay.CapacityPartitions
import VerumExternalReplay.BalancedProduct

/-! Exact outcome-independent capacity. The histogram theorem is instantiated
with proved KT factor inequalities, not a balancing assumption. Empty cells
are neutral. This is a finite likelihood-ratio bound; it proves no statistical
error guarantee, adaptive-stopping law, native execution refinement or admission. -/
namespace RefinementCapacity
open RefinementFeasibility
set_option warningAsError true

def capacity (K n : Nat) : Rat := (n : Rat)^n * BalancedProduct.bound (weight K) K n

def histogramRatio (K : Nat) (counts : List Nat) : Rat :=
  (counts.sum : Rat)^counts.sum * BalancedProduct.product (weight K) counts

theorem histogram_bound {K : Nat} (hK : 0 < K) (counts : List Nat)
    (width : counts.length = K) : histogramRatio K counts ≤ capacity K counts.sum := by
  have h := BalancedProduct.maximum (weight K) (weight_pos hK)
    (fun _ _ h => weight_slope_antitone hK h) counts
  rw [width] at h
  exact Rat.mul_le_mul_of_nonneg_left h (Rat.le_of_lt (self_power_pos counts.sum))

theorem balanced_attains {K : Nat} (hK : 0 < K) (n : Nat) :
    histogramRatio K (BalancedProduct.balancedCounts K n) = capacity K n := by
  unfold histogramRatio capacity
  rw [BalancedProduct.balanced_sum K n hK,
      BalancedProduct.balanced_attains (weight K) (weight_pos hK) K n hK]

/-- The familiar q/r formula, with K-r lower counts and r upper counts. -/
theorem capacity_formula {K : Nat} (hK : 0 < K) (n : Nat) :
    capacity K n = (n : Rat)^n *
      ((weight K (n/K))^(K-n%K) * (weight K (n/K+1))^(n%K)) := by
  have h := BalancedProduct.balanced_attains (weight K) (weight_pos hK) K n hK
  simp only [BalancedProduct.balancedCounts, BalancedProduct.product_append,
    BalancedProduct.product_replicate] at h
  unfold capacity
  rw [← h, Rat.mul_comm ((weight K (n/K+1))^(n%K))]

theorem capacity_zero (K : Nat) : capacity K 0 = 1 := by
  simp [capacity, BalancedProduct.bound]
  induction K with
  | zero => rfl
  | succ K ih => rw [Rat.pow_succ, ih, Rat.mul_one]

theorem capacity_positive {K : Nat} (hK : 0 < K) (n : Nat) : 0 < capacity K n := by
  exact Rat.mul_pos (self_power_pos n) (Rat.mul_pos
    (Rat.pow_pos (weight_pos hK (n/K)))
    (Rat.pow_pos (BalancedProduct.ratio_pos (weight K) (weight_pos hK) (n/K))))

def parentPower (counts : List Nat) : Rat := factorProduct (fun (n : Nat) => (n : Rat)^n) counts

/-- Closed empirical parent likelihood (without multinomial combinatorial factor).
Empty contexts contribute one because 0^0=1. -/
def parentLikelihood (counts : List Nat) : Rat :=
  parentPower counts / (counts.sum : Rat)^counts.sum

/-- Exact child KT divided by the empirical parent likelihood, in cross-product form. -/
def partitionScore (K : Nat) (rows : List (List Nat)) : Rat :=
  let counts := histogram K rows
  (counts.sum : Rat)^counts.sum * factorProduct (cell K) rows / parentPower counts

theorem parentPower_positive (counts : List Nat) : 0 < parentPower counts :=
  factorProduct_pos _ counts (by intro n _; exact self_power_pos n)

theorem product_bridge (w : Nat → Rat) (counts : List Nat) :
    factorProduct w counts = BalancedProduct.product w counts := by
  induction counts with
  | nil => rfl
  | cons n ns ih => simp only [factorProduct, BalancedProduct.product, ih]

theorem quotient_weights {K : Nat} (hK : 0 < K) (counts : List Nat) :
    factorProduct (RefinementFeasibility.pure K) counts / parentPower counts =
      BalancedProduct.product (weight K) counts := by
  have hg : ∀ n ∈ counts, (n : Rat)^n ≠ 0 := by intro n _; exact Rat.ne_of_gt (self_power_pos n)
  have h := factorProduct_div (RefinementFeasibility.pure K) (fun (n : Nat) => (n : Rat)^n) counts hg
  have he : (fun n => RefinementFeasibility.pure K n / (n : Rat)^n) = weight K := by
    funext n; exact (weight_eq_pure_div hK n).symm
  rw [he, product_bridge] at h
  exact h.symm

theorem partition_to_histogram {K : Nat} (hK : 0 < K) (rows : List (List Nat)) :
    partitionScore K rows ≤ histogramRatio K (histogram K rows) := by
  have hp := partition_bound hK rows
  have hc := self_power_pos (histogram K rows).sum
  have hd := parentPower_positive (histogram K rows)
  have h1 := Rat.mul_le_mul_of_nonneg_left hp (Rat.le_of_lt hc)
  have h2 := Rat.mul_le_mul_of_nonneg_right h1 (Rat.le_of_lt (Rat.inv_pos.mpr hd))
  have he := quotient_weights hK (histogram K rows)
  change (factorProduct (RefinementFeasibility.pure K) (histogram K rows)) *
    (parentPower (histogram K rows))⁻¹ = _ at he
  unfold partitionScore histogramRatio
  change _ ≤ (↑(histogram K rows).sum)^(histogram K rows).sum *
    BalancedProduct.product (weight K) (histogram K rows)
  change _ ≤ _ * (parentPower (histogram K rows))⁻¹ at h2
  simp only [Rat.mul_assoc] at h2
  rw [he] at h2
  simpa only [Rat.div_def, Rat.mul_assoc] using h2

/-- The full finite-partition upper bound, with the actual alphabet dimension. -/
theorem partition_capacity {K : Nat} (hK : 0 < K) (rows : List (List Nat))
    (width : ∀ row ∈ rows, row.length = K) :
    partitionScore K rows ≤ capacity K (rows.map List.sum).sum := by
  have h := Rat.le_trans (partition_to_histogram hK rows)
    (histogram_bound hK (histogram K rows) (histogram_length K rows width))
  simpa only [histogram_sum] using h

/-- A realizable pure-cell partition attains the histogram score. Zero rows are
neutral and can be removed using CapacityPartitions' deletion laws. -/
theorem pure_partition_score {K : Nat} (hK : 0 < K) (counts : List Nat)
    (width : counts.length = K) :
    partitionScore K (pureRows counts) = histogramRatio K counts := by
  have hh : histogram K (pureRows counts) = counts := by
    rw [← width]; exact histogram_pureRows counts
  unfold partitionScore histogramRatio
  rw [hh, pureRows_attains]
  have he := quotient_weights hK counts
  change _ * (parentPower counts)⁻¹ = _
  rw [Rat.mul_assoc]
  exact congrArg (fun x => (counts.sum : Rat)^counts.sum * x) he

/-- Sharpness: the concrete balanced diagonal partition attains the capacity. -/
theorem capacity_attained {K : Nat} (hK : 0 < K) (n : Nat) :
    partitionScore K (pureRows (BalancedProduct.balancedCounts K n)) = capacity K n := by
  rw [pure_partition_score hK _ (BalancedProduct.balanced_length K n hK), balanced_attains hK n]

/-- The normalized score is exactly the likelihood ratio, including empty contexts. -/
theorem partitionScore_eq_likelihood_ratio (K : Nat) (rows : List (List Nat)) :
    partitionScore K rows = factorProduct (cell K) rows / parentLikelihood (histogram K rows) := by
  simp only [partitionScore, parentLikelihood, Rat.div_def, Rat.inv_mul_rev, Rat.inv_inv]
  grind

theorem capacity_single_outcome (n : Nat) : capacity 1 n = 1 := by
  have hr := rising_pos (a := (1/2 : Rat)) (by grind) n
  have hp := self_power_pos n
  have hc := Rat.mul_inv_cancel (rising (1/2) n * (n : Rat)^n) (Rat.ne_of_gt (Rat.mul_pos hr hp))
  simp only [capacity, BalancedProduct.bound, Nat.div_one, Nat.mod_one,
    Rat.pow_one, Rat.pow_zero, Rat.mul_one, weight]
  grind only

theorem partitionScore_positive {K : Nat} (hK : 0 < K) (rows : List (List Nat)) :
    0 < partitionScore K rows := by
  exact Rat.mul_pos (Rat.mul_pos (self_power_pos (histogram K rows).sum)
    (factorProduct_pos (cell K) rows (by intro r _; exact cell_pos hK r)))
    (Rat.inv_pos.mpr (parentPower_positive (histogram K rows)))

/-- Contexts may have different fixed alphabets; finite products preserve the bound. -/
theorem contextual_capacity (contexts : List (Nat × List (List Nat)))
    (valid : ∀ c ∈ contexts, 0 < c.1 ∧ ∀ row ∈ c.2, row.length = c.1) :
    factorProduct (fun c => partitionScore c.1 c.2) contexts ≤
      factorProduct (fun c => capacity c.1 (c.2.map List.sum).sum) contexts := by
  induction contexts with
  | nil => exact Rat.le_refl
  | cons c cs ih =>
    have hv := valid c (by simp)
    have ht := ih (by intro x hx; exact valid x (by simp [hx]))
    have hh := partition_capacity hv.1 c.2 hv.2
    have hl := Rat.le_of_lt (factorProduct_pos (fun x => partitionScore x.1 x.2) cs
      (by intro x hx; exact partitionScore_positive (valid x (by simp [hx])).1 x.2))
    have hr := Rat.le_of_lt (capacity_positive hv.1 (c.2.map List.sum).sum)
    have h1 := Rat.mul_le_mul_of_nonneg_right hh hl
    have h2 := Rat.mul_le_mul_of_nonneg_left ht hr
    exact Rat.le_trans h1 h2

/-- Sharpness includes a dimension-correct partition containing exactly n events. -/
theorem capacity_sharp {K : Nat} (hK : 0 < K) (n : Nat) :
    ∃ rows : List (List Nat), (∀ row ∈ rows, row.length = K) ∧
      (rows.map List.sum).sum = n ∧ partitionScore K rows = capacity K n := by
  let counts := BalancedProduct.balancedCounts K n
  have hw : counts.length = K := BalancedProduct.balanced_length K n hK
  have hh : histogram K (pureRows counts) = counts := by
    rw [← hw]; exact histogram_pureRows counts
  have hs := histogram_sum K (pureRows counts)
  rw [hh, BalancedProduct.balanced_sum K n hK] at hs
  refine ⟨pureRows counts, ?_, hs.symm, capacity_attained hK n⟩
  intro row hr
  exact (pureRows_width counts row hr).trans hw

end RefinementCapacity
