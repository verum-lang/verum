import Std

/-! Callable bound algebra: one Boolean membership function per hazard set.
The native Pure tag represents no hazards. The native correspondence controls
exercise `PropertyRequirement::join` and `implies` in the type-checker crate.
These results do not establish soundness of property inference or the checker.
-/

namespace VerumExternalReplay.PropertyRequirement

-- α ranges over hazards. The native Pure marker denotes the empty hazard set,
-- not a member of α. This models bounds, not property inference or source code.
abbrev HazardSet (α : Type) := α → Bool

inductive Requirement (α : Type) where
  | unrestricted
  | atMost (allowed : HazardSet α)
  | excludes (forbidden : HazardSet α)

open Requirement

def permits {α : Type} : Requirement α → α → Prop
  | unrestricted, _ => True
  | atMost allowed, h => allowed h = true
  | excludes forbidden, h => forbidden h = false

def join {α : Type} : Requirement α → Requirement α → Requirement α
  | unrestricted, _ => unrestricted
  | _, unrestricted => unrestricted
  | atMost a, atMost b => atMost (fun h => a h || b h)
  | excludes a, excludes b => excludes (fun h => a h && b h)
  | excludes a, atMost b => excludes (fun h => a h && !b h)
  | atMost a, excludes b => excludes (fun h => b h && !a h)

-- Each allowed hazard in the join is exactly one allowed by either operand.
theorem join_permits {α : Type} (a b : Requirement α) (h : α) :
    permits (join a b) h ↔ permits a h ∨ permits b h := by
  cases a with
  | unrestricted =>
    cases b <;> exact ⟨fun _ => Or.inl True.intro, fun _ => True.intro⟩
  | atMost a =>
    cases b with
    | unrestricted => exact ⟨fun _ => Or.inr True.intro, fun _ => True.intro⟩
    | atMost b =>
      change ((a h || b h) = true) ↔ (a h = true ∨ b h = true)
      cases ha : a h <;> cases hb : b h <;> decide
    | excludes b =>
      change ((b h && !(a h)) = false) ↔ (a h = true ∨ b h = false)
      cases ha : a h <;> cases hb : b h <;> decide
  | excludes a =>
    cases b with
    | unrestricted => exact ⟨fun _ => Or.inr True.intro, fun _ => True.intro⟩
    | atMost b =>
      change ((a h && !(b h)) = false) ↔ (a h = false ∨ b h = true)
      cases ha : a h <;> cases hb : b h <;> decide
    | excludes b =>
      change ((a h && b h) = false) ↔ (a h = false ∨ b h = false)
      cases ha : a h <;> cases hb : b h <;> decide

def accepts {α : Type} (r : Requirement α) (actual : HazardSet α) : Prop :=
  ∀ h, actual h = true → permits r h

def refines {α : Type} (a b : Requirement α) : Prop :=
  ∀ h, permits a h → permits b h

theorem join_accepts_union {α : Type} (a b : Requirement α)
    (x y : HazardSet α) (hx : accepts a x) (hy : accepts b y) :
    accepts (join a b) (fun h => x h || y h) := by
  intro h hab
  change (x h || y h) = true at hab
  apply (join_permits a b h).mpr
  cases hxv : x h with
  | false =>
    have hyv : y h = true := by
      rw [hxv] at hab
      exact hab
    exact Or.inr (hy h hyv)
  | true => exact Or.inl (hx h hxv)

theorem join_left {α : Type} (a b : Requirement α) : refines a (join a b) := by
  intro h ha
  exact (join_permits a b h).mpr (Or.inl ha)

theorem join_right {α : Type} (a b : Requirement α) : refines b (join a b) := by
  intro h hb
  exact (join_permits a b h).mpr (Or.inr hb)

theorem join_least {α : Type} (a b c : Requirement α)
    (ha : refines a c) (hb : refines b c) : refines (join a b) c := by
  intro h hj
  cases (join_permits a b h).mp hj with
  | inl h => exact ha _ h
  | inr h => exact hb _ h

-- The native implication procedure deliberately declines some entailments.
def certifiedImplies {α : Type} (source target : Requirement α) : Prop :=
  match target with
  | unrestricted => True
  | atMost allowed =>
    match source with
    | unrestricted => False
    | atMost known => accepts (atMost allowed) known
    | excludes _ => False
  | excludes forbidden =>
    match source with
    | unrestricted => ∀ h, forbidden h = false
    | atMost known => accepts (excludes forbidden) known
    | excludes excluded => ∀ h, forbidden h = true → excluded h = true

theorem certified_implies_sound {α : Type} (a b : Requirement α)
    (evidence : certifiedImplies a b) : refines a b := by
  cases a with
  | unrestricted =>
    cases b with
    | unrestricted => intro _ _; exact True.intro
    | atMost _ => exact False.elim evidence
    | excludes _ => intro h _; exact evidence h
  | atMost allowed =>
    cases b with
    | unrestricted => intro _ _; exact True.intro
    | atMost _ => exact evidence
    | excludes _ => exact evidence
  | excludes forbidden =>
    cases b with
    | unrestricted => intro _ _; exact True.intro
    | atMost _ => exact False.elim evidence
    | excludes target =>
      intro h hs
      cases ht : target h with
      | false => exact ht
      | true =>
        have conflict : false = true := hs.symm.trans (evidence h ht)
        exact Bool.noConfusion conflict

theorem unknown_does_not_establish_pure {α : Type} (hazard : α) :
    ¬ refines unrestricted (atMost (fun (_ : α) => false)) := by
  intro evidence
  have contradiction : false = true := evidence hazard True.intro
  cases contradiction

#print axioms join_permits
#print axioms join_accepts_union
#print axioms join_left
#print axioms join_right
#print axioms join_least
#print axioms certified_implies_sound
#print axioms unknown_does_not_establish_pure
end VerumExternalReplay.PropertyRequirement
