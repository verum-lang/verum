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

/-- A map into this colimit factors at precisely the stages bounding its image. -/
def FactorsAt {X : Type u} (f : X → Nat) (n : Nat) : Prop :=
  ∃ g : X → Fin n, ∀ x, (g x).val = f x

theorem factorsAt_iff {X : Type u} (f : X → Nat) (n : Nat) :
    FactorsAt f n ↔ ∀ x, f x < n := by
  constructor
  · rintro ⟨g, hg⟩ x
    exact hg x ▸ (g x).isLt
  · intro h
    exact ⟨fun x => ⟨f x, h x⟩, fun _ => rfl⟩

def finiteBound : (k : Nat) → (Fin k → Nat) → Nat
  | 0, _ => 0
  | k + 1, f => max (f 0 + 1) (finiteBound k (fun i => f i.succ))

theorem below_finiteBound (k : Nat) (f : Fin k → Nat) (x : Fin k) : f x < finiteBound k f := by
  induction k with
  | zero => exact Fin.elim0 x
  | succ k ih =>
    refine Fin.cases ?_ (fun i => ?_) x
    · exact Nat.lt_of_lt_of_le (Nat.lt_succ_self _) (Nat.le_max_left _ _)
    · exact Nat.lt_of_lt_of_le (ih (fun j => f j.succ) i) (Nat.le_max_right _ _)

/-- A finite family with finite sources has a computable common stage. -/
theorem finite_family_factors (m k : Nat) (f : Fin m → Fin k → Nat) :
    ∃ n, ∀ i, FactorsAt (f i) n := by
  refine ⟨finiteBound m (fun i => finiteBound k (f i)), fun i => (factorsAt_iff _ _).mpr ?_⟩
  intro x
  exact Nat.lt_trans (below_finiteBound k (f i) x) (below_finiteBound m (fun i => finiteBound k (f i)) i)

/-- Even a single explicitly defined sequence of point maps separates the quantifier orders. -/
theorem pointwise_stages_without_common_stage :
    (∀ i : Nat, ∃ n, FactorsAt (fun _ : Unit => i) n) ∧
    ¬ ∃ n, ∀ i : Nat, FactorsAt (fun _ : Unit => i) n := by
  constructor
  · intro i
    exact ⟨i+1, (factorsAt_iff _ _).mpr (fun _ => Nat.lt_succ_self i)⟩
  · rintro ⟨n, h⟩
    exact Nat.lt_irrefl n ((factorsAt_iff _ _).mp (h n) ())

#print axioms factorsAt_iff
#print axioms below_finiteBound
#print axioms finite_family_factors
#print axioms pointwise_stages_without_common_stage
#print axioms index_directed
#print axioms inclusions_compose
#print axioms leg_factors
#print axioms factor_unique
#print axioms identity_has_no_finite_stage
end Verum.FiniteStagePresentability
