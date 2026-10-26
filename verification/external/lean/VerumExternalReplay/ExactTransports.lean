import Std.Tactic

/-! Composition of supplied exact one-step partial-execution contracts.
These theorems do not certify acquisition grammars, a learner, an implementation,
or empirical generalization. Definedness is the `Option` tag, not an IO effect.
-/
namespace ExactTransports
set_option warningAsError true
universe u v w
variable {A : Type u} {B : Type v} {C : Type w}

theorem compose (f : A → Option A) (g : B → Option B) (h : C → Option C)
    (i : A → B) (j : B → C)
    (first : ∀ x, g (i x) = (f x).map i)
    (second : ∀ y, h (j y) = (g y).map j) (x : A) :
    h (j (i x)) = (f x).map (j ∘ i) := by
  rw [second, first, Option.map_map]

theorem definedness_reflected (f : A → Option A) (g : B → Option B) (i : A → B)
    (exact : ∀ x, g (i x) = (f x).map i) (x : A) :
    (g (i x)).isSome = (f x).isSome := by
  rw [exact]
  simp

theorem undefinedness_reflected (f : A → Option A) (g : B → Option B) (i : A → B)
    (exact : ∀ x, g (i x) = (f x).map i) (x : A) :
    g (i x) = none ↔ f x = none := by
  rw [exact]
  simp

theorem injective_compose (i : A → B) (j : B → C)
    (first : Function.Injective i) (second : Function.Injective j) :
    Function.Injective (j ∘ i) := by
  intro a b equal
  exact first (second equal)

/-- info: 'ExactTransports.compose' depends on axioms: [propext] -/
#guard_msgs in
#print axioms compose
/-- info: 'ExactTransports.definedness_reflected' depends on axioms: [propext] -/
#guard_msgs in
#print axioms definedness_reflected
/-- info: 'ExactTransports.undefinedness_reflected' depends on axioms: [propext] -/
#guard_msgs in
#print axioms undefinedness_reflected
/-- info: 'ExactTransports.injective_compose' does not depend on any axioms -/
#guard_msgs in
#print axioms injective_compose
end ExactTransports
