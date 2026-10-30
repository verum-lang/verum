import VerumExternalReplay.ExactMass

/-! Exact bounds from a finite, injectively named portion of a subprobability.
Unknown classifications contribute to neither endpoint. An arbitrary rational
completion accounts for goal mass outside the finite carrier without dividing
by observed mass. Weights must denote completed events; prefix search bounds
do not establish that premise. Callback fidelity, complete wire identity and correspondence
between a learned forecast and physical dynamics are separate premises. -/
namespace ClassifiedMass
open ExactMass
set_option warningAsError true

def complement (a : Fraction) (_bounded : AtMost a one) : Fraction :=
  ⟨a.denominator - a.numerator, a.denominator, a.positive⟩

theorem complement_le_one (a : Fraction) (h : AtMost a one) :
    AtMost (complement a h) one := by
  simp [AtMost, complement, one]

theorem le_complement_iff (a b : Fraction) (hb : AtMost b one) :
    AtMost a (complement b hb) ↔ AtMost (add a b) one := by
  have hn : b.numerator ≤ b.denominator := by simpa [AtMost, one] using hb
  have scaled := Nat.mul_le_mul_right a.denominator hn
  simp only [AtMost, complement, add, one, Nat.mul_one]
  rw [Nat.sub_mul, Nat.mul_comm b.denominator a.denominator]
  rw [Nat.mul_comm b.denominator a.denominator] at scaled
  rw [Nat.one_mul]
  exact Nat.le_sub_iff_add_le scaled

theorem complement_antitone {a b : Fraction} (ha : AtMost a one)
    (hb : AtMost b one) (h : AtMost a b) :
    AtMost (complement b hb) (complement a ha) := by
  change (b.denominator - b.numerator) * a.denominator ≤
    (a.denominator - a.numerator) * b.denominator
  rw [Nat.sub_mul, Nat.sub_mul, Nat.mul_comm b.denominator a.denominator]
  exact Nat.sub_le_sub_left h _

theorem less_le_trans {a b c : Fraction} (h : Less a b) (k : AtMost b c) : Less a c := by
  by_cases ordered : Less a c
  · exact ordered
  · have reverse : AtMost c a := by unfold Less AtMost at *; omega
    have impossible := le_less_trans (le_trans k reverse) h
    exact False.elim (Nat.lt_irrefl _ impossible)

variable {α : Type}

structure FiniteWeights (α : Type) where
  names : List α
  distinct : names.Nodup
  weight : α → Fraction
  bounded : AtMost (sum (names.map weight)) one

variable {w wa wb : FiniteWeights α}

def selected (w : FiniteWeights α) (keep : α → Bool) : Fraction :=
  sum (w.names.map (fun x => if keep x then w.weight x else zero))

theorem selected_mono (w : FiniteWeights α) (a b : α → Bool)
    (h : ∀ x ∈ w.names, a x = true → b x = true) :
    AtMost (selected w a) (selected w b) := by
  apply sum_mono
  intro x hx
  cases ha : a x
  · simp only [Bool.false_eq_true, ite_false]; exact zero_le _
  · simp only [h x hx ha, ite_true]; exact le_refl _

theorem selected_le_total (w : FiniteWeights α) (keep : α → Bool) :
    AtMost (selected w keep) (sum (w.names.map w.weight)) := by
  apply sum_mono
  intro x _
  cases keep x <;> simp only [Bool.false_eq_true, ite_false, ite_true]
  · exact zero_le _
  · exact le_refl _

theorem selected_bounded (w : FiniteWeights α) (keep : α → Bool) :
    AtMost (selected w keep) one := le_trans (selected_le_total w keep) w.bounded

/-- Extending the finite carrier does not reweight observations already made.
The new carrier's uniqueness proof excludes repeated sample names. -/
theorem selected_extension (old new : FiniteWeights α) (extra : List α)
    (names : new.names = old.names ++ extra)
    (weights : ∀ x ∈ old.names, Equal (old.weight x) (new.weight x))
    (a b : α → Bool) (grows : ∀ x ∈ old.names, a x = true → b x = true) :
    AtMost (selected old a) (selected new b) := by
  have previous : AtMost (selected old a)
      (sum (old.names.map (fun x => if b x then new.weight x else zero))) := by
    apply sum_mono
    intro x hx
    cases ha : a x
    · simp only [Bool.false_eq_true, ite_false]; exact zero_le _
    · simp only [grows x hx ha, ite_true]; exact eq_le (weights x hx)
  have combined : Equal (selected new b)
      (add (sum (old.names.map (fun x => if b x then new.weight x else zero)))
        (sum (extra.map (fun x => if b x then new.weight x else zero)))) := by
    unfold selected
    rw [names, List.map_append]
    exact sum_append _ _
  exact le_trans previous (le_trans (le_add _ _) (eq_le (eq_symm combined)))

theorem disjoint_mass_le_total (w : FiniteWeights α) (a b : α → Bool)
    (disjoint : ∀ x ∈ w.names, a x = true → b x = false) :
    AtMost (add (selected w a) (selected w b)) (sum (w.names.map w.weight)) := by
  apply le_trans (eq_le (eq_symm (sum_add w.names _ _)))
  apply sum_mono
  intro x hx
  cases ha : a x <;> cases hb : b x <;>
    simp only [Bool.false_eq_true, ite_false, ite_true]
  · exact le_trans (eq_le (add_zero _)) (zero_le _)
  · exact eq_le (eq_trans (add_comm _ _) (add_zero _))
  · exact eq_le (add_zero _)
  · have impossible := disjoint x hx ha; simp [hb] at impossible

structure Classification (w : FiniteWeights α) where
  met : α → Bool
  unmet : α → Bool
  disjoint : ∀ x ∈ w.names, met x = true → unmet x = false

def lower (c : Classification w) : Fraction := selected w c.met
def upper (c : Classification w) : Fraction :=
  complement (selected w c.unmet) (selected_bounded w c.unmet)

def Correct (c : Classification w) (goal : α → Bool) : Prop :=
  ∀ x ∈ w.names, (c.met x = true → goal x = true) ∧
    (c.unmet x = true → goal x = false)

instance (c : Classification w) (goal : α → Bool) : Decidable (Correct c goal) :=
  inferInstanceAs (Decidable (∀ x ∈ w.names,
    (c.met x = true → goal x = true) ∧ (c.unmet x = true → goal x = false)))

theorem endpoints_ordered (c : Classification w) : AtMost (lower c) (upper c) :=
  (le_complement_iff _ _ _).mpr
    (le_trans (disjoint_mass_le_total w c.met c.unmet c.disjoint) w.bounded)

/-- The completion is not conditioned on successful decoding or observed work.
It can include unvisited wires; any goal portion within the remaining budget is
covered. This theorem is about exact rational completions, not a native callback
or an independently established infinite measure semantics. -/
theorem interval_sound (c : Classification w) (goal : α → Bool)
    (correct : Correct c goal) (completion : Fraction)
    (budget : AtMost (add (sum (w.names.map w.weight)) completion) one) :
    AtMost (lower c) (add (selected w goal) completion) ∧
      AtMost (add (selected w goal) completion) (upper c) := by
  constructor
  · exact le_trans (selected_mono w _ _ (fun x hx => (correct x hx).1)) (le_add _ _)
  · apply (le_complement_iff _ _ _).mpr
    have disjoint : ∀ x ∈ w.names, goal x = true → c.unmet x = false := by
      intro x hx hg
      cases hu : c.unmet x
      · rfl
      · have impossible := (correct x hx).2 hu; simp [hg] at impossible
    have rearrange : Equal (add (add (selected w goal) completion) (selected w c.unmet))
        (add (add (selected w goal) (selected w c.unmet)) completion) :=
      eq_trans (add_assoc _ _ _) (eq_trans (add_congr_left _ (add_comm _ _))
        (eq_symm (add_assoc _ _ _)))
    exact le_trans (eq_le rearrange)
      (le_trans (add_mono_right completion (disjoint_mass_le_total w _ _ disjoint)) budget)

def Refines (old new : Classification w) : Prop :=
  (∀ x ∈ w.names, old.met x = true → new.met x = true) ∧
  (∀ x ∈ w.names, old.unmet x = true → new.unmet x = true)

instance (old new : Classification w) : Decidable (Refines old new) :=
  inferInstanceAs (Decidable ((∀ x ∈ w.names, old.met x = true → new.met x = true) ∧
    (∀ x ∈ w.names, old.unmet x = true → new.unmet x = true)))

theorem refinement_narrows (old new : Classification w) (h : Refines old new) :
    AtMost (lower old) (lower new) ∧ AtMost (upper new) (upper old) :=
  ⟨selected_mono w _ _ h.1,
    complement_antitone _ _ (selected_mono w _ _ h.2)⟩

theorem extension_narrows {old new : FiniteWeights α}
    (a : Classification old) (b : Classification new) (extra : List α)
    (names : new.names = old.names ++ extra)
    (weights : ∀ x ∈ old.names, Equal (old.weight x) (new.weight x))
    (met : ∀ x ∈ old.names, a.met x = true → b.met x = true)
    (unmet : ∀ x ∈ old.names, a.unmet x = true → b.unmet x = true) :
    AtMost (lower a) (lower b) ∧ AtMost (upper b) (upper a) :=
  ⟨selected_extension old new extra names weights _ _ met,
    complement_antitone _ _ (selected_extension old new extra names weights _ _ unmet)⟩

/-- An overlap or equality of endpoints establishes no strict ranking. -/
theorem interval_dominance {β : Type} {other : FiniteWeights β}
    (a : Classification wa) (b : Classification other)
    (pa pb : Fraction) (a_lower : AtMost (lower a) pa) (b_upper : AtMost pb (upper b))
    (separated : Less (upper b) (lower a)) : Less pb pa :=
  less_le_trans (le_less_trans b_upper separated) a_lower

end ClassifiedMass
