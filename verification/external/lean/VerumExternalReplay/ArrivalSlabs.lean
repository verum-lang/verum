import Std.Tactic

/-!
Canonical partition of nonempty finite tuples by the first maximal arrival ID.
Earlier positions range below the maximum; later positions include it. IDs are
immutable coordinates, not semantic equality of the children they designate.
The partition proves neither native scheduler fairness nor application behavior.
-/
namespace ArrivalSlabs
set_option autoImplicit false
set_option warningAsError true

def Slab {arity : Nat} (row : Fin arity → Nat) (maximum : Nat) (anchor : Fin arity) : Prop :=
  row anchor = maximum ∧
  (∀ index, index < anchor → row index < maximum) ∧
  (∀ index, anchor < index → row index ≤ maximum)

instance {arity : Nat} (row : Fin arity → Nat) (maximum : Nat) (anchor : Fin arity) :
    Decidable (Slab row maximum anchor) := inferInstanceAs (Decidable (_ ∧ _ ∧ _))

def firstMaximum : {n : Nat} → (Fin (n + 1) → Nat) → Nat × Fin (n + 1)
  | 0, row => (row 0, 0)
  | n + 1, row =>
      let tail := firstMaximum (fun index : Fin (n + 1) => row index.succ)
      if row 0 < tail.1 then (tail.1, tail.2.succ) else (row 0, 0)

theorem slab_upper {arity : Nat} {row : Fin arity → Nat} {maximum : Nat} {anchor : Fin arity}
    (member : Slab row maximum anchor) (index : Fin arity) : row index ≤ maximum := by
  by_cases same : index = anchor
  · simp [same, member.1]
  by_cases before : index < anchor
  · exact Nat.le_of_lt (member.2.1 index before)
  · exact member.2.2 index (by omega)

theorem slab_unique {arity : Nat} {row : Fin arity → Nat}
    {m n : Nat} {a b : Fin arity} (left : Slab row m a) (right : Slab row n b) :
    m = n ∧ a = b := by
  have lower := slab_upper left b
  have upper := slab_upper right a
  have same : m = n := by rw [left.1] at upper; rw [right.1] at lower; omega
  refine ⟨same, ?_⟩
  by_cases different : a = b
  · exact different
  by_cases before : a < b
  · have smaller := right.2.1 a before
    rw [left.1] at smaller
    omega
  · have smaller := left.2.1 b (by omega)
    rw [right.1] at smaller
    omega

theorem firstMaximum_spec {n : Nat} (row : Fin (n + 1) → Nat) :
    Slab row (firstMaximum row).1 (firstMaximum row).2 := by
  induction n with
  | zero =>
      refine ⟨rfl, ?_, ?_⟩
      · intro index before; change index < (0 : Fin 1) at before; omega
      · intro index after; change (0 : Fin 1) < index at after; omega
  | succ n ih =>
      have tail := ih (fun index : Fin (n + 1) => row index.succ)
      simp only [firstMaximum]
      split
      · rename_i greater
        refine ⟨tail.1, ?_, ?_⟩
        · intro index before
          cases index using Fin.cases with
          | zero => exact greater
          | succ prior => exact tail.2.1 prior (by simpa using before)
        · intro index after
          cases index using Fin.cases with
          | zero => simp at after
          | succ prior => exact tail.2.2 prior (by simpa using after)
      · rename_i head
        refine ⟨rfl, ?_, ?_⟩
        · intro index before; simp at before
        · intro index after
          cases index using Fin.cases with
          | zero => simp at after
          | succ prior =>
              have bound := slab_upper tail prior
              omega

/-- Every nonempty tuple lies in one slab, selected constructively. -/
theorem coverage {n : Nat} (row : Fin (n + 1) → Nat) :
    ∃ key : Nat × Fin (n + 1), Slab row key.1 key.2 :=
  ⟨firstMaximum row, firstMaximum_spec row⟩

theorem canonical_membership {n : Nat} (row : Fin (n + 1) → Nat)
    (key : Nat × Fin (n + 1)) : Slab row key.1 key.2 ↔ key = firstMaximum row := by
  constructor
  · intro member
    obtain ⟨maximum, anchor⟩ := slab_unique member (firstMaximum_spec row)
    exact Prod.ext maximum anchor
  · intro same; subst key; exact firstMaximum_spec row

theorem disjoint {arity : Nat} {row : Fin arity → Nat} {m n : Nat} {a b : Fin arity}
    (different : (m, a) ≠ (n, b)) (left : Slab row m a) : ¬ Slab row n b := by
  intro right
  obtain ⟨maximum, anchor⟩ := slab_unique left right
  exact different (Prod.ext maximum anchor)

/-- Filtering by a tree-size bound (or another exact predicate) does not change
the unique slab of an admitted tuple. No arithmetic or scheduler is assumed. -/
theorem filtered_membership {n : Nat} (admitted : (Fin (n + 1) → Nat) → Prop)
    (row : Fin (n + 1) → Nat) (key : Nat × Fin (n + 1)) :
    (admitted row ∧ Slab row key.1 key.2) ↔
      (admitted row ∧ key = firstMaximum row) := by
  rw [canonical_membership]

end ArrivalSlabs
