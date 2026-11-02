import Std

/- Independent semantics for declaration-level substitution and call-local
   universe inference. Parameters and metavariables inhabit disjoint namespaces.
   A solver success flag is not an assumption of any theorem in this file. -/
namespace Verum.UniverseInstantiation

inductive Formula where
  | concrete (value : Nat)
  | parameter (id : Nat)
  | metavar (id : Nat)
  | succ (inner : Formula)
  | max (left right : Formula)
  deriving DecidableEq

abbrev Bindings := List (Nat × Formula)
abbrev Assignment := List (Nat × Nat)

def lookup {α : Type} (entries : List (Nat × α)) (id : Nat) : Option α :=
  match entries with
  | [] => none
  | (key, value) :: rest => if id = key then some value else lookup rest id

def denote (parameters metas : Nat → Nat) : Formula → Nat
  | .concrete value => value
  | .parameter id => parameters id
  | .metavar id => metas id
  | .succ inner => denote parameters metas inner + 1
  | .max left right => max (denote parameters metas left) (denote parameters metas right)

def parameterValuation (bindings : Bindings) (parameters metas : Nat → Nat) (id : Nat) : Nat :=
  match lookup bindings id with
  | none => parameters id
  | some replacement => denote parameters metas replacement

def Model (bindings : Bindings) (parameters metas : Nat → Nat) : Prop :=
  ∀ id replacement, lookup bindings id = some replacement →
    metas id = denote parameters metas replacement

/- The fuel bounds validation; it does not bound the mathematical valuations.
   Replacements are visited with rename=false, so a parameter inside a
   replacement is never captured by the declaration substitution. -/
def resolve (parameters metas : Bindings) : Nat → Bool → Formula → Option Formula
  | 0, _, _ => none
  | fuel + 1, rename, source =>
      match source with
      | .concrete value => some (.concrete value)
      | .parameter id =>
          if rename then
            match lookup parameters id with
            | none => some (.parameter id)
            | some replacement => resolve parameters metas fuel false replacement
          else some (.parameter id)
      | .metavar id =>
          match lookup metas id with
          | none => some (.metavar id)
          | some replacement => resolve parameters metas fuel false replacement
      | .succ inner => (resolve parameters metas fuel rename inner).map Formula.succ
      | .max left right => do
          let left ← resolve parameters metas fuel rename left
          let right ← resolve parameters metas fuel rename right
          some (.max left right)

theorem resolve_sound {parameterBindings metaBindings : Bindings}
    {parameters metas : Nat → Nat} (model : Model metaBindings parameters metas)
    {fuel : Nat} {rename : Bool} {source output : Formula}
    (accepted : resolve parameterBindings metaBindings fuel rename source = some output) :
    denote parameters metas output =
      denote (if rename then parameterValuation parameterBindings parameters metas else parameters)
        metas source := by
  induction fuel generalizing rename source output with
  | zero => simp [resolve] at accepted
  | succ fuel ih =>
      cases source with
      | concrete value =>
          simp only [resolve, Option.some.injEq] at accepted
          subst output
          rfl
      | parameter id =>
          cases rename with
          | false =>
              simp only [resolve, Bool.false_eq_true, ↓reduceIte, Option.some.injEq] at accepted
              subst output
              rfl
          | true =>
              simp only [resolve, ↓reduceIte] at accepted
              cases found : lookup parameterBindings id with
              | none =>
                  simp only [found, Option.some.injEq] at accepted
                  subst output
                  simp [denote, parameterValuation, found]
              | some replacement =>
                  rw [found] at accepted
                  have result := ih accepted
                  simpa [denote, parameterValuation, found] using result
      | metavar id =>
          simp only [resolve] at accepted
          cases found : lookup metaBindings id with
          | none =>
              simp only [found, Option.some.injEq] at accepted
              subst output
              rfl
          | some replacement =>
              rw [found] at accepted
              have result := ih accepted
              simpa only [Bool.false_eq_true, ↓reduceIte, denote, model id replacement found] using result
      | succ inner =>
          simp only [resolve] at accepted
          cases innerResult : resolve parameterBindings metaBindings fuel rename inner with
          | none => simp [innerResult] at accepted
          | some value =>
              simp only [innerResult, Option.map_some, Option.some.injEq] at accepted
              subst output
              simpa only [denote] using congrArg (· + 1) (ih innerResult)
      | max left right =>
          simp only [resolve] at accepted
          cases leftResult : resolve parameterBindings metaBindings fuel rename left with
          | none => simp [leftResult] at accepted
          | some a =>
              cases rightResult : resolve parameterBindings metaBindings fuel rename right with
              | none => simp [leftResult, rightResult] at accepted
              | some b =>
                  simp [leftResult, rightResult] at accepted
                  subst output
                  simp only [denote, ih leftResult, ih rightResult]

theorem unquantified_parameter_is_preserved (bindings : Bindings)
    (parameters metas : Nat → Nat) (id : Nat) (absent : lookup bindings id = none) :
    parameterValuation bindings parameters metas id = parameters id := by
  simp [parameterValuation, absent]

theorem resolve_preserves_agreed_valuation {parameterBindings metaBindings : Bindings}
    {sourceParameters parameters metas : Nat → Nat}
    (agreement : ∀ id, sourceParameters id = parameterValuation parameterBindings parameters metas id)
    (model : Model metaBindings parameters metas)
    {fuel : Nat} {source output : Formula}
    (accepted : resolve parameterBindings metaBindings fuel true source = some output) :
    denote parameters metas output = denote sourceParameters metas source := by
  have same : sourceParameters = parameterValuation parameterBindings parameters metas :=
    funext agreement
  rw [same]
  exact resolve_sound model accepted

def metaIds (bindings : Bindings) : List Nat :=
  bindings.filterMap fun entry => match entry.2 with | .metavar id => some id | _ => none

def RosterValid (declared : List Nat) (bindings : Bindings) : Bool :=
  decide (declared.Nodup ∧ bindings.map Prod.fst = declared ∧
    (metaIds bindings).length = bindings.length ∧ (metaIds bindings).Nodup)

theorem roster_injective {declared : List Nat} {bindings : Bindings}
    (accepted : RosterValid declared bindings = true) : (metaIds bindings).Nodup := by
  exact (of_decide_eq_true accepted).2.2.2

def assignmentValuation (assignment : Assignment) (id : Nat) : Nat :=
  (lookup assignment id).getD 0

def groundEval (assignment : Assignment) : Formula → Option Nat
  | .concrete value => some value
  | .parameter _ => none
  | .metavar id => lookup assignment id
  | .succ inner => (groundEval assignment inner).map (· + 1)
  | .max left right => do
      let left ← groundEval assignment left
      let right ← groundEval assignment right
      some (max left right)

theorem groundEval_sound {assignment : Assignment} {source : Formula} {value : Nat}
    (accepted : groundEval assignment source = some value) :
    ∀ parameters, denote parameters (assignmentValuation assignment) source = value := by
  intro parameters
  induction source generalizing value with
  | concrete n => simpa [groundEval, denote] using accepted
  | parameter id => simp [groundEval] at accepted
  | metavar id => simp [groundEval] at accepted; simp [denote, assignmentValuation, accepted]
  | succ inner ih =>
      simp only [groundEval] at accepted
      cases found : groundEval assignment inner with
      | none => simp [found] at accepted
      | some n =>
          simp only [found, Option.map_some, Option.some.injEq] at accepted
          simp only [denote, ih found, accepted]
  | max left right hl hr =>
      simp only [groundEval] at accepted
      cases leftResult : groundEval assignment left with
      | none => simp [leftResult] at accepted
      | some a =>
          cases rightResult : groundEval assignment right with
          | none => simp [leftResult, rightResult] at accepted
          | some b =>
              simp [leftResult, rightResult] at accepted
              simp only [denote, hl leftResult, hr rightResult, accepted]

theorem lookup_member {α : Type} {entries : List (Nat × α)} {id : Nat} {value : α}
    (found : lookup entries id = some value) : (id, value) ∈ entries := by
  induction entries with
  | nil => simp [lookup] at found
  | cons head rest ih =>
      obtain ⟨key, item⟩ := head
      simp only [lookup] at found
      split at found
      · rename_i same
        simp only [Option.some.injEq] at found
        subst key
        subst item
        simp
      · exact List.mem_cons_of_mem _ (ih found)

def GroundModelValid (bindings : Bindings) (assignment : Assignment) : Bool :=
  bindings.all fun entry =>
    decide (groundEval assignment entry.2 = some (assignmentValuation assignment entry.1))

theorem groundModel_sound {bindings : Bindings} {assignment : Assignment}
    (accepted : GroundModelValid bindings assignment = true) :
    ∀ parameters, Model bindings parameters (assignmentValuation assignment) := by
  intro parameters id replacement found
  simp only [GroundModelValid, List.all_eq_true, decide_eq_true_eq] at accepted
  exact (groundEval_sound (accepted _ (lookup_member found)) parameters).symm

/- A native resolved result is checked against the exact original expression,
   both substitution maps, and the explicit model before it can certify a query. -/
theorem checked_resolution {parameterBindings metaBindings : Bindings}
    {assignment : Assignment} {source output : Formula} {fuel : Nat}
    (modelChecked : GroundModelValid metaBindings assignment = true)
    (resolved : resolve parameterBindings metaBindings fuel true source = some output) :
    ∀ parameters,
      denote parameters (assignmentValuation assignment) output =
      denote (parameterValuation parameterBindings parameters (assignmentValuation assignment))
        (assignmentValuation assignment) source := by
  intro parameters
  exact resolve_sound (groundModel_sound modelChecked parameters) resolved

theorem checked_equation {parameterBindings metaBindings : Bindings}
    {assignment : Assignment} {left right leftOutput rightOutput : Formula} {fuel : Nat}
    (modelChecked : GroundModelValid metaBindings assignment = true)
    (leftResolved : resolve parameterBindings metaBindings fuel true left = some leftOutput)
    (rightResolved : resolve parameterBindings metaBindings fuel true right = some rightOutput)
    (outputsAgree : ∀ parameters,
      denote parameters (assignmentValuation assignment) leftOutput =
      denote parameters (assignmentValuation assignment) rightOutput) :
    ∀ parameters,
      denote (parameterValuation parameterBindings parameters (assignmentValuation assignment))
        (assignmentValuation assignment) left =
      denote (parameterValuation parameterBindings parameters (assignmentValuation assignment))
        (assignmentValuation assignment) right := by
  intro parameters
  rw [← checked_resolution modelChecked leftResolved parameters,
    ← checked_resolution modelChecked rightResolved parameters]
  exact outputsAgree parameters

end Verum.UniverseInstantiation
