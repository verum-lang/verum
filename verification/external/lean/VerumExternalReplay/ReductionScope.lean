import VerumExternalReplay.TermScope

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

/-- Every completed or fuel-truncated reduction preserves scope. The theorem
    covers every term constructor and every fuel amount, without typing premises. -/
theorem scope_whnfFuel (fuel bound : Nat) (term : Term)
    (good : scopeError bound term = none) :
    scopeError bound (whnfFuel fuel term) = none := by
  induction fuel generalizing term with
  | zero => simpa only [whnfFuel] using good
  | succ fuel ih =>
    cases term with
    | var index => simpa only [whnfFuel] using good
    | «universe» level => simpa only [whnfFuel] using good
    | data name level indices constructors => simpa only [whnfFuel] using good
    | construct data ctor args => simpa only [whnfFuel] using good
    | pi a b => simpa only [whnfFuel] using good
    | lam a b => simpa only [whnfFuel] using good
    | sigma a b => simpa only [whnfFuel] using good
    | pair a b => simpa only [whnfFuel] using good
    | identity a b c => simpa only [whnfFuel] using good
    | refl value => simpa only [whnfFuel] using good
    | letE value body =>
      have parts : scopeError bound value = none ∧ scopeError (bound + 1) body = none := by
        simpa only [scopeError, Option.or_eq_none_iff] using good
      simpa only [whnfFuel] using ih (subst 0 value body) (scope_subst bound 0 value body parts.1 parts.2)
    | app f x =>
      have parts : scopeError bound f = none ∧ scopeError bound x = none := by
        simpa only [scopeError, Option.or_eq_none_iff] using good
      have headGood := ih f parts.1
      simp only [whnfFuel]
      split
      next domain body equation =>
        simp only [equation, scopeError, Option.or_eq_none_iff] at headGood
        exact ih (subst 0 x body) (scope_subst bound 0 x body parts.2 headGood.2)
      next other equation =>
        simp only [scopeError, Option.or_eq_none_iff]
        exact ⟨by simpa only [equation] using headGood, parts.2⟩
    | fst value =>
      have headGood := ih value (by simpa only [scopeError] using good)
      simp only [whnfFuel]
      split
      next a b equation =>
        simp only [equation, scopeError, Option.or_eq_none_iff] at headGood
        exact ih a headGood.1
      next other equation =>
        simpa only [scopeError, equation] using headGood
    | snd value =>
      have headGood := ih value (by simpa only [scopeError] using good)
      simp only [whnfFuel]
      split
      next a b equation =>
        simp only [equation, scopeError, Option.or_eq_none_iff] at headGood
        exact ih b headGood.2
      next other equation =>
        simpa only [scopeError, equation] using headGood
    | transport motive base path =>
      have parts : scopeError bound motive = none ∧ scopeError bound base = none ∧
          scopeError bound path = none := by
        simpa only [scopeError, Option.or_eq_none_iff] using good
      have headGood := ih path parts.2.2
      simp only [whnfFuel]
      split
      next value equation => exact ih base parts.2.1
      next other equation =>
        simp only [scopeError, Option.or_eq_none_iff]
        exact ⟨parts.1, parts.2.1, by simpa only [equation] using headGood⟩
    | induct value motive cases =>
      have parts : scopeError bound value = none ∧ scopeError bound motive = none ∧
          scopeTerms bound cases = none := by
        simpa only [scopeError, Option.or_eq_none_iff] using good
      have headGood := ih value parts.1
      have neutralGood : scopeError bound (.induct (whnfFuel fuel value) motive cases) = none := by
        simpa only [scopeError, Option.or_eq_none_iff] using And.intro headGood parts.2
      simp only [whnfFuel]
      split
      next data ctor args headEq =>
        have constructorGood : scopeError bound data = none ∧ scopeTerms bound args = none := by
          simpa only [headEq, scopeError, Option.or_eq_none_iff] using headGood
        have argsGood := constructorGood.2
        split
        next name level indices constructors declarationEq =>
          split
          next fields branch fieldsEq branchEq =>
            split
            next arity =>
              exact ih _ (scope_applyInductionCase bound motive cases fields.fields args branch
                parts.2.1 parts.2.2 argsGood
                (scopeTerms_member bound cases parts.2.2 branch (List.mem_of_getElem? branchEq)))
            next arity => simpa only [headEq] using neutralGood
          next => simpa only [headEq] using neutralGood
        next => simpa only [headEq] using neutralGood
      next other headEq => simpa only [headEq] using neutralGood

theorem scope_whnf (bound : Nat) (term : Term) (good : scopeError bound term = none) :
    scopeError bound (whnf term) = none := scope_whnfFuel whnfFuelCeiling bound term good

/-- info: 'VerumKernel.applyInductionCase_eq_fold' depends on axioms: [propext] -/
#guard_msgs in
#print axioms applyInductionCase_eq_fold
/-- info: 'VerumKernel.scope_applyInductionCase' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms scope_applyInductionCase
/-- info: 'VerumKernel.scope_whnfFuel' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms scope_whnfFuel
/-- info: 'VerumKernel.scope_whnf' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms scope_whnf
end VerumKernel
