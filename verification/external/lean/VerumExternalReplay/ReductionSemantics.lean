import VerumExternalReplay.ReductionScope

/-! Operational correspondence for the executable reference reducer.
Rules specify primitive contractions and the head contexts in which they run.
No rule grants a typing judgment or assumes a checker result to be sound.
-/
namespace VerumKernel
set_option warningAsError true

/-- One permitted computational step. This relation is independent of the
    fuel-driven evaluator: neither `whnf` nor `whnfFuel` occurs in its rules. -/
inductive HeadStep : Term → Term → Prop where
  | beta (domain body argument : Term) :
      HeadStep (.app (.lam domain body) argument) (subst 0 argument body)
  | letValue (value body : Term) : HeadStep (.letE value body) (subst 0 value body)
  | first (a b : Term) : HeadStep (.fst (.pair a b)) a
  | second (a b : Term) : HeadStep (.snd (.pair a b)) b
  | transport (motive base value : Term) : HeadStep (.transport motive base (.refl value)) base
  | induction (name : String) (level : Level) (constructors : List (List (Option Term)))
      (ctor : Nat) (args : List Term) (motive : Term) (cases : List Term)
      (fields : List (Option Term)) (branch : Term)
      (fieldsFound : constructors[ctor]? = some fields) (branchFound : cases[ctor]? = some branch)
      (arity : fields.length = args.length) :
      HeadStep (.induct (.construct (.data name level constructors) ctor args) motive cases)
        (applyInductionCase motive cases fields args branch)
  | appHead {a b : Term} (argument : Term) : HeadStep a b → HeadStep (.app a argument) (.app b argument)
  | fstHead {a b : Term} : HeadStep a b → HeadStep (.fst a) (.fst b)
  | sndHead {a b : Term} : HeadStep a b → HeadStep (.snd a) (.snd b)
  | transportHead {a b : Term} (motive base : Term) :
      HeadStep a b → HeadStep (.transport motive base a) (.transport motive base b)
  | inductHead {a b : Term} (motive : Term) (cases : List Term) :
      HeadStep a b → HeadStep (.induct a motive cases) (.induct b motive cases)
  | inductDeclaration {a b : Term} (ctor : Nat) (args : List Term) (motive : Term) (cases : List Term) :
      HeadStep a b → HeadStep (.induct (.construct a ctor args) motive cases)
        (.induct (.construct b ctor args) motive cases)

/-- A finite sequence of permitted steps, possibly empty. -/
inductive HeadSteps : Term → Term → Prop where
  | refl (term : Term) : HeadSteps term term
  | step {a b c : Term} : HeadStep a b → HeadSteps b c → HeadSteps a c

theorem HeadSteps.trans {a b c : Term} (first : HeadSteps a b) (second : HeadSteps b c) :
    HeadSteps a c := by
  induction first with
  | refl => exact second
  | step edge rest ih => exact .step edge (ih second)

theorem HeadSteps.lift (context : Term → Term)
    (preserve : ∀ {a b}, HeadStep a b → HeadStep (context a) (context b))
    {a b : Term} (steps : HeadSteps a b) : HeadSteps (context a) (context b) := by
  induction steps with
  | refl => exact .refl _
  | step edge rest ih => exact .step (preserve edge) ih

/-- Every result returned by the evaluator is reached by the explicit rules.
    This includes zero fuel, truncated computations and neutral/stuck heads. -/
theorem whnfFuel_reduces (fuel : Nat) (term : Term) : HeadSteps term (whnfFuel fuel term) := by
  induction fuel generalizing term with
  | zero => simpa only [whnfFuel] using HeadSteps.refl term
  | succ fuel ih =>
    cases term with
    | var index =>
      simp only [whnfFuel]
      exact .refl _
    | «universe» level =>
      simp only [whnfFuel]
      exact .refl _
    | data name level constructors =>
      simp only [whnfFuel]
      exact .refl _
    | construct data ctor args =>
      simp only [whnfFuel]
      exact .refl _
    | pi a b =>
      simp only [whnfFuel]
      exact .refl _
    | lam a b =>
      simp only [whnfFuel]
      exact .refl _
    | sigma a b =>
      simp only [whnfFuel]
      exact .refl _
    | pair a b =>
      simp only [whnfFuel]
      exact .refl _
    | identity a b c =>
      simp only [whnfFuel]
      exact .refl _
    | refl value =>
      simp only [whnfFuel]
      exact .refl _
    | letE value body =>
      simp only [whnfFuel]
      exact .step (.letValue value body) (ih (subst 0 value body))
    | app f x =>
      have headPath := HeadSteps.lift (fun head => .app head x) (HeadStep.appHead x) (ih f)
      simp only [whnfFuel]
      split
      next domain body equation =>
        have path : HeadSteps (.app f x) (.app (.lam domain body) x) := by
          simpa only [equation] using headPath
        exact path.trans (.step (.beta domain body x) (ih (subst 0 x body)))
      next other equation => simpa only [equation] using headPath
    | fst value =>
      have headPath := HeadSteps.lift Term.fst HeadStep.fstHead (ih value)
      simp only [whnfFuel]
      split
      next a b equation =>
        have path : HeadSteps (.fst value) (.fst (.pair a b)) := by
          simpa only [equation] using headPath
        exact path.trans (.step (.first a b) (ih a))
      next other equation => simpa only [equation] using headPath
    | snd value =>
      have headPath := HeadSteps.lift Term.snd HeadStep.sndHead (ih value)
      simp only [whnfFuel]
      split
      next a b equation =>
        have path : HeadSteps (.snd value) (.snd (.pair a b)) := by
          simpa only [equation] using headPath
        exact path.trans (.step (.second a b) (ih b))
      next other equation => simpa only [equation] using headPath
    | transport motive base path =>
      have headPath := HeadSteps.lift (fun head => .transport motive base head)
        (HeadStep.transportHead motive base) (ih path)
      simp only [whnfFuel]
      split
      next value equation =>
        have steps : HeadSteps (.transport motive base path) (.transport motive base (.refl value)) := by
          simpa only [equation] using headPath
        exact steps.trans (.step (.transport motive base value) (ih base))
      next other equation => simpa only [equation] using headPath
    | induct value motive cases =>
      have headPath := HeadSteps.lift (fun head => .induct head motive cases)
        (HeadStep.inductHead motive cases) (ih value)
      simp only [whnfFuel]
      split
      next data ctor args headEq =>
        have constructorPath : HeadSteps (.induct value motive cases) (.induct (.construct data ctor args) motive cases) := by
          simpa only [headEq] using headPath
        have declarationPath := HeadSteps.lift (fun declaration => .induct (.construct declaration ctor args) motive cases)
          (HeadStep.inductDeclaration ctor args motive cases) (ih data)
        split
        next name level constructors declarationEq =>
          split
          next fields branch fieldsEq branchEq =>
            split
            next arity =>
              have path : HeadSteps (.induct (.construct data ctor args) motive cases)
                  (.induct (.construct (.data name level constructors) ctor args) motive cases) := by
                simpa only [declarationEq] using declarationPath
              exact constructorPath.trans (path.trans (.step
                (.induction name level constructors ctor args motive cases fields branch fieldsEq branchEq
                  (by simpa only [beq_iff_eq] using arity))
                (ih (applyInductionCase motive cases fields args branch))))
            next arity => exact headPath
          next => exact headPath
        next => exact headPath
      next other equation => simpa only [equation] using headPath

theorem whnf_reduces (term : Term) : HeadSteps term (whnf term) :=
  whnfFuel_reduces whnfFuelCeiling term

/-- If no primitive/context step is available, a finite reduction can only
    return the input. The relation cannot grant arbitrary results by reflexivity. -/
theorem HeadSteps.normal_unique {term result : Term}
    (normal : ∀ next, ¬ HeadStep term next) (steps : HeadSteps term result) : result = term := by
  cases steps with
  | refl => rfl
  | step edge rest => exact False.elim (normal _ edge)

/-- Fuel exhaustion does not establish normality: zero fuel leaves even an
    explicit beta redex untouched, and that redex still has a permitted step. -/
theorem zero_fuel_retains_beta_redex (domain body argument : Term) :
    whnfFuel 0 (.app (.lam domain body) argument) = .app (.lam domain body) argument ∧
    HeadStep (whnfFuel 0 (.app (.lam domain body) argument)) (subst 0 argument body) := by
  simp only [whnfFuel]
  exact ⟨True.intro, .beta domain body argument⟩

/-- info: 'VerumKernel.whnfFuel_reduces' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms whnfFuel_reduces
/-- info: 'VerumKernel.whnf_reduces' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms whnf_reduces
/-- info: 'VerumKernel.HeadSteps.normal_unique' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms HeadSteps.normal_unique
/-- info: 'VerumKernel.zero_fuel_retains_beta_redex' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms zero_fuel_retains_beta_redex
end VerumKernel
