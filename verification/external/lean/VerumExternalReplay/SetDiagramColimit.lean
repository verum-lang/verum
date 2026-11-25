namespace Verum.SetDiagramColimit

universe u v w z
variable {I : Type u} {Arrow : I → I → Type v}

/-- A family of sets and the maps assigned to its generating arrows. -/
structure Diagram (I : Type u) (Arrow : I → I → Type v) where
  fibre : I → Type w
  map : {i j : I} → Arrow i j → fibre i → fibre j

variable (D : Diagram I Arrow)

/-- Relations include every source element and every generating arrow. -/
inductive Step : (Σ i, D.fibre i) → (Σ i, D.fibre i) → Prop where
  | edge {i j} (a : Arrow i j) (x : D.fibre i) :
      Step ⟨i, x⟩ ⟨j, D.map a x⟩

abbrev Apex := Quot (Step D)

def point (i : I) (x : D.fibre i) : Apex D := Quot.mk _ ⟨i, x⟩

theorem point_commutes {i j} (a : Arrow i j) (x : D.fibre i) :
    point D i x = point D j (D.map a x) :=
  Quot.sound (Step.edge a x)

structure Cocone (Y : Type z) where
  leg : (i : I) → D.fibre i → Y
  commutes : ∀ {i j} (a : Arrow i j) (x : D.fibre i), leg i x = leg j (D.map a x)

variable {Y : Type z}

def descend (c : Cocone D Y) : Apex D → Y :=
  Quot.lift (fun p => c.leg p.1 p.2) (by
    intro p q h
    cases h with
    | edge a x => exact c.commutes a x)

theorem descend_point (c : Cocone D Y) (i : I) (x : D.fibre i) :
    descend D c (point D i x) = c.leg i x := rfl

theorem determined_by_points (f g : Apex D → Y)
    (h : ∀ i x, f (point D i x) = g (point D i x)) (q : Apex D) : f q = g q :=
  Quot.inductionOn q (fun p => h p.1 p.2)

theorem descend_unique (c : Cocone D Y) (f : Apex D → Y)
    (h : ∀ i x, f (point D i x) = c.leg i x) (q : Apex D) :
    f q = descend D c q :=
  determined_by_points D f (descend D c) h q

/-- A family distinguishing the ends of one generator cannot factor. -/
theorem incompatible_legs {i j} (a : Arrow i j) (x : D.fibre i)
    (legs : (k : I) → D.fibre k → Y) (separates : legs i x ≠ legs j (D.map a x)) :
    ¬ ∃ f : Apex D → Y, ∀ k y, f (point D k y) = legs k y := by
  intro ⟨f, h⟩
  exact separates ((h i x).symm.trans ((congrArg f (point_commutes D a x)).trans (h j (D.map a x))))

#print axioms point_commutes
#print axioms descend_point
#print axioms determined_by_points
#print axioms descend_unique
#print axioms incompatible_legs
end Verum.SetDiagramColimit
