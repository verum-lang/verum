/- Kernel replay of the empty carrier and witness contracts.
   This models the rules; it is not a proof of the Rust checker's implementation. -/
namespace VerumBottom
universe u
abbrev Void := Empty
structure Proof (P : Sort u) where
  witness : P

def absurd {A : Sort u} (v : Void) : A := nomatch v

theorem noVoid : ¬ Nonempty Void := by
  intro h
  cases h with
  | intro v => exact absurd v

theorem proofHasWitness {P : Sort u} : Nonempty (Proof P) ↔ Nonempty P := by
  constructor
  · intro h
    cases h with
    | intro p => exact ⟨p.witness⟩
  · intro h
    cases h with
    | intro p => exact ⟨⟨p⟩⟩

theorem noUnitToVoid : ¬ Nonempty (Unit → Void) := by
  intro h
  cases h with
  | intro f => exact absurd (f ())

#print axioms noVoid
#print axioms proofHasWitness
#print axioms noUnitToVoid
end VerumBottom
