import Std

/-! Logical laws used by the legacy tactic backend's structured obligations.
These are general elimination laws, not a proof of the Rust implementation,
SMT solver, AST translator, or kernel. Rust regressions check generated goals
and reject the concrete counterexamples independently. -/

namespace StructuredElimination

theorem guarded_cases (H G : Prop) (guards : List Prop)
    (coverage : H → ∃ guard ∈ guards, guard)
    (branches : ∀ guard ∈ guards, H → guard → G) : H → G := by
  intro h
  obtain ⟨guard, member, holds⟩ := coverage h
  exact branches guard member h holds

theorem signed_induction (Q : Int → Prop)
    (base : Q 0)
    (up : ∀ k, 0 ≤ k → Q k → Q (k + 1))
    (down : ∀ k, k ≤ 0 → Q k → Q (k - 1)) : ∀ k, Q k := by
  have positive : ∀ k : Nat, Q ((k : Int)) := by
    intro k
    induction k with
    | zero => exact base
    | succ k ih => exact up ((k : Int)) (by omega) ih
  have negative : ∀ k : Nat, Q (-(k : Int)) := by
    intro k
    induction k with
    | zero => exact base
    | succ k ih =>
      have target : -((k + 1 : Nat) : Int) = -(k : Int) - 1 := by omega
      rw [target]
      exact down (-(k : Int)) (by omega) ih
  intro k
  cases k with
  | ofNat n => exact positive n
  | negSucc n => exact negative (n + 1)

theorem contextual_signed_induction (H P : Int → Prop)
    (base : H 0 → P 0)
    (up : ∀ k, 0 ≤ k → H (k + 1) → (H k → P k) → P (k + 1))
    (down : ∀ k, k ≤ 0 → H (k - 1) → (H k → P k) → P (k - 1)) :
    ∀ k, H k → P k :=
  signed_induction (fun k => H k → P k) base
    (fun k domain ih h => up k domain h ih)
    (fun k domain ih h => down k domain h ih)

/-- Omitting the downward obligation admits a false universal claim. -/
theorem nonnegative_is_not_all_integers :
    (0 : Int) ≤ 0 ∧
    (∀ k : Int, 0 ≤ k → 0 ≤ k + 1) ∧
    ¬ (∀ k : Int, 0 ≤ k) := by
  refine ⟨by omega, ?_, ?_⟩
  · intro k hk
    omega
  · intro total
    have impossible := total (-1)
    omega

/-- Decision evidence is an explicit premise, not an imported choice axiom. -/
theorem decidable_contradiction (G : Prop) [Decidable G]
    (refute : ¬ G → False) : G := by
  cases (inferInstance : Decidable G) with
  | isTrue h => exact h
  | isFalse h => exact False.elim (refute h)

end StructuredElimination
