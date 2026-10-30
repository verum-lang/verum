import Lean
import VerumExternalReplay.SharedUpdates

/-! Reuse of checked append evidence at exact nominal endpoints. `origin` and
`events` are immutable functions of retained keys; event comparison is lawful
and stable. Native Arc alone does not establish these premises. No work refund,
allocation bound, VM refinement or physical-source authenticity follows. -/
namespace SealedAppend
set_option warningAsError true
variable {Owner Key Event : Type} {origin : Key → Owner} {events : Key → List Event}

structure Boundary (Owner Key : Type) where
  source : Owner
  previous : Option Key
  next : Key
  prefixLength : Nat
  deriving DecidableEq

def prior (events : Key → List Event) (key : Option Key) : List Event :=
  match key with | none => [] | some key => events key

def previousOwner [DecidableEq Owner] (origin : Key → Owner) (b : Boundary Owner Key) : Bool :=
  match b.previous with | none => true | some key => decide (origin key = b.source)

def verify [DecidableEq Owner] [BEq Event] (origin : Key → Owner)
    (events : Key → List Event) (b : Boundary Owner Key) : Bool :=
  decide (b.prefixLength = (prior events b.previous).length) &&
  decide (origin b.next = b.source) && previousOwner origin b &&
  (prior events b.previous).isPrefixOf (events b.next)

structure Verified [DecidableEq Owner] [BEq Event]
    (origin : Key → Owner) (events : Key → List Event) where
  boundary : Boundary Owner Key
  checked : verify origin events boundary = true

def mint [DecidableEq Owner] [BEq Event] (origin : Key → Owner)
    (events : Key → List Event) (b : Boundary Owner Key) : Option (Verified origin events) :=
  if h : verify origin events b = true then some ⟨b, h⟩ else none

inductive Decision where | scan | reuse | refuse
  deriving DecidableEq

def admit [DecidableEq Owner] [DecidableEq Key] [BEq Event]
    (witness : Option (Verified origin events)) (receiver : Boundary Owner Key) : Decision :=
  match witness with
  | none => .scan
  | some witness => if witness.boundary = receiver then .reuse else .refuse

theorem verification_exact [DecidableEq Owner] [BEq Event] [LawfulBEq Event]
    (origin : Key → Owner) (events : Key → List Event) (b : Boundary Owner Key) :
    verify origin events b = true ↔
      b.prefixLength = (prior events b.previous).length ∧ origin b.next = b.source ∧
      previousOwner origin b = true ∧ prior events b.previous <+: events b.next := by
  simp only [verify, Bool.and_eq_true, decide_eq_true_eq, List.isPrefixOf_iff_prefix, and_assoc]

theorem sealed_prefix [DecidableEq Owner] [BEq Event] [LawfulBEq Event]
    (w : Verified origin events) : prior events w.boundary.previous <+: events w.boundary.next :=
  ((verification_exact origin events w.boundary).mp w.checked).2.2.2

theorem sealed_length [DecidableEq Owner] [BEq Event] [LawfulBEq Event]
    (w : Verified origin events) : w.boundary.prefixLength = (prior events w.boundary.previous).length :=
  ((verification_exact origin events w.boundary).mp w.checked).1

theorem seal_refuses_failed_check [DecidableEq Owner] [BEq Event]
    (origin : Key → Owner) (events : Key → List Event) (b : Boundary Owner Key)
    (failed : verify origin events b = false) : mint origin events b = none := by
  simp only [mint, failed, Bool.false_eq_true, ↓reduceDIte]

theorem absent_witness_scans [DecidableEq Owner] [DecidableEq Key] [BEq Event]
    (origin : Key → Owner) (events : Key → List Event) (receiver : Boundary Owner Key) :
    admit (origin := origin) (events := events) none receiver = .scan := rfl

theorem exact_boundary_reuses [DecidableEq Owner] [DecidableEq Key] [BEq Event]
    (w : Verified origin events) : admit (some w) w.boundary = .reuse := by simp [admit]

theorem foreign_boundary_refuses [DecidableEq Owner] [DecidableEq Key] [BEq Event]
    (w : Verified origin events) (receiver : Boundary Owner Key) (foreign : w.boundary ≠ receiver) :
    admit (some w) receiver = .refuse := by simp only [admit, ite_eq_right foreign]

theorem reused_boundary_exact [DecidableEq Owner] [DecidableEq Key] [BEq Event]
    (w : Verified origin events) (receiver : Boundary Owner Key)
    (accepted : admit (some w) receiver = .reuse) : w.boundary = receiver := by
  by_cases same : w.boundary = receiver
  · exact same
  · simp only [admit, ite_eq_right same] at accepted
    cases accepted

theorem reused_prefix [DecidableEq Owner] [DecidableEq Key] [BEq Event] [LawfulBEq Event]
    (w : Verified origin events) (receiver : Boundary Owner Key)
    (accepted : admit (some w) receiver = .reuse) : prior events receiver.previous <+: events receiver.next := by
  have same := reused_boundary_exact w receiver accepted
  rw [← same]
  exact sealed_prefix w

theorem same_key_verifies [DecidableEq Owner] [BEq Event] [LawfulBEq Event]
    (origin : Key → Owner) (events : Key → List Event) (key : Key) :
    verify origin events ⟨origin key, some key, key, (events key).length⟩ = true := by
  apply (verification_exact _ _ _).mpr
  exact ⟨rfl, rfl, by simp [previousOwner], List.prefix_refl _⟩

theorem initial_verifies [DecidableEq Owner] [BEq Event] [LawfulBEq Event]
    (origin : Key → Owner) (events : Key → List Event) (key : Key) :
    verify origin events ⟨origin key, none, key, 0⟩ = true := by
  apply (verification_exact _ _ _).mpr
  exact ⟨rfl, rfl, rfl, List.nil_prefix⟩

theorem chained_source [DecidableEq Owner] [BEq Event] [LawfulBEq Event]
    (first next : Verified origin events)
    (contiguous : next.boundary.previous = some first.boundary.next) :
    next.boundary.source = first.boundary.source := by
  have firstSource := ((verification_exact _ _ _).mp first.checked).2.1
  have nextSource := ((verification_exact _ _ _).mp next.checked).2.2.1
  simp only [previousOwner, contiguous, decide_eq_true_eq] at nextSource
  exact nextSource.symm.trans firstSource

theorem chained_prefix [DecidableEq Owner] [BEq Event] [LawfulBEq Event]
    (first next : Verified origin events)
    (contiguous : next.boundary.previous = some first.boundary.next) :
    prior events first.boundary.previous <+: events next.boundary.next := by
  have tail := sealed_prefix next
  rw [contiguous] at tail
  exact (sealed_prefix first).trans tail

inductive Trace [DecidableEq Owner] [BEq Event]
    (origin : Key → Owner) (events : Key → List Event) : Key → Key → Prop where
  | refl (key : Key) : Trace origin events key key
  | step {start previous : Key} (history : Trace origin events start previous)
      (w : Verified origin events) (contiguous : w.boundary.previous = some previous) :
      Trace origin events start w.boundary.next

theorem trace_prefix [DecidableEq Owner] [BEq Event] [LawfulBEq Event]
    (history : Trace origin events first last) : events first <+: events last := by
  induction history with
  | refl => exact List.prefix_refl _
  | step _ w contiguous ih =>
    have tail := sealed_prefix w
    rw [contiguous] at tail
    exact ih.trans tail

theorem trace_source [DecidableEq Owner] [BEq Event] [LawfulBEq Event]
    (history : Trace origin events first last) : origin first = origin last := by
  induction history with
  | refl => rfl
  | step _ w contiguous ih =>
    have currentSource := ((verification_exact _ _ _).mp w.checked).2.1
    have priorSource := ((verification_exact _ _ _).mp w.checked).2.2.1
    simp only [previousOwner, contiguous, decide_eq_true_eq] at priorSource
    exact ih.trans (priorSource.trans currentSource.symm)

theorem exact_suffix [DecidableEq Owner] [BEq Event] [LawfulBEq Event]
    (w : Verified origin events) :
    prior events w.boundary.previous ++ (events w.boundary.next).drop w.boundary.prefixLength =
      events w.boundary.next := by
  rw [sealed_length w, List.prefix_iff_eq_take.mp (sealed_prefix w)]
  rw [List.length_take_of_le (List.IsPrefix.length_le (sealed_prefix w))]
  exact List.take_append_drop _ _

/-- Reuse only removes a repeated prefix comparison. The existing evaluator's
    state updates, reads and first refusal remain exactly the same. -/
theorem continued_execution [DecidableEq Owner] [BEq Event] [LawfulBEq Event]
    (w : Verified origin events) (update : State → Event → Except Error State)
    (position : Nat) (initial : State) :
    SharedUpdates.runFrom update position (events w.boundary.next) initial =
      SharedUpdates.continueFrom update (position + w.boundary.prefixLength)
        ((events w.boundary.next).drop w.boundary.prefixLength)
        (SharedUpdates.runFrom update position (prior events w.boundary.previous) initial) := by
  have equality := SharedUpdates.append update (prior events w.boundary.previous)
    ((events w.boundary.next).drop w.boundary.prefixLength) position initial
  rw [exact_suffix w, ← sealed_length w] at equality
  exact equality

end SealedAppend
