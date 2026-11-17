/- The post-normalization refinement admission boundary. Type inference,
normalization, and computation of the two depth inputs are external premises.
No theorem here claims correctness of those producers or of the whole checker. -/
namespace VerumRefinementAdmission
inductive PredicateType where
  | proposition
  | other
  deriving DecidableEq
inductive Verdict (Level : Type) where
  | accepted (level : Level)
  | invalidBase
  | depthViolation
  | invalidPredicate
  deriving DecidableEq

def admit {Level : Type} (base : Option Level) (baseDepth predicateDepth : Nat)
    (predicateType : PredicateType) : Verdict Level :=
  match base with
  | none => .invalidBase
  | some level =>
    if predicateDepth > baseDepth then .depthViolation
    else match predicateType with
    | .proposition => .accepted level
    | .other => .invalidPredicate

theorem accepted_iff {Level : Type} (base : Option Level) (level : Level)
    (baseDepth predicateDepth : Nat) (predicateType : PredicateType) :
    admit base baseDepth predicateDepth predicateType = .accepted level ↔
      base = some level ∧ predicateDepth ≤ baseDepth ∧ predicateType = .proposition := by
  cases base with
  | none =>
    constructor
    · intro h; cases h
    · intro h; cases h.1
  | some actual =>
    change (if predicateDepth > baseDepth then Verdict.depthViolation else
      match predicateType with
      | .proposition => Verdict.accepted actual
      | .other => Verdict.invalidPredicate) = Verdict.accepted level ↔ _
    by_cases hdepth : predicateDepth > baseDepth
    · rw [ite_eq_left hdepth]
      constructor
      · intro h; cases h
      · intro h; exact False.elim ((Nat.not_le_of_gt hdepth) h.2.1)
    · rw [ite_eq_right hdepth]
      have bound : predicateDepth ≤ baseDepth := Nat.le_of_not_gt hdepth
      cases predicateType with
      | proposition =>
        constructor
        · intro h
          cases h
          exact ⟨rfl, bound, rfl⟩
        · intro h
          cases Option.some.inj h.1
          rfl
      | other =>
        constructor
        · intro h; cases h
        · intro h; cases h.2.2

example : admit (some 7) 4 4 .proposition = .accepted 7 := rfl
example : admit (some 7) 4 4 .other = .invalidPredicate := rfl
example : admit (some 7) 4 5 .proposition = .depthViolation := rfl
example : admit (none : Option Nat) 4 4 .proposition = .invalidBase := rfl
#print axioms accepted_iff
end VerumRefinementAdmission
