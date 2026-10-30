/-! Factoring pure deterministic partial updates across observers with separate
reads. The result retains the first refusal, its exact before length, state,
event and error. Equality of names, hashes or sampled outputs is not a premise.
This model does not certify native scheduling, ownership or resource use. -/
namespace SharedUpdates
set_option warningAsError true

inductive Outcome (S E F : Type) where
  | complete (state : S)
  | refused (position : Nat) (state : S) (event : E) (error : F)
  deriving DecidableEq

def runFrom (update : S → E → Except F S) : Nat → List E → S → Outcome S E F
  | _, [], state => .complete state
  | position, event :: rest, state =>
      match update state event with
      | .ok next => runFrom update (position + 1) rest next
      | .error error => .refused position state event error

def continueFrom (update : S → E → Except F S) (position : Nat) (events : List E) :
    Outcome S E F → Outcome S E F
  | .complete state => runFrom update position events state
  | .refused index state event error => .refused index state event error

theorem append (update : S → E → Except F S) (before suffix : List E)
    (position : Nat) (state : S) :
    runFrom update position (before ++ suffix) state =
      continueFrom update (position + before.length) suffix (runFrom update position before state) := by
  induction before generalizing position state with
  | nil => simp [runFrom, continueFrom]
  | cons event rest ih =>
      simp only [List.cons_append, runFrom, List.length_cons]
      cases h : update state event with
      | error error => rfl
      | ok next =>
          simpa [Nat.add_assoc, Nat.add_comm, Nat.add_left_comm, continueFrom] using (ih (position + 1) next)

theorem first_refusal (update : S → E → Except F S) (before suffix : List E)
    (initial state : S) (event : E) (error : F)
    (accepted : runFrom update 0 before initial = .complete state)
    (refused : update state event = .error error) :
    runFrom update 0 (before ++ event :: suffix) initial =
      .refused before.length state event error := by
  rw [append, accepted]
  simp [continueFrom, runFrom, refused]

theorem refusal_retained (update : S → E → Except F S) (before suffix : List E)
    (initial state : S) (event : E) (error : F) (position : Nat)
    (refused : runFrom update 0 before initial = .refused position state event error) :
    runFrom update 0 (before ++ suffix) initial = .refused position state event error := by
  rw [append, refused]
  rfl

theorem update_agreement (left right : S → E → Except F S)
    (same : ∀ state event, left state event = right state event)
    (position : Nat) (events : List E) (state : S) :
    runFrom left position events state = runFrom right position events state := by
  induction events generalizing position state with
  | nil => rfl
  | cons event rest ih =>
      simp only [runFrom, same]
      cases right state event with
      | error error => rfl
      | ok next => exact ih (position + 1) next

/-- Every observer keeps its own read function and result type. Sharing only
the deterministic update cannot change any of those separate observations. -/
def observe (read : S → R) : Outcome S E F → Option R
  | .complete state => some (read state)
  | .refused .. => none

theorem separate_reads (left right : S → E → Except F S)
    (same : ∀ state event, left state event = right state event)
    (initial other : S) (initial_eq : initial = other) (events : List E)
    (read : S → R) :
    observe read (runFrom left 0 events initial) = observe read (runFrom right 0 events other) := by
  subst other
  rw [update_agreement left right same]

theorem shared_refusal (left right : S → E → Except F S)
    (same : ∀ state event, left state event = right state event)
    (initial : S) (events : List E) (position : Nat) (state : S) (event : E) (error : F) :
    runFrom left 0 events initial = .refused position state event error ↔
      runFrom right 0 events initial = .refused position state event error := by
  rw [update_agreement left right same]

end SharedUpdates
