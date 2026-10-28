import VerumExternalReplay.AtomicPublication
import VerumExternalReplay.PartialObservers

/-! One causal update contract for live execution and retained-history replay.
An anchor denotes a retained dispatch identity, not merely equal action bytes.
The native adapter must establish that snapshot values and accepted capture
belong to that dispatch; this model does not manufacture physical evidence or
prove affine typing. Newly allocated revisions retain nominal identities.
-/
namespace OutcomeTransitions
set_option warningAsError true

structure Protocol (State Bank Input Forecast Outcome : Type) where
  predict : State → Bank → Input → Option Forecast
  update : State → Input → Outcome → Option State
  learn : Bank → Forecast → Outcome → Option Bank

structure Frozen (Anchor State Bank Input Forecast : Type) where
  anchor : Anchor
  state : State
  bank : Bank
  input : Input
  forecast : Forecast

structure Capture (Anchor Outcome : Type) where
  anchor : Anchor
  outcome : Outcome

structure Ready (Anchor State Bank Input Forecast Outcome : Type) where
  before : Frozen Anchor State Bank Input Forecast
  outcome : Outcome
  state : State
  bank : Bank

variable {A S B E F Y : Type}

def freeze (protocol : Protocol S B E F Y) (anchor : A) (state : S) (bank : B) (input : E) :
    Option (Frozen A S B E F) :=
  (protocol.predict state bank input).map (fun forecast => ⟨anchor,state,bank,input,forecast⟩)

def complete [DecidableEq A] (protocol : Protocol S B E F Y) (ticket : Frozen A S B E F)
    (capture : Capture A Y) : Option (Ready A S B E F Y) := do
  if capture.anchor ≠ ticket.anchor then none else
  let state ← protocol.update ticket.state ticket.input capture.outcome
  let bank ← protocol.learn ticket.bank ticket.forecast capture.outcome
  pure ⟨ticket,capture.outcome,state,bank⟩

theorem foreign_capture_refused [DecidableEq A] (protocol : Protocol S B E F Y)
    (ticket : Frozen A S B E F) (capture : Capture A Y) (foreign : capture.anchor ≠ ticket.anchor) :
    complete protocol ticket capture = none := by simp [complete,foreign]

theorem completion_exact [DecidableEq A] (protocol : Protocol S B E F Y)
    (ticket : Frozen A S B E F) (capture : Capture A Y) (ready : Ready A S B E F Y) :
    complete protocol ticket capture = some ready ↔
      capture.anchor = ticket.anchor ∧ ∃ state bank,
        protocol.update ticket.state ticket.input capture.outcome = some state ∧
        protocol.learn ticket.bank ticket.forecast capture.outcome = some bank ∧
        ready = ⟨ticket,capture.outcome,state,bank⟩ := by
  by_cases same : capture.anchor = ticket.anchor
  · cases hu : protocol.update ticket.state ticket.input capture.outcome <;>
      cases hl : protocol.learn ticket.bank ticket.forecast capture.outcome <;>
      simp [complete,same,hu,hl,eq_comm]
  · simp [complete,same]

/-- Successful learning cannot replace the forecast or the observed outcome. -/
theorem completion_retains_evidence [DecidableEq A] (protocol : Protocol S B E F Y)
    (ticket : Frozen A S B E F) (capture : Capture A Y) (ready : Ready A S B E F Y)
    (h : complete protocol ticket capture = some ready) :
    ready.before = ticket ∧ ready.outcome = capture.outcome := by
  obtain ⟨_,state,bank,_,_,rfl⟩ := (completion_exact protocol ticket capture ready).mp h
  exact ⟨rfl,rfl⟩

structure Row (Input Outcome : Type) where
  input : Input
  outcome : Outcome

def evolve (update : S → E → Y → Option S) (rows : List (Row E Y)) (state : S) : Option S :=
  PartialObservers.fold (fun state row => update state row.input row.outcome) rows state

def ignoreOutcome (old : S → E → Option S) : S → E → Y → Option S := fun state input _ => old state input

theorem old_embedding (old : S → E → Option S) (rows : List (Row E Y)) (state : S) :
    evolve (ignoreOutcome old) rows state = PartialObservers.fold old (rows.map Row.input) state := by
  induction rows generalizing state with
  | nil => rfl
  | cons row rest ih =>
      simp only [evolve,PartialObservers.fold,ignoreOutcome,List.map_cons]
      cases h : old state row.input with
      | none => rfl
      | some next => exact ih next

theorem old_outcome_invariance (old : S → E → Option S) (first second : List (Row E Y)) (state : S)
    (same_inputs : first.map Row.input = second.map Row.input) :
    evolve (ignoreOutcome old) first state = evolve (ignoreOutcome old) second state := by
  rw [old_embedding,old_embedding,same_inputs]

theorem evolve_append (update : S → E → Y → Option S) (first second : List (Row E Y)) (state : S) :
    evolve update (first++second) state = (evolve update first state).bind (evolve update second) :=
  PartialObservers.fold_append _ _ _ _

/-- The same complete row supplies both prediction and learning in live and
replay paths. An adapter must preserve this row, including its outcome. -/
def step (protocol : Protocol S B E F Y) (state : S × B) (row : Row E Y) : Option (S × B) := do
  let ticket ← freeze protocol () state.1 state.2 row.input
  let ready ← complete protocol ticket ⟨(),row.outcome⟩
  pure (ready.state,ready.bank)

def replay (protocol : Protocol S B E F Y) (rows : List (Row E Y)) (state : S × B) : Option (S × B) :=
  PartialObservers.fold (step protocol) rows state

theorem replay_append (protocol : Protocol S B E F Y) (first second : List (Row E Y)) (state : S × B) :
    replay protocol (first++second) state = (replay protocol first state).bind (replay protocol second) :=
  PartialObservers.fold_append _ _ _ _

/-- Reuse the existing nominal publication guard. Allocation decoding is a
separate, explicit correspondence premise below. -/
def publication (source : A → AtomicPublication.Frontier D O Revision BankId)
    (ready : Ready A S B E F Y) (revision : Revision) (bankId : BankId) :
    AtomicPublication.Update D O Revision BankId Y :=
  let before := source ready.before.anchor
  ⟨before.cut,⟨before.cut,revision⟩,before.bank,bankId,ready.outcome,rfl⟩

theorem published_payloads [DecidableEq D] [DecidableEq O] [DecidableEq Revision] [DecidableEq BankId]
    (source : A → AtomicPublication.Frontier D O Revision BankId)
    (ready : Ready A S B E F Y) (revision : Revision) (bankId : BankId)
    (decodeState : Revision → S) (decodeBank : BankId → B)
    (state_matches : decodeState revision = ready.state) (bank_matches : decodeBank bankId = ready.bank) :
    let result := AtomicPublication.commit (source ready.before.anchor) (publication source ready revision bankId)
    decodeState result.current.cut.state = ready.state ∧ decodeBank result.current.bank = ready.bank := by
  simp [AtomicPublication.commit,AtomicPublication.admissible,AtomicPublication.published,
    publication,state_matches,bank_matches]

theorem publication_becomes_stale [DecidableEq D] [DecidableEq O] [DecidableEq Revision] [DecidableEq BankId]
    (source : A → AtomicPublication.Frontier D O Revision BankId)
    (ready : Ready A S B E F Y) (revision : Revision) (bankId : BankId)
    (fresh : bankId ≠ (source ready.before.anchor).bank) :
    let before := source ready.before.anchor
    let update := publication source ready revision bankId
    AtomicPublication.commit (AtomicPublication.commit before update).current update =
      ⟨(AtomicPublication.commit before update).current,.error update⟩ := by
  apply AtomicPublication.old_bank_stale
  · simp [AtomicPublication.admissible,publication]
  · exact fresh

end OutcomeTransitions
