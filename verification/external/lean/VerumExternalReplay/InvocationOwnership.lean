/-! Identity phase observations for a uniquely owned invocation. A phase trace
checks consistency of supplied checkpoints, not unobserved native execution,
atomic memory ordering, allocation freshness, or ownership of hidden payloads.
-/
namespace InvocationOwnership
set_option warningAsError true

inductive Phase where
  | prepared | armed | invoked | completed | cancelled | unknown
  deriving DecidableEq

inductive Action where
  | arm | invoke | complete | cancel | abandon
  deriving DecidableEq

def step : Phase → Action → Option Phase
  | .prepared, .arm => some .armed
  | .armed, .invoke => some .invoked
  | .invoked, .complete => some .completed
  | .armed, .cancel => some .cancelled
  | .prepared, .abandon => some .cancelled
  | .armed, .abandon | .invoked, .abandon => some .unknown
  | .completed, .abandon => some .completed
  | .cancelled, .abandon => some .cancelled
  | .unknown, .abandon => some .unknown
  | _, _ => none

def run (phase : Phase) (actions : List Action) : Option Phase :=
  actions.foldlM step phase

/-- A witness for a possible interval between observations. Repeated samples
need no transition. Missing intermediate phases are not reported as observed. -/
def interval : Phase → Phase → Option (List Action)
  | .prepared, .prepared => some []
  | .prepared, .armed => some [.arm]
  | .prepared, .invoked => some [.arm, .invoke]
  | .prepared, .completed => some [.arm, .invoke, .complete]
  | .prepared, .cancelled => some [.abandon]
  | .prepared, .unknown => some [.arm, .abandon]
  | .armed, .armed => some []
  | .armed, .invoked => some [.invoke]
  | .armed, .completed => some [.invoke, .complete]
  | .armed, .cancelled => some [.cancel]
  | .armed, .unknown => some [.abandon]
  | .invoked, .invoked => some []
  | .invoked, .completed => some [.complete]
  | .invoked, .unknown => some [.abandon]
  | .completed, .completed | .cancelled, .cancelled | .unknown, .unknown => some []
  | _, _ => none

def follows (before after : Phase) : Bool := (interval before after).isSome

theorem interval_sound (before after : Phase) (actions : List Action)
    (found : interval before after = some actions) : run before actions = some after := by
  cases before <;> cases after <;>
    simp only [interval, Option.some.injEq, reduceCtorEq] at found <;> subst actions <;> rfl

theorem completed_irreversible (after : Phase) :
    follows .completed after = true ↔ after = .completed := by
  cases after <;> decide

theorem cancelled_irreversible (after : Phase) :
    follows .cancelled after = true ↔ after = .cancelled := by
  cases after <;> decide

theorem unknown_irreversible (after : Phase) :
    follows .unknown after = true ↔ after = .unknown := by
  cases after <;> decide

/-- Actual capture-slot equality and presence are separate observations. -/
def authentic (phase : Phase) (sameSlot present : Bool) : Bool :=
  phase == .completed && sameSlot && present

theorem authentic_iff (phase : Phase) (sameSlot present : Bool) :
    authentic phase sameSlot present = true ↔
      phase = .completed ∧ sameSlot = true ∧ present = true := by
  cases phase <;> cases sameSlot <;> cases present <;> decide

end InvocationOwnership
