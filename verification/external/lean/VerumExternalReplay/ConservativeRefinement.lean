/- Conservative representation refinement preserves execution of a selected
   finite action word. No claim about learning, search or program selection. -/

namespace ConservativeRefinement

universe u v w x y z o

inductive Outcome (State : Type u) (Terminal : Type v) (Reason : Type w) where
  | next (state : State)
  | stop (state : State) (terminal : Terminal)
  | refuse (reason : Reason)
  deriving Repr, DecidableEq

def Outcome.map (project : S → R) : Outcome S T E → Outcome R T E
  | .next s => .next (project s)
  | .stop s terminal => .stop (project s) terminal
  | .refuse reason => .refuse reason

structure Execution (State : Type u) (Terminal : Type v) (Reason : Type w) where
  states : List State
  result : Outcome State Terminal Reason
  deriving Repr, DecidableEq

def Execution.map (project : S → R) (execution : Execution S T E) : Execution R T E :=
  ⟨execution.states.map project, execution.result.map project⟩

-- The initial state is observed, as is every reached state, including a
-- terminal state. Refusal carries a reason and introduces no invented state.
def run (step : S → A → Outcome S T E) : List A → S → Execution S T E
  | [], state => ⟨[state], .next state⟩
  | action :: rest, state =>
    match step state action with
    | .next successor =>
      let tail := run step rest successor
      ⟨state :: tail.states, tail.result⟩
    | .stop successor terminal => ⟨[state, successor], .stop successor terminal⟩
    | .refuse reason => ⟨[state], .refuse reason⟩

-- A restricted simulation premise must hold at every executed action.
-- Actions after termination/refusal are not executed and need no premise.
def Admissible (step : R → A → Outcome R T E) (allowed : R → A → Prop) : List A → R → Prop
  | [], _ => True
  | action :: rest, state => allowed state action ∧
    match step state action with
    | .next successor => Admissible step allowed rest successor
    | _ => True

section Simulation

variable {S : Type u} {R : Type v} {A : Type w} {B : Type x}
  {T : Type y} {E : Type z} {O : Type o}
  (project : R → S) (liftAction : R → A → B)
  (stepS : S → A → Outcome S T E) (stepR : R → B → Outcome R T E)
  (allowed : R → A → Prop)
  (commutes : ∀ r a, allowed r a →
    stepS (project r) a = (stepR r (liftAction r a)).map project)

include commutes

theorem finite_execution_preserved (actions : List A) (r : R)
    (admissible : Admissible (fun r a => stepR r (liftAction r a)) allowed actions r) :
    run stepS actions (project r) =
      (run (fun r a => stepR r (liftAction r a)) actions r).map project := by
  induction actions generalizing r with
  | nil => rfl
  | cons action rest ih =>
    have first := commutes r action admissible.1
    cases nextStep : stepR r (liftAction r action) with
    | next successor =>
      have tail : Admissible (fun r a => stepR r (liftAction r a)) allowed rest successor := by
        simpa only [Admissible, nextStep] using admissible.2
      have nextS : stepS (project r) action = .next (project successor) := by
        simpa only [nextStep, Outcome.map] using first
      simp only [run, nextS, nextStep, Execution.map, List.map_cons]
      rw [ih successor tail]
      rfl
    | stop successor terminal =>
      have nextS : stepS (project r) action = .stop (project successor) terminal := by
        simpa only [nextStep, Outcome.map] using first
      simp only [run, nextS, nextStep, Execution.map, List.map_cons, List.map_nil, Outcome.map]
    | refuse reason =>
      have nextS : stepS (project r) action = .refuse reason := by
        simpa only [nextStep, Outcome.map] using first
      simp only [run, nextS, nextStep, Execution.map, List.map_cons, List.map_nil, Outcome.map]

theorem finite_observations_preserved (observeS : S → O) (observeR : R → O)
    (observationsCommute : ∀ r, observeS (project r) = observeR r)
    (actions : List A) (r : R)
    (admissible : Admissible (fun r a => stepR r (liftAction r a)) allowed actions r) :
    (run stepS actions (project r)).states.map observeS =
      (run (fun r a => stepR r (liftAction r a)) actions r).states.map observeR := by
  rw [finite_execution_preserved project liftAction stepS stepR allowed commutes actions r admissible]
  change ((run (fun r a => stepR r (liftAction r a)) actions r).states.map project).map observeS = _
  have observations : ∀ states : List R,
      (states.map project).map observeS = states.map observeR := by
    intro states
    induction states with
    | nil => rfl
    | cons state rest ih =>
      simp only [List.map_cons]
      rw [observationsCommute state, ih]
  exact observations _

end Simulation

-- These are mathematical counterexamples, checked by Lean's kernel. An
-- altered argument, observation, terminal or refusal cannot satisfy the
-- corresponding exact contract. They do not merely assert a metadata label.
namespace NegativeControls

def step (state action : Nat) : Outcome Nat Nat Nat := .next (state + action)

example : step 0 1 ≠ (step 0 0).map id := by decide
example : (fun n : Nat => n) 1 ≠ (fun _ : Nat => 0) 1 := by decide
example : (Outcome.stop 3 7 : Outcome Nat Nat Nat) ≠ (Outcome.next 3).map id := by decide
example : (Outcome.stop 3 7 : Outcome Nat Nat Nat) ≠ (Outcome.stop 3 8).map id := by decide
example : (Outcome.refuse 5 : Outcome Nat Nat Nat) ≠ (Outcome.next 0).map id := by decide
example : (Outcome.refuse 5 : Outcome Nat Nat Nat) ≠ (Outcome.refuse 6).map id := by decide

-- A premise that holds only at the initial state is insufficient for a word.
def divergesLater (state action : Nat) : Outcome Nat Nat Nat :=
  if state = 0 then .next action else .next 99
example : step 0 1 = (divergesLater 0 1).map id := by decide
example : run step [1, 1] 0 ≠ (run divergesLater [1, 1] 0).map id := by decide

end NegativeControls

#print axioms finite_execution_preserved
#print axioms finite_observations_preserved

end ConservativeRefinement
