namespace Verum.FiniteStagePresentability

universe u

def inclusion {n m : Nat} (h : n ≤ m) (x : Fin n) : Fin m :=
  ⟨x.val, Nat.lt_of_lt_of_le x.isLt h⟩

structure Cocone (Y : Type u) where
  leg : (n : Nat) → Fin n → Y
  commutes : ∀ {n m} (h : n ≤ m) (x : Fin n), leg n x = leg m (inclusion h x)

variable {Y : Type u}

def descend (c : Cocone Y) (k : Nat) : Y := c.leg (k + 1) ⟨k, Nat.lt_succ_self k⟩

theorem index_directed (n m : Nat) : ∃ k, n ≤ k ∧ m ≤ k :=
  ⟨max n m, Nat.le_max_left n m, Nat.le_max_right n m⟩

theorem inclusions_compose {n m k : Nat} (h : n ≤ m) (j : m ≤ k) (x : Fin n) :
    inclusion j (inclusion h x) = inclusion (Nat.le_trans h j) x := rfl

theorem leg_factors (c : Cocone Y) (n : Nat) (x : Fin n) :
    descend c x.val = c.leg n x :=
  c.commutes (Nat.succ_le_of_lt x.isLt) ⟨x.val, Nat.lt_succ_self x.val⟩

theorem factor_unique (c : Cocone Y) (f : Nat → Y)
    (on_stage : ∀ n (x : Fin n), f x.val = c.leg n x) (k : Nat) : f k = descend c k :=
  on_stage (k + 1) ⟨k, Nat.lt_succ_self k⟩

/-- Nat is the colimit above, yet its identity factors through no finite stage. -/
theorem identity_has_no_finite_stage (n : Nat) (f : Nat → Fin n) :
    ¬ ∀ k, (f k).val = k := by
  intro factors
  have bound := (f n).isLt
  rw [factors n] at bound
  exact Nat.lt_irrefl n bound

#print axioms index_directed
#print axioms inclusions_compose
#print axioms leg_factors
#print axioms factor_unique
#print axioms identity_has_no_finite_stage
end Verum.FiniteStagePresentability
