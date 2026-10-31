import VerumExternalReplay.TermOperations

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
  | idElim (motive base value : Term) : HeadStep (.idElim motive base (.refl value)) base
  | induction (name : String) (level : Level) (indices : List Term) (constructors : List (DataConstructor Term))
      (ctor : Nat) (args : List Term) (motive : Term) (cases : List Term)
      (fields : DataConstructor Term) (branch : Term)
      (fieldsFound : constructors[ctor]? = some fields) (branchFound : cases[ctor]? = some branch)
      (arity : fields.fields.length = args.length) :
      HeadStep (.induct (.construct (.data name level indices constructors) ctor args) motive cases)
        (applyInductionCase motive cases fields.fields args branch)
  | appHead {a b : Term} (argument : Term) : HeadStep a b → HeadStep (.app a argument) (.app b argument)
  | fstHead {a b : Term} : HeadStep a b → HeadStep (.fst a) (.fst b)
  | sndHead {a b : Term} : HeadStep a b → HeadStep (.snd a) (.snd b)
  | transportHead {a b : Term} (motive base : Term) :
      HeadStep a b → HeadStep (.transport motive base a) (.transport motive base b)
  | idElimHead {a b : Term} (motive base : Term) :
      HeadStep a b → HeadStep (.idElim motive base a) (.idElim motive base b)
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

/-- Every successful result is reached by the explicit reduction rules.
    Exhaustion supplies no term and cannot be used as a normal form. -/
theorem whnfFuel_reduces (fuel : Nat) (term result : Term)
    (completed : whnfFuel fuel term = .ok result) : HeadSteps term result := by
  induction fuel generalizing term result with
  | zero => simp [whnfFuel] at completed
  | succ fuel ih =>
    cases term with
    | var index | «universe» level | data name level indices constructors
      | construct data ctor args | pi a b | lam a b | sigma a b | pair a b
      | identity a b c | refl value | funExt a b c d =>
      simp only [whnfFuel, pure, Except.pure, Except.ok.injEq] at completed
      subst result
      exact .refl _
    | letE value body =>
      simp only [whnfFuel] at completed
      exact .step (.letValue value body) (ih _ _ completed)
    | app f x =>
      cases headEq : whnfFuel fuel f with
      | error error => simp [whnfFuel, headEq, bind, Except.bind] at completed
      | ok head =>
        have path := HeadSteps.lift (fun head => .app head x) (HeadStep.appHead x) (ih _ _ headEq)
        simp only [whnfFuel, headEq, bind, Except.bind, pure, Except.pure] at completed
        split at completed
        next domain body => exact path.trans (.step (.beta domain body x) (ih _ _ completed))
        next other => cases completed; exact path
    | fst value =>
      cases headEq : whnfFuel fuel value with
      | error error => simp [whnfFuel, headEq, bind, Except.bind] at completed
      | ok head =>
        have path := HeadSteps.lift Term.fst HeadStep.fstHead (ih _ _ headEq)
        simp only [whnfFuel, headEq, bind, Except.bind, pure, Except.pure] at completed
        split at completed
        next a b => exact path.trans (.step (.first a b) (ih _ _ completed))
        next other => cases completed; exact path
    | snd value =>
      cases headEq : whnfFuel fuel value with
      | error error => simp [whnfFuel, headEq, bind, Except.bind] at completed
      | ok head =>
        have path := HeadSteps.lift Term.snd HeadStep.sndHead (ih _ _ headEq)
        simp only [whnfFuel, headEq, bind, Except.bind, pure, Except.pure] at completed
        split at completed
        next a b => exact path.trans (.step (.second a b) (ih _ _ completed))
        next other => cases completed; exact path
    | transport motive base value =>
      cases headEq : whnfFuel fuel value with
      | error error => simp [whnfFuel, headEq, bind, Except.bind] at completed
      | ok head =>
        have path := HeadSteps.lift (fun head => .transport motive base head)
          (HeadStep.transportHead motive base) (ih _ _ headEq)
        simp only [whnfFuel, headEq, bind, Except.bind, pure, Except.pure] at completed
        split at completed
        next value => exact path.trans (.step (.transport motive base value) (ih _ _ completed))
        next other => cases completed; exact path
    | idElim motive base value =>
      cases headEq : whnfFuel fuel value with
      | error error => simp [whnfFuel, headEq, bind, Except.bind] at completed
      | ok head =>
        have path := HeadSteps.lift (fun head => .idElim motive base head)
          (HeadStep.idElimHead motive base) (ih _ _ headEq)
        simp only [whnfFuel, headEq, bind, Except.bind, pure, Except.pure] at completed
        split at completed
        next value => exact path.trans (.step (.idElim motive base value) (ih _ _ completed))
        next other => cases completed; exact path
    | induct value motive cases =>
      cases headEq : whnfFuel fuel value with
      | error error => simp [whnfFuel, headEq, bind, Except.bind] at completed
      | ok head =>
        have path := HeadSteps.lift (fun head => .induct head motive cases)
          (HeadStep.inductHead motive cases) (ih _ _ headEq)
        simp only [whnfFuel, headEq, bind, Except.bind, pure, Except.pure] at completed
        split at completed
        next data ctor args =>
          cases declarationEq : whnfFuel fuel data with
          | error error => simp [declarationEq] at completed
          | ok declaration =>
            have declarationPath := HeadSteps.lift
              (fun declaration => .induct (.construct declaration ctor args) motive cases)
              (HeadStep.inductDeclaration ctor args motive cases) (ih _ _ declarationEq)
            simp only [declarationEq] at completed
            split at completed
            next name level indices constructors =>
              split at completed
              next fields branch fieldsEq branchEq =>
                split at completed
                next arity =>
                  exact path.trans (declarationPath.trans (.step
                    (.induction name level indices constructors ctor args motive cases fields branch fieldsEq branchEq
                      (by simpa only [beq_iff_eq] using arity)) (ih _ _ completed)))
                next => cases completed; exact path
              next => cases completed; exact path
            next => cases completed; exact path
        next other => cases completed; exact path

theorem whnf_reduces (term result : Term) (completed : whnf term = .ok result) :
    HeadSteps term result := whnfFuel_reduces whnfFuelCeiling term result completed

/-- If no primitive/context step is available, a finite reduction can only
    return the input. The relation cannot grant arbitrary results by reflexivity. -/
theorem HeadSteps.normal_unique {term result : Term}
    (normal : ∀ next, ¬ HeadStep term next) (steps : HeadSteps term result) : result = term := by
  cases steps with
  | refl => rfl
  | step edge rest => exact False.elim (normal _ edge)

/-- Zero fuel refuses even a reducible term. An operational failure is
    not a completed normalization or a proof that the term is ill-typed. -/
theorem zero_fuel_refuses_beta_redex (domain body argument : Term) :
    whnfFuel 0 (.app (.lam domain body) argument) = .error .fuel_exhausted ∧
    HeadStep (.app (.lam domain body) argument) (subst 0 argument body) := by
  constructor
  · simp only [whnfFuel]
  · exact .beta domain body argument

/-- info: 'VerumKernel.whnfFuel_reduces' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms whnfFuel_reduces
/-- info: 'VerumKernel.whnf_reduces' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms whnf_reduces
/-- info: 'VerumKernel.HeadSteps.normal_unique' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms HeadSteps.normal_unique
/-- info: 'VerumKernel.zero_fuel_refuses_beta_redex' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms zero_fuel_refuses_beta_redex
end VerumKernel
