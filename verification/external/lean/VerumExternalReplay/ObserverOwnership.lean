/- Ownership guards for the native observer lifecycle. Identity types denote
   live Arc allocations, not equal payload values. A cut retains its allocation;
   a fresh allocation therefore differs from every retained one. Establishing
   this representation and the private-constructor invariant in foreign code
   remains a correspondence obligation. These are model laws, not a proof of
   Rust's allocator, affine typing, generic Clone, or panic behavior. -/

namespace ObserverOwnership

set_option warningAsError true

structure Cut (Definition Owner State : Type) where
  definition : Definition
  owner : Owner
  state : State
  deriving DecidableEq

structure Prepared (Definition Owner State : Type) where
  before : Cut Definition Owner State
  after : State
  deriving DecidableEq

inductive Refusal where
  | foreignInstance
  | staleState
  deriving DecidableEq

structure Committed (Definition Owner State : Type) where
  before : Cut Definition Owner State
  after : Cut Definition Owner State
  deriving DecidableEq

structure Result (Definition Owner State : Type) where
  current : Cut Definition Owner State
  receipt : Except (Refusal × Prepared Definition Owner State)
    (Committed Definition Owner State)

variable {D O S E V : Type}

def fork (cut : Cut D O S) (freshOwner : O) : Cut D O S :=
  { cut with owner := freshOwner }

-- Evaluation returns an allocated immutable state, or its exact refusal.
-- Preparing and reading have no operation that changes the current cut.
def prepare (cut : Cut D O S) (evaluated : Except E S) : Except E (Prepared D O S) :=
  evaluated.map fun after => ⟨cut, after⟩

def read (cut : Cut D O S) (evaluated : Except E V) : Except E (Cut D O S × V) :=
  evaluated.map fun value => (cut, value)

def commit [DecidableEq O] [DecidableEq S]
    (current : Cut D O S) (prepared : Prepared D O S) : Result D O S :=
  if current.owner ≠ prepared.before.owner then
    ⟨current, .error (.foreignInstance, prepared)⟩
  else if current.state ≠ prepared.before.state then
    ⟨current, .error (.staleState, prepared)⟩
  else
    let after := { current with state := prepared.after }
    ⟨after, .ok ⟨prepared.before, after⟩⟩

theorem prepare_refusal (cut : Cut D O S) (error : E) :
    prepare cut (.error error) = .error error := rfl

theorem prepare_preserves_before (cut : Cut D O S) (after : S) :
    prepare (E := E) cut (.ok after) = .ok ⟨cut, after⟩ := rfl

theorem read_retains_cut (cut : Cut D O S) (value : V) :
    read (E := E) cut (.ok value) = .ok (cut, value) := rfl

theorem read_refusal (cut : Cut D O S) (error : E) :
    read (V := V) cut (.error error) = .error error := rfl

variable [DecidableEq O] [DecidableEq S]

theorem foreign_preserves (current : Cut D O S) (prepared : Prepared D O S)
    (foreign : current.owner ≠ prepared.before.owner) :
    commit current prepared = ⟨current, .error (.foreignInstance, prepared)⟩ := by
  simp [commit, foreign]

theorem stale_preserves (current : Cut D O S) (prepared : Prepared D O S)
    (owner : current.owner = prepared.before.owner)
    (stale : current.state ≠ prepared.before.state) :
    commit current prepared = ⟨current, .error (.staleState, prepared)⟩ := by
  simp [commit, owner, stale]

theorem commit_exact (current : Cut D O S) (prepared : Prepared D O S)
    (owner : current.owner = prepared.before.owner)
    (state : current.state = prepared.before.state) :
    commit current prepared =
      ⟨{ current with state := prepared.after },
        .ok ⟨prepared.before, { current with state := prepared.after }⟩⟩ := by
  simp [commit, owner, state]

theorem success_iff (current : Cut D O S) (prepared : Prepared D O S) :
    (∃ receipt, (commit current prepared).receipt = .ok receipt) ↔
      current.owner = prepared.before.owner ∧ current.state = prepared.before.state := by
  simp [commit]; split <;> simp_all
  split <;> simp_all

theorem fork_cannot_commit_source (current : Cut D O S) (after : S) (freshOwner : O)
    (fresh : freshOwner ≠ current.owner) :
    commit current ⟨fork current freshOwner, after⟩ =
      ⟨current, .error (.foreignInstance, ⟨fork current freshOwner, after⟩)⟩ := by
  apply foreign_preserves
  exact Ne.symm fresh

-- A successful no-op update still allocates a distinct state owner. Comparing
-- payload values instead would wrongly admit another preparation at the old cut.
theorem committed_preparation_becomes_stale (before : Cut D O S) (after : S)
    (fresh : after ≠ before.state) :
    commit (commit before ⟨before, after⟩).current ⟨before, after⟩ =
      ⟨{ before with state := after }, .error (.staleState, ⟨before, after⟩)⟩ := by
  rw [commit_exact before ⟨before, after⟩ rfl rfl]
  exact stale_preserves _ _ rfl fresh

theorem foreign_rejection_remains_committable (source target : Cut D O S) (after : S)
    (foreign : target.owner ≠ source.owner) :
    (commit target ⟨source, after⟩).receipt = .error (.foreignInstance, ⟨source, after⟩) ∧
    (commit source ⟨source, after⟩).receipt =
      .ok ⟨source, { source with state := after }⟩ := by
  exact ⟨congrArg Result.receipt (foreign_preserves _ _ foreign),
    congrArg Result.receipt (commit_exact source ⟨source, after⟩ rfl rfl)⟩

-- An old immutable cut remains meaningful after the live instance advances.
omit [DecidableEq O] [DecidableEq S] in
theorem retained_cut_reads_old_state (cut : Cut D O S) (eval : S → Except E V) :
    read cut (eval cut.state) = (eval cut.state).map (fun value => (cut, value)) := rfl

end ObserverOwnership
