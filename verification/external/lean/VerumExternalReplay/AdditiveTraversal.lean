import Lean

/-! Outcome-independent enumeration by additive structural cost. Each component
provides a finite, duplicate-free list at each cost and exact membership evidence.
Supports may be unequal or infinite; an infinite zero-cost layer is excluded by
these premises. No native iterator, work bound, statistical mass or rank encoding
is certified by this model. -/
namespace AdditiveTraversal
set_option warningAsError true
universe u

structure Layers (A : Type u) where
  admissible : A → Prop
  cost : A → Nat
  layer : Nat → List A
  membership : ∀ a n, a ∈ layer n ↔ admissible a ∧ cost a = n
  unique : ∀ n, (layer n).Nodup

private theorem finite_interval_nodup (start count : Nat) : (List.range' start count).Nodup := by
  induction count generalizing start with
  | zero => simp
  | succ count ih =>
      simp only [List.range'_succ, List.nodup_cons]
      constructor
      · intro member
        have bounds := List.mem_range'_1.mp member
        omega
      · exact ih (start + 1)

private theorem ordinal_range_nodup (count : Nat) : (List.range count).Nodup := by
  simpa [List.range_eq_range'] using finite_interval_nodup 0 count

def product (xs : List A) (ys : List B) (zs : List C) : List (A × B × C) :=
  xs.flatMap fun a => ys.flatMap fun b => zs.map fun c => (a, b, c)

theorem mem_product (xs : List A) (ys : List B) (zs : List C) (a : A) (b : B) (c : C) :
    (a, b, c) ∈ product xs ys zs ↔ a ∈ xs ∧ b ∈ ys ∧ c ∈ zs := by
  simp only [product, List.mem_flatMap, List.mem_map, Prod.mk.injEq]
  constructor
  · rintro ⟨a', ha, b', hb, c', hc, ea, eb, ec⟩
    subst a'; subst b'; subst c'
    exact ⟨ha, hb, hc⟩
  · rintro ⟨ha, hb, hc⟩
    exact ⟨a, ha, b, hb, c, hc, rfl, rfl, rfl⟩

theorem nodup_fibers (indices : List I) (rows : I → List X) (key : X → I)
    (unique : indices.Nodup) (each : ∀ i, (rows i).Nodup)
    (belongs : ∀ i a, a ∈ rows i → key a = i) : (indices.flatMap rows).Nodup := by
  apply List.pairwise_flatMap.mpr
  refine ⟨fun i _ => each i, unique.imp ?_⟩
  intro i j different a ha b hb equal
  apply different
  exact (belongs i a ha).symm.trans ((congrArg key equal).trans (belongs j b hb))

theorem product_nodup (xs : List A) (ys : List B) (zs : List C)
    (hx : xs.Nodup) (hy : ys.Nodup) (hz : zs.Nodup) : (product xs ys zs).Nodup := by
  unfold product
  apply nodup_fibers xs _ (fun t => t.1) hx
  · intro a
    apply nodup_fibers ys _ (fun t => t.2.1) hy
    · intro b
      apply List.pairwise_map.mpr
      apply hz.imp
      intro c d different equal
      exact different (congrArg (fun t : A × B × C => t.2.2) equal)
    · intro b t member
      rcases List.mem_map.mp member with ⟨c, _, equal⟩
      subst t
      rfl
  · intro a t member
    rcases List.mem_flatMap.mp member with ⟨b, _, member⟩
    rcases List.mem_map.mp member with ⟨c, _, equal⟩
    subst t
    rfl

def layer (x : Layers A) (y : Layers B) (z : Layers C) (n : Nat) : List (A × B × C) :=
  (List.range (n + 1)).flatMap fun i =>
    (List.range (n - i + 1)).flatMap fun j =>
      product (x.layer i) (y.layer j) (z.layer (n - i - j))

theorem mem_layer (x : Layers A) (y : Layers B) (z : Layers C)
    (a : A) (b : B) (c : C) (n : Nat) :
    (a, b, c) ∈ layer x y z n ↔
      x.admissible a ∧ y.admissible b ∧ z.admissible c ∧ x.cost a + y.cost b + z.cost c = n := by
  simp only [layer, List.mem_flatMap, List.mem_range, mem_product,
    x.membership, y.membership, z.membership]
  constructor
  · rintro ⟨i, hi, j, hj, ⟨ha, ea⟩, ⟨hb, eb⟩, hc, ec⟩
    exact ⟨ha, hb, hc, by omega⟩
  · rintro ⟨ha, hb, hc, total⟩
    exact ⟨x.cost a, by omega, y.cost b, by omega, ⟨ha, rfl⟩, ⟨hb, rfl⟩, hc, by omega⟩

theorem unique_bucket (x : Layers A) (y : Layers B) (z : Layers C)
    (a : A) (b : B) (c : C) (n m : Nat)
    (first : (a, b, c) ∈ layer x y z n) (second : (a, b, c) ∈ layer x y z m) : n = m := by
  have hn := (mem_layer x y z a b c n).mp first
  have hm := (mem_layer x y z a b c m).mp second
  omega

theorem layer_nodup (x : Layers A) (y : Layers B) (z : Layers C) (n : Nat) :
    (layer x y z n).Nodup := by
  unfold layer
  apply nodup_fibers _ _ (fun t => x.cost t.1) (ordinal_range_nodup _)
  · intro i
    apply nodup_fibers _ _ (fun t => y.cost t.2.1) (ordinal_range_nodup _)
    · intro j
      exact product_nodup _ _ _ (x.unique i) (y.unique j) (z.unique _)
    · intro j ⟨a, b, c⟩ member
      exact ((y.membership b j).mp ((mem_product _ _ _ a b c).mp member).2.1).2
  · intro i ⟨a, b, c⟩ member
    rcases List.mem_flatMap.mp member with ⟨j, _, member⟩
    exact ((x.membership a i).mp ((mem_product _ _ _ a b c).mp member).1).2

def through (x : Layers A) (y : Layers B) (z : Layers C) (n : Nat) : List (A × B × C) :=
  (List.range (n + 1)).flatMap (layer x y z)

theorem mem_through (x : Layers A) (y : Layers B) (z : Layers C)
    (a : A) (b : B) (c : C) (n : Nat) :
    (a, b, c) ∈ through x y z n ↔
      x.admissible a ∧ y.admissible b ∧ z.admissible c ∧ x.cost a + y.cost b + z.cost c ≤ n := by
  simp only [through, List.mem_flatMap, List.mem_range, mem_layer]
  constructor
  · rintro ⟨k, hk, ha, hb, hc, total⟩
    exact ⟨ha, hb, hc, by omega⟩
  · rintro ⟨ha, hb, hc, total⟩
    exact ⟨x.cost a + y.cost b + z.cost c, by omega, ha, hb, hc, rfl⟩

theorem eventually_enumerated (x : Layers A) (y : Layers B) (z : Layers C)
    (a : A) (b : B) (c : C)
    (ha : x.admissible a) (hb : y.admissible b) (hc : z.admissible c) :
    (a, b, c) ∈ through x y z (x.cost a + y.cost b + z.cost c) :=
  (mem_through x y z a b c _).mpr ⟨ha, hb, hc, Nat.le_refl _⟩

theorem through_monotone (x : Layers A) (y : Layers B) (z : Layers C)
    (a : A) (b : B) (c : C) (n m : Nat) (le : n ≤ m)
    (seen : (a, b, c) ∈ through x y z n) : (a, b, c) ∈ through x y z m := by
  rcases (mem_through x y z a b c n).mp seen with ⟨ha, hb, hc, bound⟩
  exact (mem_through x y z a b c m).mpr ⟨ha, hb, hc, Nat.le_trans bound le⟩

theorem through_nodup (x : Layers A) (y : Layers B) (z : Layers C) (n : Nat) :
    (through x y z n).Nodup := by
  apply nodup_fibers _ _ (fun t => x.cost t.1 + y.cost t.2.1 + z.cost t.2.2)
    (ordinal_range_nodup _) (layer_nodup x y z)
  intro i ⟨a, b, c⟩ member
  exact ((mem_layer x y z a b c i).mp member).2.2.2

def finite [DecidableEq A] (values : List A) (unique : values.Nodup) (cost : A → Nat) : Layers A where
  admissible := (· ∈ values)
  cost := cost
  layer := fun n => values.filter (fun a => cost a == n)
  membership := by intro a n; simp
  unique := fun _ => unique.filter _

def clipped (x : Layers A) (keep : A → Bool) : Layers A where
  admissible := fun a => x.admissible a ∧ keep a = true
  cost := x.cost
  layer := fun n => (x.layer n).filter keep
  membership := by intro a n; simp [x.membership, and_assoc, and_comm]
  unique := fun n => (x.unique n).filter _

/-- Cost zero contains only ordinal zero. Positive cost e contains exactly
the original ordinals in [2^(e-1), 2^e); names are not replaced by visit ranks. -/
def dyadicCost (n : Nat) : Nat := if n = 0 then 0 else n.log2 + 1

def dyadicLayer : Nat → List Nat
  | 0 => [0]
  | e + 1 => List.range' (2^e) (2^e)

theorem mem_dyadic (n e : Nat) : n ∈ dyadicLayer e ↔ dyadicCost n = e := by
  cases n with
  | zero =>
      cases e with
      | zero => simp [dyadicLayer, dyadicCost]
      | succ e =>
          have positive : 0 < 2^e := Nat.pow_pos (by decide)
          simp only [dyadicLayer, List.mem_range'_1]
          change (2^e ≤ 0 ∧ 0 < 2^e + 2^e) ↔ 0 = e + 1
          constructor
          · intro member
            exact False.elim (Nat.not_le_of_lt positive member.1)
          · intro impossible
            cases impossible
  | succ n =>
      cases e with
      | zero => simp [dyadicLayer, dyadicCost]
      | succ e =>
          simpa [dyadicLayer, dyadicCost, Nat.pow_succ, Nat.mul_two] using
            (Nat.log2_eq_iff (n := n + 1) (k := e) (by omega)).symm

def dyadic : Layers Nat where
  admissible := fun _ => True
  cost := dyadicCost
  layer := dyadicLayer
  membership := by intro n e; simpa using mem_dyadic n e
  unique := by
    intro e
    cases e with
    | zero => simp [dyadicLayer]
    | succ e => exact finite_interval_nodup _ _

def boundedOrdinals (size : Nat) : Layers Nat := clipped dyadic (fun n => decide (n < size))

theorem bounded_membership (nx ny nz a b c level : Nat) :
    (a, b, c) ∈ through (boundedOrdinals nx) (boundedOrdinals ny) (boundedOrdinals nz) level ↔
      a < nx ∧ b < ny ∧ c < nz ∧ dyadicCost a + dyadicCost b + dyadicCost c ≤ level := by
  simp [mem_through, boundedOrdinals, clipped, dyadic]

theorem dyadic_cost_monotone (a b : Nat) (bound : a ≤ b) : dyadicCost a ≤ dyadicCost b := by
  cases a with
  | zero => simp [dyadicCost]
  | succ a =>
      cases b with
      | zero => omega
      | succ b =>
          have logs : (a + 1).log2 ≤ (b + 1).log2 :=
            (Nat.le_log2 (by omega)).mpr (Nat.le_trans (Nat.log2_self_le (by omega)) bound)
          simpa [dyadicCost] using Nat.add_le_add_right logs 1

theorem finite_exhaustion (nx ny nz a b c : Nat) :
    (a, b, c) ∈ through (boundedOrdinals nx) (boundedOrdinals ny) (boundedOrdinals nz)
      (dyadicCost (nx - 1) + dyadicCost (ny - 1) + dyadicCost (nz - 1)) ↔
      a < nx ∧ b < ny ∧ c < nz := by
  rw [bounded_membership]
  constructor
  · exact fun ⟨ha, hb, hc, _⟩ => ⟨ha, hb, hc⟩
  · rintro ⟨ha, hb, hc⟩
    have ca := dyadic_cost_monotone a (nx - 1) (by omega)
    have cb := dyadic_cost_monotone b (ny - 1) (by omega)
    have cc := dyadic_cost_monotone c (nz - 1) (by omega)
    exact ⟨ha, hb, hc, by omega⟩

end AdditiveTraversal
