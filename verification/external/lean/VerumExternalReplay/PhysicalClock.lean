import Std.Tactic

/-!
Operational model of the physical dispatch clock, including its pending phases.
Origin equality denotes native allocation identity, not equality of timestamps.
The native receipt constructor is private and its value is moved, not cloned.
Here receipts are inspectable mathematical values: the duplicate-consumption law
is a state-transition property, not a claim that Lean enforces Rust affine types.

`observe` models the private edge constructor after dispatch has preflighted age.
`commit = none` represents the unreachable native missing-tick-successor panic;
it is not converted into an ordinary refusal. Capture authenticity, origin
freshness and the mapping from native allocation identities require evidence
from the implementation. No physical-world reset or periodicity is inferred.
-/
namespace PhysicalClock
set_option autoImplicit false
set_option warningAsError true

inductive Phase where
  | ready | inFlight | terminalPending
  deriving DecidableEq, Repr

inductive Refusal where
  | missingEpisode | ageOverflow | pendingOutcome | pendingTerminal
  | undeclaredTime | foreignPoint
  deriving DecidableEq, Repr

inductive TerminalKind where
  | death | time | completed | ended
  deriving DecidableEq, Repr

inductive Transition where
  | tick | terminal (kind : TerminalKind)
  deriving DecidableEq, Repr

structure Time where
  episode : Nat
  age : Nat
  deriving DecidableEq, Repr

structure Point (Origin : Type) where
  origin : Origin
  time : Time
  deriving DecidableEq, Repr

structure Clock (Origin : Type) where
  origin : Origin
  episode : Option Nat
  age : Nat
  phase : Phase
  deriving DecidableEq, Repr

structure Receipt (Origin : Type) where
  point : Point Origin
  sample : Option Nat
  transition : Transition
  successor : Option Time
  deriving DecidableEq, Repr

variable {O : Type}

def checkedNext (maximum value : Nat) : Option Nat :=
  if value.blt maximum then some (value + 1) else none

def point (maximum : Nat) (clock : Clock O) : Except Refusal (Point O) :=
  match clock.phase with
  | .inFlight => .error .pendingOutcome
  | .terminalPending => .error .pendingTerminal
  | .ready => match clock.episode with
    | none => .error .missingEpisode
    | some episode => if clock.age.blt maximum
        then .ok ⟨clock.origin, ⟨episode, clock.age⟩⟩ else .error .ageOverflow

def start [DecidableEq O] (maximum : Nat) (clock : Clock O) (selected : Point O) :
    Clock O × Except Refusal Unit :=
  match point maximum clock with
  | .error reason => (clock, .error reason)
  | .ok current => if current = selected then
      ({clock with phase := .inFlight}, .ok ()) else (clock, .error .foreignPoint)

/-- The sample and source point are retained exactly. The tick successor is
defined only after preflight, just as the native constructor's checked_add. -/
def observe (maximum : Nat) (selected : Point O) (sample : Option Nat)
    (transition : Transition) : Option (Receipt O) :=
  match transition with
  | .tick => (checkedNext maximum selected.time.age).map fun age =>
      ⟨selected, sample, .tick, some ⟨selected.time.episode, age⟩⟩
  | .terminal kind => some ⟨selected, sample, .terminal kind,
      (checkedNext maximum selected.time.episode).map fun episode => ⟨episode, 0⟩⟩

def owns (clock : Clock O) (receipt : Receipt O) : Prop :=
  clock.phase = .inFlight ∧ clock.origin = receipt.point.origin ∧
    clock.episode = some receipt.point.time.episode ∧ clock.age = receipt.point.time.age

instance [DecidableEq O] (clock : Clock O) (receipt : Receipt O) :
    Decidable (owns clock receipt) := inferInstanceAs (Decidable (_ ∧ _ ∧ _ ∧ _))

def commit [DecidableEq O] (clock : Clock O) (receipt : Receipt O) :
    Option (Clock O × Except (Refusal × Receipt O) Unit) :=
  if owns clock receipt then
    match receipt.transition with
    | .tick => receipt.successor.map fun next =>
        ({clock with age := next.age, phase := .ready}, .ok ())
    | .terminal _ => some ({clock with age := 0, phase := .terminalPending}, .ok ())
  else some (clock, .error (.foreignPoint, receipt))

def closeTerminal (maximum : Nat) (clock : Clock O) : Clock O × Except Refusal Unit :=
  if clock.phase = .terminalPending then
    ({clock with episode := clock.episode.bind (checkedNext maximum), phase := .ready}, .ok ())
  else (clock, .error .pendingTerminal)

def checkRestart (clock : Clock O) : Except Refusal Unit :=
  match clock.phase with
  | .ready => .ok ()
  | .inFlight => .error .pendingOutcome
  | .terminalPending => .error .pendingTerminal

def restart (maximum : Nat) (fresh : O) (clock : Clock O) : Clock O × Except Refusal Unit :=
  match checkRestart clock with
  | .error reason => (clock, .error reason)
  | .ok () => (⟨fresh, clock.episode.bind (checkedNext maximum), 0, .ready⟩, .ok ())

inductive LifecycleRefusal where
  | pendingBoundary | unknownDelivery | pendingPhysicalTime | pendingTerminalClosure
  deriving DecidableEq, Repr

structure JournalGate where
  pendingBoundary : Bool
  historyKnown : Bool
  deriving DecidableEq, Repr

/-- The optional journal never substitutes for the independent clock guard.
An existing pending boundary precedes unknown delivery, which precedes time. -/
def lifecycleGuard (journal : Option JournalGate) (clock : Clock O) :
    Except LifecycleRefusal Unit :=
  if journal.any JournalGate.pendingBoundary then .error .pendingBoundary
  else if journal.any (fun gate => !gate.historyKnown) then .error .unknownDelivery
  else match clock.phase with
    | .ready => .ok ()
    | .inFlight => .error .pendingPhysicalTime
    | .terminalPending => .error .pendingTerminalClosure

/-- A caller's episode-local projection is reset only after lifecycle admission.
The admission theorem below proves that the guarded restart succeeds on this
branch. The external world's restart is not performed or certified here. -/
def beginRun {Payload : Type} (maximum : Nat) (fresh : O)
    (journal : Option JournalGate) (clock : Clock O) (payload : Payload)
    (reset : Payload → Payload) : (Clock O × Payload) × Except LifecycleRefusal Unit :=
  match lifecycleGuard journal clock with
  | .error reason => ((clock, payload), .error reason)
  | .ok () => (((restart maximum fresh clock).1, reset payload), .ok ())

theorem checked_next_exact (maximum value : Nat) :
    checkedNext maximum value = some (value + 1) ↔ value < maximum := by
  simp [checkedNext, Nat.blt_eq]

theorem checked_next_at_limit (maximum : Nat) : checkedNext maximum maximum = none := by
  simp [checkedNext, Nat.blt_eq]

theorem pending_point_refused (maximum : Nat) (clock : Clock O)
    (pending : clock.phase = .inFlight) : point maximum clock = .error .pendingOutcome := by
  simp [point, pending]

theorem point_exact (maximum : Nat) (clock : Clock O) (selected : Point O) :
    point maximum clock = .ok selected ↔
      clock.phase = .ready ∧ clock.episode = some selected.time.episode ∧
      clock.origin = selected.origin ∧ clock.age = selected.time.age ∧ clock.age < maximum := by
  rcases clock with ⟨origin, episode, age, phase⟩
  rcases selected with ⟨selectedOrigin, selectedEpisode, selectedAge⟩
  cases phase <;> cases episode <;> by_cases bound : age < maximum <;>
    simp [point, Nat.blt_eq, bound, and_comm, and_assoc]

theorem start_retains_time [DecidableEq O] (maximum : Nat) (clock : Clock O)
    (selected : Point O) (current : point maximum clock = .ok selected) :
    start maximum clock selected = ({clock with phase := .inFlight}, .ok ()) := by
  simp [start, current]

theorem captured_tick_advances_once [DecidableEq O] (maximum : Nat) (clock : Clock O)
    (selected : Point O) (sample : Option Nat) (current : point maximum clock = .ok selected) :
    let receipt : Receipt O := ⟨selected, sample, .tick,
      some ⟨selected.time.episode, selected.time.age + 1⟩⟩
    observe maximum selected sample .tick = some receipt ∧
    commit (start maximum clock selected).1 receipt =
      some ({clock with age := clock.age + 1, phase := .ready}, .ok ()) := by
  obtain ⟨_, episode, origin, age, bound⟩ := (point_exact maximum clock selected).mp current
  simp [observe, checkedNext, Nat.blt_eq, start, current, commit, owns, ← age, bound, origin, episode]

theorem observed_receipt_retains_source (maximum : Nat) (selected : Point O)
    (sample : Option Nat) (transition : Transition) (receipt : Receipt O)
    (observed : observe maximum selected sample transition = some receipt) :
    receipt.point = selected ∧ receipt.sample = sample ∧ receipt.transition = transition := by
  cases transition <;> simp only [observe] at observed
  · cases successor : checkedNext maximum selected.time.age <;>
      simp_all <;> cases observed <;> simp
  · simp_all; cases observed; simp

theorem restart_requires_ready (clock : Clock O) :
    checkRestart clock = .ok () ↔ clock.phase = .ready := by
  cases phase : clock.phase <;> simp [checkRestart, phase]

theorem refused_restart_retains_clock (maximum : Nat) (fresh : O) (clock : Clock O)
    (reason : Refusal) (refused : checkRestart clock = .error reason) :
    restart maximum fresh clock = (clock, .error reason) := by
  simp [restart, refused]

theorem pending_receipt_blocks_restart (maximum : Nat) (fresh : O) (clock : Clock O)
    (pending : clock.phase = .inFlight) :
    restart maximum fresh clock = (clock, .error .pendingOutcome) := by
  simp [restart, checkRestart, pending]

theorem pending_terminal_blocks_restart (maximum : Nat) (fresh : O) (clock : Clock O)
    (pending : clock.phase = .terminalPending) :
    restart maximum fresh clock = (clock, .error .pendingTerminal) := by
  simp [restart, checkRestart, pending]

theorem foreign_receipt_retained [DecidableEq O] (clock : Clock O) (receipt : Receipt O)
    (foreign : clock.origin ≠ receipt.point.origin) :
    commit clock receipt = some (clock, .error (.foreignPoint, receipt)) := by
  simp [commit, owns, foreign]

theorem stale_receipt_retained [DecidableEq O] (clock : Clock O) (receipt : Receipt O)
    (stale : clock.episode ≠ some receipt.point.time.episode ∨ clock.age ≠ receipt.point.time.age) :
    commit clock receipt = some (clock, .error (.foreignPoint, receipt)) := by
  cases stale <;> simp_all [commit, owns]

theorem committed_phase [DecidableEq O] (clock after : Clock O) (receipt : Receipt O)
    (accepted : commit clock receipt = some (after, .ok ())) :
    after.phase = .ready ∨ after.phase = .terminalPending := by
  unfold commit at accepted
  split at accepted
  · cases transition : receipt.transition <;> simp only [transition] at accepted
    · cases successor : receipt.successor <;> simp_all <;> cases accepted <;> simp
    · simp_all; cases accepted; simp
  · simp_all

theorem immediate_duplicate_refused [DecidableEq O] (clock after : Clock O) (receipt : Receipt O)
    (accepted : commit clock receipt = some (after, .ok ())) :
    commit after receipt = some (after, .error (.foreignPoint, receipt)) := by
  have phase := committed_phase clock after receipt accepted
  cases phase <;> simp_all [commit, owns]

theorem terminal_commit_retains_episode [DecidableEq O] (clock : Clock O) (receipt : Receipt O)
    (kind : TerminalKind) (authorized : owns clock receipt)
    (terminal : receipt.transition = .terminal kind) :
    commit clock receipt = some ({clock with age := 0, phase := .terminalPending}, .ok ()) := by
  simp [commit, authorized, terminal]

theorem close_terminal_once (maximum : Nat) (clock : Clock O)
    (pending : clock.phase = .terminalPending) :
    (closeTerminal maximum clock).1.episode = clock.episode.bind (checkedNext maximum) ∧
    closeTerminal maximum (closeTerminal maximum clock).1 =
      ((closeTerminal maximum clock).1, .error .pendingTerminal) := by
  simp [closeTerminal, pending]

theorem terminal_overflow_never_wraps (maximum : Nat) (clock : Clock O)
    (pending : clock.phase = .terminalPending) (last : clock.episode = some maximum) :
    (closeTerminal maximum clock).1.episode = none := by
  simp [closeTerminal, pending, last, checkedNext, Nat.blt_eq]

theorem restart_cannot_resurrect_episode (maximum : Nat) (fresh : O) (clock : Clock O)
    (exhausted : clock.episode = none) : (restart maximum fresh clock).1.episode = none := by
  cases phase : clock.phase <;> simp [restart, checkRestart, phase, exhausted]

theorem fresh_restart_changes_origin (maximum : Nat) (fresh : O) (clock : Clock O)
    (ready : clock.phase = .ready) (different : fresh ≠ clock.origin) :
    (restart maximum fresh clock).1.origin ≠ clock.origin := by
  simpa [restart, checkRestart, ready] using different

theorem journal_cannot_hide_pending_clock (clock : Clock O)
    (pending : clock.phase = .inFlight) :
    lifecycleGuard none clock = .error .pendingPhysicalTime ∧
    lifecycleGuard (some ⟨false, true⟩) clock = .error .pendingPhysicalTime := by
  simp [lifecycleGuard, pending]

theorem boundary_refusal_precedes_clock (clock : Clock O) (known : Bool) :
    lifecycleGuard (some ⟨true, known⟩) clock = .error .pendingBoundary := by
  simp [lifecycleGuard]

theorem unknown_delivery_precedes_clock (clock : Clock O) :
    lifecycleGuard (some ⟨false, false⟩) clock = .error .unknownDelivery := by
  simp [lifecycleGuard]

theorem lifecycle_admission_requires_ready (journal : Option JournalGate) (clock : Clock O)
    (admitted : lifecycleGuard journal clock = .ok ()) : clock.phase = .ready := by
  unfold lifecycleGuard at admitted
  split at admitted <;> simp_all
  split at admitted <;> simp_all
  cases phase : clock.phase <;> simp_all

theorem admitted_restart_succeeds (maximum : Nat) (fresh : O)
    (journal : Option JournalGate) (clock : Clock O)
    (admitted : lifecycleGuard journal clock = .ok ()) :
    (restart maximum fresh clock).2 = .ok () := by
  have ready := lifecycle_admission_requires_ready journal clock admitted
  simp [restart, checkRestart, ready]

theorem refused_run_preserves_all {Payload : Type} (maximum : Nat) (fresh : O)
    (journal : Option JournalGate) (clock : Clock O) (payload : Payload)
    (reset : Payload → Payload) (reason : LifecycleRefusal)
    (refused : lifecycleGuard journal clock = .error reason) :
    beginRun maximum fresh journal clock payload reset = ((clock, payload), .error reason) := by
  simp [beginRun, refused]

theorem admitted_run_resets_projection {Payload : Type} (maximum : Nat) (fresh : O)
    (journal : Option JournalGate) (clock : Clock O) (payload : Payload)
    (reset : Payload → Payload) (admitted : lifecycleGuard journal clock = .ok ()) :
    beginRun maximum fresh journal clock payload reset =
      ((⟨fresh, clock.episode.bind (checkedNext maximum), 0, .ready⟩, reset payload), .ok ()) := by
  have ready := lifecycle_admission_requires_ready journal clock admitted
  simp [beginRun, admitted, restart, checkRestart, ready]

end PhysicalClock
