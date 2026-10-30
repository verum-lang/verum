import Lean
import VerumExternalReplay.ConservativeRefinement

/-! Exact chronological quotients of a closed deterministic lasso. Labels are
immutable mathematical observations. Native execution, exact equality, carrier
completeness and binding to a currently published owner are separate premises.
No acquisition, habitat-period or host-resource claim follows. -/
namespace ObservableLasso
set_option warningAsError true
variable {O : Type} {n r p q i t : Nat} {label w : Nat → O}

def next (n r i : Nat) : Nat := if i + 1 < n then i + 1 else r
def walk (n r : Nat) : Nat → Nat
  | 0 => 0
  | t + 1 => next n r (walk n r t)
def word (n r : Nat) (label : Nat → O) (t : Nat) : O := label (walk n r t)
def Periodic (w : Nat → O) (p : Nat) : Prop := ∀ t, w (t + p) = w t
def Candidate (n r : Nat) (label : Nat → O) (p : Nat) : Prop :=
  0 < p ∧ p ∣ n - r ∧ ∀ i, i < n → label i = label (i % p)

theorem successor_closed (closed : r < n) (i : Nat) : next n r i < n := by
  unfold next
  split <;> omega

theorem closing_congruence (before : r ≤ n) :
    n % p = r % p ↔ p ∣ n - r := by
  constructor
  · intro h
    exact Nat.dvd_of_mod_eq_zero (Nat.sub_mod_eq_zero_of_mod_eq h)
  · intro h
    have sum : n = n - r + r := by omega
    rw [sum, Nat.add_mod, Nat.mod_eq_zero_of_dvd h, Nat.zero_add, Nat.mod_mod]

theorem successor_congruence (closed : r < n) :
    (∀ i, i < n → next n r i % p = (i % p + 1) % p) ↔ p ∣ n - r := by
  constructor
  · intro h
    have last := h (n - 1) (by omega)
    have edge : next n r (n - 1) = r := by
      exact ite_eq_right (by omega : ¬ n - 1 + 1 < n)
    rw [edge, Nat.mod_add_mod] at last
    have lastIndex : n - 1 + 1 = n := by omega
    rw [lastIndex] at last
    exact (closing_congruence (by omega)).mp last.symm
  · intro h i hi
    rw [Nat.mod_add_mod]
    unfold next
    split
    · rfl
    · have last : i + 1 = n := by omega
      rw [last]
      exact ((closing_congruence (by omega)).mpr h).symm

theorem candidate_exact (closed : r < n) : Candidate n r label p ↔
    0 < p ∧ (∀ i, i < n → next n r i % p = (i % p + 1) % p) ∧
      (∀ i, i < n → label i = label (i % p)) := by
  constructor
  · intro h
    exact ⟨h.1, (successor_congruence closed).mpr h.2.1, h.2.2⟩
  · intro h
    exact ⟨h.1, (successor_congruence closed).mp h.2.1, h.2.2⟩

theorem candidate_bound (closed : r < n) (h : Candidate n r label p) : p ≤ n := by
  have bound := Nat.le_of_dvd (show 0 < n - r by omega) h.2.1
  omega

theorem walk_closed (closed : r < n) (t : Nat) : walk n r t < n := by
  cases t with
  | zero => simp only [walk]; omega
  | succ t => exact successor_closed closed _

theorem walk_initial (within : t < n) : walk n r t = t := by
  induction t with
  | zero => rfl
  | succ t ih =>
    rw [walk, ih (by omega)]
    exact ite_eq_left within

theorem walk_code (closed : r < n) (divides : p ∣ n - r) (t : Nat) :
    walk n r t % p = t % p := by
  induction t with
  | zero => rfl
  | succ t ih =>
    rw [walk, (successor_congruence closed).mpr divides _ (walk_closed closed t), ih,
      Nat.mod_add_mod]

theorem word_canonical (closed : r < n) (h : Candidate n r label p) (t : Nat) :
    word n r label t = label (t % p) := by
  unfold word
  rw [h.2.2 _ (walk_closed closed t), walk_code closed h.2.1]

theorem periodic_multiple (h : Periodic w p) (k t : Nat) : w (t + p * k) = w t := by
  induction k with
  | zero => simp only [Nat.mul_zero, Nat.add_zero]
  | succ k ih =>
    rw [Nat.mul_succ, ← Nat.add_assoc, h, ih]

theorem periodic_reduce (h : Periodic w p) (t : Nat) : w t = w (t % p) := by
  have result := periodic_multiple h (t / p) (t % p)
  rw [Nat.mod_add_div] at result
  exact result

theorem periodic_remainder (hp : Periodic w p) (hq : Periodic w q) :
    Periodic w (q % p) := by
  intro t
  have shifted := periodic_multiple hp (q / p) (t + q % p)
  rw [Nat.add_assoc, Nat.mod_add_div] at shifted
  exact shifted.symm.trans (hq t)

theorem periodic_gcd (hp : Periodic w p) (hq : Periodic w q) :
    Periodic w (Nat.gcd p q) := by
  induction p, q using Nat.gcd.induction with
  | H0 q => simpa only [Nat.gcd_zero_left] using hq
  | H1 p q _ ih =>
    rw [Nat.gcd_rec]
    exact ih (periodic_remainder hp hq) hp

theorem candidate_period (closed : r < n) (h : Candidate n r label p) :
    Periodic (word n r label) p := by
  intro t
  rw [word_canonical closed h, word_canonical closed h]
  simp only [Nat.add_mod_right]

theorem candidate_tail_period (closed : r < n) (h : Candidate n r label p) :
    Periodic (word n r label) (n - r) := by
  obtain ⟨k, hk⟩ := h.2.1
  rw [hk]
  exact fun t => periodic_multiple (candidate_period closed h) k t

theorem period_labels (h : Periodic (word n r label) p) (within : i < n) :
    label i = label (i % p) := by
  have eq := periodic_reduce h i
  have bound : i % p < n := Nat.lt_of_le_of_lt (Nat.mod_le _ _) within
  simpa only [word, walk_initial within, walk_initial bound] using eq

-- Minimality among checked candidates implies minimality among ALL positive
-- observable periods, including periods not dividing the raw cycle length.
theorem minimal_candidate_primitive (closed : r < n) (h : Candidate n r label p)
    (minimal : ∀ d, Candidate n r label d → p ≤ d)
    (positive : 0 < d) (period : Periodic (word n r label) d) : p ≤ d := by
  have common := periodic_gcd (candidate_tail_period closed h) period
  have valid : Candidate n r label (Nat.gcd (n - r) d) :=
    ⟨Nat.gcd_pos_of_pos_right _ positive, Nat.gcd_dvd_left _ _,
      fun _ within => period_labels common within⟩
  exact Nat.le_trans (minimal _ valid) (Nat.gcd_le_right _ positive)

theorem least_period_divides (positive : 0 < p) (hp : Periodic w p)
    (minimal : ∀ d, 0 < d → Periodic w d → p ≤ d) (hq : Periodic w q) : p ∣ q := by
  have rest := periodic_remainder hp hq
  have bound := Nat.mod_lt q positive
  have zero : q % p = 0 := by
    by_cases eq : q % p = 0
    · exact eq
    · have least := minimal _ (by omega) rest
      omega
  exact Nat.dvd_of_mod_eq_zero zero

theorem minimal_candidate_divides_every_period (closed : r < n)
    (h : Candidate n r label p) (minimal : ∀ d, Candidate n r label d → p ≤ d)
    (period : Periodic (word n r label) q) : p ∣ q :=
  least_period_divides h.1 (candidate_period closed h)
    (fun _ positive hd => minimal_candidate_primitive closed h minimal positive hd) period

-- Reuse the shared execution semantics; no lasso-specific evaluator replaces
-- its treatment of actions, initial observations, stopping or refusals.
def rawStep (n r : Nat) (i : Nat) (_ : Unit) :
    ConservativeRefinement.Outcome Nat Unit Unit := .next (next n r i)
def cycleStep (p : Nat) (i : Nat) (_ : Unit) :
    ConservativeRefinement.Outcome Nat Unit Unit := .next ((i + 1) % p)

theorem finite_admissible (closed : r < n) (within : i < n) (actions : List Unit) :
    ConservativeRefinement.Admissible (rawStep n r) (fun i _ => i < n) actions i := by
  induction actions generalizing i with
  | nil => trivial
  | cons _ rest ih => exact ⟨within, ih (successor_closed closed i)⟩

theorem finite_execution_preserved (closed : r < n) (h : Candidate n r label p)
    (actions : List Unit) (within : i < n) :
    ConservativeRefinement.run (cycleStep p) actions (i % p) =
      (ConservativeRefinement.run (rawStep n r) actions i).map (fun i => i % p) := by
  apply ConservativeRefinement.finite_execution_preserved
    (fun i => i % p) (fun _ a => a) (cycleStep p) (rawStep n r) (fun i _ => i < n)
  · intro j _ hj
    change ConservativeRefinement.Outcome.next ((j % p + 1) % p) = .next (next n r j % p)
    rw [(successor_congruence closed).mpr h.2.1 j hj]
  · exact finite_admissible closed within actions

theorem finite_observations_preserved (closed : r < n) (h : Candidate n r label p)
    (actions : List Unit) (within : i < n) :
    (ConservativeRefinement.run (cycleStep p) actions (i % p)).states.map label =
      (ConservativeRefinement.run (rawStep n r) actions i).states.map label := by
  rw [finite_execution_preserved closed h actions within]
  change ((ConservativeRefinement.run (rawStep n r) actions i).states.map
    (fun i => i % p)).map label = _
  induction actions generalizing i with
  | nil =>
    simp only [ConservativeRefinement.run, List.map_cons, List.map_nil, h.2.2 i within]
  | cons action rest ih =>
    simp only [ConservativeRefinement.run, rawStep, List.map_cons]
    rw [ih (successor_closed closed i), h.2.2 i within]

end ObservableLasso
