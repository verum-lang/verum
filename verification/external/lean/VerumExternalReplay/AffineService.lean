/-! Reusable lifecycle of an effectful, retained, single-result service.

Retention is ownership, not a promise that another grant enables progress.
Error classification is independent of whether the owner has been consumed.
These laws concern this transition system; native correspondence requires
separate extraction and replay of actual operations and effects.
-/
namespace AffineService
set_option warningAsError true

inductive Terminal (E : Type) where
  | returned
  | failed (error : E)
  deriving DecidableEq, Repr

structure State (E : Type) where
  poisoned : Bool
  terminal : Option (Terminal E)
  deriving DecidableEq, Repr

inductive Gate (E : Type) where
  | enter
  | poisoned
  | consumed (prior : Terminal E)
  deriving DecidableEq, Repr

def gate (s : State E) (budgetPoisoned : Bool) : Gate E :=
  if s.poisoned || budgetPoisoned then .poisoned
  else match s.terminal with
    | none => .enter
    | some prior => .consumed prior

inductive Outcome (E : Type) where
  | retained
  | returned
  | failed (error : E)
  deriving DecidableEq, Repr

/-- Normal return clears the active guard. Unwind never calls this function. -/
def settle : Outcome E → State E
  | .retained => ⟨false, none⟩
  | .returned => ⟨false, some .returned⟩
  | .failed e => ⟨false, some (.failed e)⟩

structure Observation (E P A : Type) where
  state : State E
  payload : P
  effects : List A

/-- The payload can include work counters, callbacks, publications and owners.
Only admitted service invokes the transition. There is no synthetic success
on a guarded retry and no promise of termination for an admitted transition. -/
def serve (s : State E) (budgetPoisoned : Bool) (p : P)
    (step : P → Outcome E × P × List A) : Observation E P A :=
  match gate s budgetPoisoned with
  | .enter => let (outcome, next, effects) := step p
              ⟨settle outcome, next, effects⟩
  | _ => ⟨s, p, []⟩

theorem poison_precedes_consumption (prior : Option (Terminal E)) (b : Bool) :
    gate ⟨true, prior⟩ b = .poisoned := by rfl

theorem poisoned_budget_refuses (s : State E) : gate s true = .poisoned := by
  simp [gate]

theorem consumption_preserves_result (prior : Terminal E) :
    gate ⟨false, some prior⟩ false = .consumed prior := by rfl

theorem enter_iff (s : State E) (b : Bool) :
    gate s b = .enter ↔ s.poisoned = false ∧ b = false ∧ s.terminal = none := by
  rcases s with ⟨poison, terminal⟩
  cases poison <;> cases b <;> cases terminal <;> simp [gate]

theorem consumed_never_enters (poison b : Bool) (prior : Terminal E) :
    gate ⟨poison, some prior⟩ b ≠ .enter := by
  cases poison <;> cases b <;> simp [gate]

theorem consumed_retry_inert (poison b : Bool) (prior : Terminal E)
    (p : P) (step : P → Outcome E × P × List A) :
    serve ⟨poison, some prior⟩ b p step = ⟨⟨poison, some prior⟩, p, []⟩ := by
  cases poison <;> cases b <;> rfl

theorem poisoned_retry_inert (prior : Option (Terminal E)) (b : Bool)
    (p : P) (step : P → Outcome E × P × List A) :
    serve ⟨true, prior⟩ b p step = ⟨⟨true, prior⟩, p, []⟩ := by rfl

theorem failed_stays_consumed (e : E) (b : Bool)
    (p : P) (step : P → Outcome E × P × List A) :
    serve (settle (.failed e)) b p step = ⟨settle (.failed e), p, []⟩ := by
  exact consumed_retry_inert false b (.failed e) p step

theorem returned_stays_consumed (b : Bool)
    (p : P) (step : P → Outcome E × P × List A) :
    serve (settle (.returned : Outcome E)) b p step = ⟨settle .returned, p, []⟩ := by
  exact consumed_retry_inert false b .returned p step

theorem retained_keeps_owner :
    gate (settle (.retained : Outcome E)) false = .enter := by rfl

end AffineService
