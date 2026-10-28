import Lean

/-! Unconditional algebra for the label-free KT refinement feasibility bound.

`normalized_gap` proves the strict scalar inequality underlying discrete
log-concavity of `(1/2)_m / m^m`, for every positive natural `m`. Its proof
uses a cubic Bonferroni bound over exact rationals, with no analytic axiom.

This module does not yet prove the sharp balanced-histogram bound: the
rising-factorial bridge, finite balancing argument, zero-event probability,
and correspondence to the native per-context scorer remain separate obligations.
No theorem here authorizes admission or substitutes for the scorer's premises.

Core `Rat` arithmetic has transitive standard dependencies on `propext`,
`Classical.choice`, and `Quot.sound`; this is outside the finite native replay
module's narrower axiom roster and does not relax that roster.
-/
namespace RefinementFeasibility
set_option warningAsError true

/-- Third-order inclusion-exclusion polynomial; natural exponents use rational coefficients. -/
def cubic (r u : Rat) : Rat :=
  1 - r*u + r*(r-1)/2*u^2 - r*(r-1)*(r-2)/6*u^3

/-- The omitted fourth-order term has nonnegative coefficient at natural exponents. -/
theorem cubic_step (r u : Rat) :
    (1-u)*cubic r u - cubic (r+1) u = r*(r-1)*(r-2)/6*u^4 := by
  unfold cubic
  grind

theorem coefficient_nonneg (n : Nat) :
    0 ≤ (n : Rat)*((n : Rat)-1)*((n : Rat)-2)/6 := by
  cases n with
  | zero => grind
  | succ n =>
    cases n with
    | zero => grind
    | succ n =>
      have h : ((2 : Nat) : Rat) ≤ ((n+1+1 : Nat) : Rat) := Rat.natCast_le_natCast.mpr (by omega)
      have h1 : 0 ≤ ((n+1+1 : Nat) : Rat) := by grind
      have h2 : 0 ≤ ((n+1+1 : Nat) : Rat)-1 := by grind
      have h3 : 0 ≤ ((n+1+1 : Nat) : Rat)-2 := by grind
      have hh := Rat.mul_nonneg (Rat.mul_nonneg h1 h2) h3
      grind

/-- Uniform lower bound, including the empty product at exponent zero. -/
theorem cubic_lower (n : Nat) (u : Rat) (h0 : 0 ≤ u) (h1 : u ≤ 1) :
    cubic n u ≤ (1-u)^n := by
  induction n with
  | zero => simp [cubic]; grind
  | succ n ih =>
    have hn : 0 ≤ 1-u := by grind
    have hm := Rat.mul_le_mul_of_nonneg_left ih hn
    have hp := Rat.mul_nonneg (coefficient_nonneg n) (Rat.pow_nonneg (n := 4) h0)
    have he := cubic_step n u
    rw [Rat.pow_succ]
    simp only [Rat.natCast_add] at *
    grind


/-- Exact algebraic identity, independent of positivity. -/
theorem gap_identity (m : Rat) (hm : m ≠ 0) :
    (2*m-1)*(1+1/m)^2*cubic (m-1) (1/m^2) - (2*m+1) =
    (m^6 + m^2*(5*m-6)*(m-1) + 11*m-6)/(6*m^8) := by
  unfold cubic
  grind


theorem cubic_gap_positive (m : Rat) (hm : 2 ≤ m) :
    2*m+1 < (2*m-1)*(1+1/m)^2*cubic (m-1) (1/m^2) := by
  have hpos : 0 < m := by grind
  have h5 : 0 ≤ 5*m-6 := by grind
  have hminus : 0 ≤ m-1 := by grind
  have hp := Rat.mul_nonneg (Rat.mul_nonneg (Rat.pow_nonneg (n := 2) (show 0 ≤ m by grind)) h5) hminus
  have hnum : 0 < m^6 + m^2*(5*m-6)*(m-1) + 11*m-6 := by
    have hh := Rat.pow_nonneg (n := 6) (show 0 ≤ m by grind)
    grind
  have hden : 0 < 6*m^8 := Rat.mul_pos (by decide) (Rat.pow_pos hpos)
  have hid := gap_identity m (by grind)
  have hquot := Rat.mul_pos hnum (Rat.inv_pos.mpr hden)
  change 0 < (m^6 + m^2*(5*m-6)*(m-1) + 11*m-6)/(6*m^8) at hquot
  rw [← hid] at hquot
  grind


/-- Strict scalar balancing inequality, with `m = n + 1`; no hypothesis assumes balancing. -/
theorem normalized_gap (n : Nat) :
    2*((n : Rat)+1)+1 <
      (2*((n : Rat)+1)-1)*(1+1/((n : Rat)+1))^2*
        (1-1/((n : Rat)+1)^2)^n := by
  cases n with
  | zero => simp only [Rat.pow_zero]; grind
  | succ n =>
    let m : Rat := ((n+1 : Nat) : Rat)+1
    have hm : 2 ≤ m := by
      have hn := Rat.natCast_nonneg (a := n)
      dsimp [m]
      simp only [Rat.natCast_add]
      grind
    have hmpos : 0 < m := by grind
    have hspos := Rat.pow_pos (n := 2) hmpos
    have hsq := Rat.mul_le_mul_of_nonneg_left (show 1 ≤ m by grind) (show 0 ≤ m by grind)
    have hsqone : 1 ≤ m^2 := by grind
    have hu0 : 0 ≤ 1/m^2 := by
      have h := Rat.inv_pos.mpr hspos
      grind
    have hu1 : 1/m^2 ≤ 1 := by
      have h := Rat.mul_le_mul_of_nonneg_right hsqone (Rat.le_of_lt (Rat.inv_pos.mpr hspos))
      have hc := Rat.mul_inv_cancel (m^2) (Rat.ne_of_gt hspos)
      grind
    have hl := cubic_lower (n+1) (1/m^2) hu0 hu1
    have hinv := Rat.inv_pos.mpr hmpos
    have hc : 0 ≤ (2*m-1)*(1+1/m)^2 :=
      Rat.mul_nonneg (by grind) (Rat.pow_nonneg (n := 2) (by grind))
    have hmul := Rat.mul_le_mul_of_nonneg_left hl hc
    have hp := cubic_gap_positive m hm
    have hid : ((n+1 : Nat) : Rat) = m-1 := by dsimp [m]; grind
    rw [hid] at hmul
    change 2*m+1 < (2*m-1)*(1+1/m)^2*(1-1/m^2)^(n+1)
    grind only

end RefinementFeasibility
