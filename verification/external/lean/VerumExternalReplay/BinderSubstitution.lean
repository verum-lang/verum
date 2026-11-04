/-
The free-variable law of the binder step used by Type::try_apply_subst.
`body` is the body's free-variable set before its outer binders are removed.
`range v` is the free-variable set of v's simultaneous replacement (or {v}
when v is absent from the substitution). `renamed` represents the selected
binder names; identity names may be retained. Injectivity is separately needed
to preserve bound-variable identity; this free-variable law does not prove it.

This proves the binder algebra under explicit freshness, not the Rust walker,
the allocator, all Type constructors, type soundness, or compiler correctness.
-/
namespace Verum.BinderSubstitution

variable {V : Type}

def renamedBinder (bound : V → Prop) (renamed : V → V) (x : V) : Prop :=
  ∃ b, bound b ∧ x = renamed b

def substitutedBody (body bound : V → Prop) (range : V → V → Prop)
    (renamed : V → V) (x : V) : Prop :=
  ∃ v, body v ∧ ((bound v ∧ x = renamed v) ∨ (¬bound v ∧ range v x))

def substitutedFree (body bound : V → Prop) (range : V → V → Prop)
    (x : V) : Prop :=
  ∃ v, body v ∧ ¬bound v ∧ range v x

/-- Renaming bound occurrences and substituting free ones simultaneously
preserves exactly the replacement free variables, provided no selected binder
name occurs free in a replacement that is actually used. -/
theorem binder_free_variable_law
    (body bound : V → Prop) (range : V → V → Prop) (renamed : V → V)
    (fresh : ∀ v, body v → ¬bound v → ∀ b, bound b → ¬range v (renamed b))
    (x : V) :
    (substitutedBody body bound range renamed x ∧ ¬renamedBinder bound renamed x)
      ↔ substitutedFree body bound range x := by
  constructor
  · rintro ⟨⟨v, hv, hvbound | hvfree⟩, hnotbound⟩
    · exact False.elim (hnotbound ⟨v, hvbound.1, hvbound.2⟩)
    · exact ⟨v, hv, hvfree.1, hvfree.2⟩
  · rintro ⟨v, hv, hvfree, hrange⟩
    refine ⟨⟨v, hv, Or.inr ⟨hvfree, hrange⟩⟩, ?_⟩
    rintro ⟨b, hb, rfl⟩
    exact fresh v hv hvfree b hb hrange

/-- Existing binder IDs need not change when the used replacement ranges
already exclude them. This is the no-renaming path of the implementation. -/
theorem unchanged_binders_when_fresh
    (body bound : V → Prop) (range : V → V → Prop)
    (fresh : ∀ v, body v → ¬bound v → ∀ b, bound b → ¬range v b)
    (x : V) :
    (substitutedBody body bound range id x ∧ ¬bound x)
      ↔ substitutedFree body bound range x := by
  have h := binder_free_variable_law body bound range id fresh x
  constructor
  · rintro ⟨hb, hn⟩
    apply h.mp
    refine ⟨hb, ?_⟩
    rintro ⟨b, hbound, rfl⟩
    exact hn hbound
  · intro hf
    obtain ⟨hb, hn⟩ := h.mpr hf
    refine ⟨hb, ?_⟩
    intro hbound
    exact hn ⟨x, hbound, rfl⟩

/-- The concrete native counterexample: replacing α by free β in the body of
∀β.(α,β) loses β if the binder remains β, but retains it after freshening. -/
def exampleBody (v : Nat) : Prop := v = 0 ∨ v = 1
def exampleBound (v : Nat) : Prop := v = 1
def exampleRange (v x : Nat) : Prop := (v = 0 ∧ x = 1) ∨ (v ≠ 0 ∧ x = v)

theorem naive_binder_captures :
    substitutedFree exampleBody exampleBound exampleRange 1 ∧
    ¬(substitutedBody exampleBody exampleBound exampleRange id 1 ∧
       ¬renamedBinder exampleBound id 1) := by
  constructor
  · exact ⟨0, Or.inl rfl, by intro h; exact Nat.noConfusion h, Or.inl ⟨rfl, rfl⟩⟩
  · rintro ⟨_, h⟩
    exact h ⟨1, rfl, rfl⟩

theorem fresh_binder_preserves_replacement :
    substitutedBody exampleBody exampleBound exampleRange (fun _ => 2) 1 ∧
    ¬renamedBinder exampleBound (fun _ => 2) 1 := by
  constructor
  · exact ⟨0, Or.inl rfl, Or.inr ⟨by intro h; exact Nat.noConfusion h, Or.inl ⟨rfl, rfl⟩⟩⟩
  · rintro ⟨_, _, h⟩
    exact (by decide : (1 : Nat) ≠ 2) h

#print axioms binder_free_variable_law
#print axioms unchanged_binders_when_fresh
#print axioms naive_binder_captures
#print axioms fresh_binder_preserves_replacement

end Verum.BinderSubstitution
