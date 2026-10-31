import VerumExternalReplay.TermScope
import VerumExternalReplay.ReductionSemantics

/-! Scope preservation for the actual fuel-bounded reference reducer.
The statements concern all scoped terms, including terms that are not typable.
They do not establish typing preservation, normalization or Rust correspondence.
-/
namespace VerumKernel
set_option warningAsError true

/-- The allocation-free branch application retains the exact former semantics,
    including malformed arity truncation and recursive-hypothesis ordering. -/
theorem applyInductionCase_eq_fold (motive : Term) (cases : List Term)
    (fields : List (DataField Term)) (args : List Term) (branch : Term) :
    applyInductionCase motive cases fields args branch =
      (fields.zip args).foldl (fun body (field, arg) =>
        match field with
        | .recursive | .recursiveAt _ => .app (.app body arg) (.induct arg motive cases)
        | .value _ | .dependent _ => .app body arg) branch := by
  induction fields generalizing args branch with
  | nil => simp [applyInductionCase]
  | cons field rest ih =>
    cases args with
    | nil => simp [applyInductionCase]
    | cons arg tail =>
      cases field <;> simp [applyInductionCase, ih]

theorem scopeTerms_member (bound : Nat) (terms : List Term)
    (good : scopeTerms bound terms = none) (term : Term) (member : term ∈ terms) :
    scopeError bound term = none := by
  induction terms with
  | nil => simp at member
  | cons head tail ih =>
    have parts : scopeError bound head = none ∧ scopeTerms bound tail = none := by
      simpa only [scopeTerms, Option.or_eq_none_iff] using good
    rcases List.mem_cons.mp member with equal | rest
    · simpa only [equal] using parts.1
    · exact ih parts.2 rest

theorem scope_applyInductionCase (bound : Nat) (motive : Term) (cases : List Term)
    (fields : List (DataField Term)) (args : List Term) (branch : Term)
    (motiveGood : scopeError bound motive = none) (casesGood : scopeTerms bound cases = none)
    (argsGood : scopeTerms bound args = none) (branchGood : scopeError bound branch = none) :
    scopeError bound (applyInductionCase motive cases fields args branch) = none := by
  induction fields generalizing args branch with
  | nil => simpa only [applyInductionCase] using branchGood
  | cons field rest ih =>
    cases args with
    | nil => simpa only [applyInductionCase] using branchGood
    | cons arg tail =>
      have parts : scopeError bound arg = none ∧ scopeTerms bound tail = none := by
        simpa only [scopeTerms, Option.or_eq_none_iff] using argsGood
      cases field with
      | recursive | recursiveAt _ =>
        apply ih tail _ parts.2
        simp only [scopeError, branchGood, parts.1, motiveGood, casesGood, Option.or_none]
      | value ty | dependent ty =>
        apply ih tail _ parts.2
        simp only [scopeError, branchGood, parts.1, Option.or_none]

/-- Scope preservation follows from the operational rules, independently of
    the evaluator's traversal and resource budget. -/
theorem HeadStep.preserves_scope {term result : Term} (step : HeadStep term result)
    (bound : Nat) (good : scopeError bound term = none) : scopeError bound result = none := by
  induction step with
  | beta domain body argument =>
    simp only [scopeError, Option.or_eq_none_iff] at good
    exact scope_subst bound 0 argument body good.2 good.1.2
  | letValue value body =>
    simp only [scopeError, Option.or_eq_none_iff] at good
    exact scope_subst bound 0 value body good.1 good.2
  | first a b =>
    simp only [scopeError, Option.or_eq_none_iff] at good
    exact good.1
  | second a b =>
    simp only [scopeError, Option.or_eq_none_iff] at good
    exact good.2
  | transport motive base value | idElim motive base value =>
    simp only [scopeError, Option.or_eq_none_iff] at good
    exact good.2.1
  | induction name level indices constructors ctor args motive cases fields branch fieldsFound branchFound arity =>
    simp only [scopeError, Option.or_eq_none_iff] at good
    exact scope_applyInductionCase bound motive cases fields.fields args branch
      good.2.1 good.2.2 good.1.2
      (scopeTerms_member bound cases good.2.2 branch (List.mem_of_getElem? branchFound))
  | appHead argument step ih =>
    simp only [scopeError, Option.or_eq_none_iff] at good ⊢
    exact ⟨ih good.1, good.2⟩
  | fstHead step ih | sndHead step ih =>
    simp only [scopeError] at good ⊢
    exact ih good
  | transportHead motive base step ih | idElimHead motive base step ih =>
    simp only [scopeError, Option.or_eq_none_iff] at good ⊢
    exact ⟨good.1, good.2.1, ih good.2.2⟩
  | inductHead motive cases step ih =>
    simp only [scopeError, Option.or_eq_none_iff] at good ⊢
    exact ⟨ih good.1, good.2⟩
  | inductDeclaration ctor args motive cases step ih =>
    simp only [scopeError, Option.or_eq_none_iff] at good ⊢
    exact ⟨⟨ih good.1.1, good.1.2⟩, good.2⟩

theorem HeadSteps.preserves_scope {term result : Term} (steps : HeadSteps term result)
    (bound : Nat) (good : scopeError bound term = none) : scopeError bound result = none := by
  induction steps with
  | refl => exact good
  | step edge rest ih => exact ih (edge.preserves_scope bound good)

/-- Every successful reduction preserves scope, for all scoped inputs including
    untypable terms. Exhaustion has no returned term to use as evidence. -/
theorem scope_whnfFuel (fuel bound : Nat) (term result : Term)
    (good : scopeError bound term = none) (completed : whnfFuel fuel term = .ok result) :
    scopeError bound result = none :=
  (whnfFuel_reduces fuel term result completed).preserves_scope bound good

theorem scope_whnf (bound : Nat) (term result : Term) (good : scopeError bound term = none)
    (completed : whnf term = .ok result) : scopeError bound result = none :=
  scope_whnfFuel whnfFuelCeiling bound term result good completed

/-- info: 'VerumKernel.applyInductionCase_eq_fold' depends on axioms: [propext] -/
#guard_msgs in
#print axioms applyInductionCase_eq_fold
/-- info: 'VerumKernel.scope_applyInductionCase' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms scope_applyInductionCase
/-- info: 'VerumKernel.HeadStep.preserves_scope' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms HeadStep.preserves_scope
/-- info: 'VerumKernel.HeadSteps.preserves_scope' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms HeadSteps.preserves_scope
/-- info: 'VerumKernel.scope_whnfFuel' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms scope_whnfFuel
/-- info: 'VerumKernel.scope_whnf' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms scope_whnf
end VerumKernel
