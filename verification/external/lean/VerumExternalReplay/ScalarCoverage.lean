import Std

/- A model of IntervalSet.first_uncovered's ordered scan. The theorem covers
   every integer interval and sorted list, not only native test fixtures.
   Rust represents its endpoints with i128. The +1 step is
   bounded by the upper endpoint in the implementation; this model uses Int.
   The theorem does not certify the Rust compiler or pattern-to-set lowering. -/
namespace Verum.ScalarCoverage

structure Interval where
  lo : Int
  hi : Int

def Covered (xs : List Interval) (x : Int) : Prop :=
  ∃ interval ∈ xs, interval.lo ≤ x ∧ x ≤ interval.hi

def firstGap (lo hi : Int) : List Interval → Option Int
  | [] => if hi < lo then none else some lo
  | interval :: rest =>
    if hi < lo then none
    else if interval.hi < lo then firstGap lo hi rest
    else if lo < interval.lo then some lo
    else if hi ≤ interval.hi then none
    else firstGap (interval.hi + 1) hi rest

theorem firstGap_spec (xs : List Interval)
    (sorted : xs.Pairwise (fun a b => a.lo ≤ b.lo)) :
    ∀ lo hi,
      match firstGap lo hi xs with
      | none => ∀ x, lo ≤ x → x ≤ hi → Covered xs x
      | some gap => lo ≤ gap ∧ gap ≤ hi ∧ ¬ Covered xs gap := by
  induction xs with
  | nil =>
    intro lo hi
    by_cases empty : hi < lo
    · simp only [firstGap, ite_eq_left empty]
      intro x h₁ h₂; omega
    · simp only [firstGap, ite_eq_right empty]
      exact ⟨Int.le_refl lo, by omega, by simp [Covered]⟩
  | cons interval rest ih =>
    have order := List.pairwise_cons.mp sorted
    intro lo hi
    by_cases empty : hi < lo
    · simp only [firstGap, ite_eq_left empty]
      intro x h₁ h₂; omega
    · by_cases behind : interval.hi < lo
      · simp only [firstGap, ite_eq_right empty, ite_eq_left behind]
        have previous := ih order.2 lo hi
        cases result : firstGap lo hi rest with
        | none =>
          simp only [result] at previous ⊢
          intro x h₁ h₂
          obtain ⟨i, member, bounds⟩ := previous x h₁ h₂
          exact ⟨i, List.mem_cons_of_mem interval member, bounds⟩
        | some gap =>
          simp only [result] at previous ⊢
          refine ⟨previous.1, previous.2.1, ?_⟩
          rintro ⟨i, member, bounds⟩
          rcases List.mem_cons.mp member with head | tail
          · subst i; omega
          · exact previous.2.2 ⟨i, tail, bounds⟩
      · by_cases ahead : lo < interval.lo
        · simp only [firstGap, ite_eq_right empty, ite_eq_right behind, ite_eq_left ahead]
          refine ⟨Int.le_refl lo, by omega, ?_⟩
          rintro ⟨i, member, bounds⟩
          rcases List.mem_cons.mp member with head | tail
          · subst i; omega
          · have start := order.1 i tail; omega
        · by_cases covered : hi ≤ interval.hi
          · simp only [firstGap, ite_eq_right empty, ite_eq_right behind, ite_eq_right ahead, ite_eq_left covered]
            intro x h₁ h₂
            exact ⟨interval, List.mem_cons_self, by omega, by omega⟩
          · simp only [firstGap, ite_eq_right empty, ite_eq_right behind, ite_eq_right ahead, ite_eq_right covered]
            have previous := ih order.2 (interval.hi + 1) hi
            cases result : firstGap (interval.hi + 1) hi rest with
            | none =>
              simp only [result] at previous ⊢
              intro x h₁ h₂
              by_cases before : x ≤ interval.hi
              · exact ⟨interval, List.mem_cons_self, by omega, before⟩
              · obtain ⟨i, member, bounds⟩ := previous x (by omega) h₂
                exact ⟨i, List.mem_cons_of_mem interval member, bounds⟩
            | some gap =>
              simp only [result] at previous ⊢
              refine ⟨by omega, previous.2.1, ?_⟩
              rintro ⟨i, member, bounds⟩
              rcases List.mem_cons.mp member with head | tail
              · subst i; omega
              · exact previous.2.2 ⟨i, tail, bounds⟩

#print axioms firstGap_spec
end Verum.ScalarCoverage
