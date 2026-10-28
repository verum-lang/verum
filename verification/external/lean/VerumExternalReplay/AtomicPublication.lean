import VerumExternalReplay.ObserverOwnership

/-! One publication owns a native revision and an immutable learned frontier.
Identity parameters denote allocations, not equal payload values. The checked
update carries the private-constructor invariant required by the native commit.
No allocator, callback purity or physical provenance is assumed to be proved. -/
namespace AtomicPublication
open ObserverOwnership
set_option warningAsError true

structure Frontier (D O S B : Type) where
  cut : Cut D O S
  bank : B
  deriving DecidableEq

structure Update (D O S B T : Type) where
  reading : Cut D O S
  prepared : Prepared D O S
  previous : B
  next : B
  target : T
  coherent : prepared.before = reading

structure Receipt (D O S B T : Type) where
  reading : Cut D O S
  committed : Committed D O S
  target : T
  before : B
  after : B

structure Result (D O S B T : Type) where
  current : Frontier D O S B
  receipt : Except (Update D O S B T) (Receipt D O S B T)

variable {D O S B T : Type}

def fork (f : Frontier D O S B) (owner : O) : Frontier D O S B :=
  ⟨ObserverOwnership.fork f.cut owner, f.bank⟩

def published (f : Frontier D O S B) (u : Update D O S B T) : Frontier D O S B :=
  ⟨{f.cut with state := u.prepared.after}, u.next⟩

def admissible [DecidableEq D] [DecidableEq O] [DecidableEq S] [DecidableEq B]
    (f : Frontier D O S B) (u : Update D O S B T) : Bool :=
  decide (f.bank = u.previous ∧ f.cut = u.reading)

def commit [DecidableEq D] [DecidableEq O] [DecidableEq S] [DecidableEq B]
    (f : Frontier D O S B) (u : Update D O S B T) : Result D O S B T :=
  if admissible f u then
    let next := published f u
    ⟨next, .ok ⟨u.reading, ⟨u.prepared.before, next.cut⟩, u.target, u.previous, next.bank⟩⟩
  else ⟨f, .error u⟩

variable [DecidableEq D] [DecidableEq O] [DecidableEq S] [DecidableEq B]

theorem admission_exact (f : Frontier D O S B) (u : Update D O S B T) :
    admissible f u = true ↔ f.bank = u.previous ∧ f.cut = u.reading := by
  simp [admissible]

theorem rejected_retains_both (f : Frontier D O S B) (u : Update D O S B T)
    (no : admissible f u = false) : commit f u = ⟨f, .error u⟩ := by
  simp [commit, no]

theorem accepted_publishes_both (f : Frontier D O S B) (u : Update D O S B T)
    (yes : admissible f u = true) : (commit f u).current = published f u := by
  simp [commit, yes]

/-- This is the reason the underlying native `expect` is justified: equality
of the reading cut alone would not suffice without the prepared-owner premise. -/
theorem underlying_commit_succeeds (f : Frontier D O S B) (u : Update D O S B T)
    (yes : admissible f u = true) :
    ObserverOwnership.commit f.cut u.prepared =
      ⟨(published f u).cut, .ok ⟨u.prepared.before, (published f u).cut⟩⟩ := by
  have same : f.cut = u.prepared.before :=
    ((admission_exact f u).mp yes).2.trans u.coherent.symm
  exact ObserverOwnership.commit_exact f.cut u.prepared
    (congrArg Cut.owner same) (congrArg Cut.state same)

theorem foreign_bank_refused (f : Frontier D O S B) (u : Update D O S B T)
    (foreign : f.bank ≠ u.previous) : commit f u = ⟨f, .error u⟩ := by
  simp [commit, admissible, foreign]

theorem foreign_cut_refused (f : Frontier D O S B) (u : Update D O S B T)
    (foreign : f.cut ≠ u.reading) : commit f u = ⟨f, .error u⟩ := by
  simp [commit, admissible, foreign]

theorem sibling_refused (f : Frontier D O S B) (u : Update D O S B T) (owner : O)
    (same : u.reading = f.cut) (fresh : owner ≠ f.cut.owner) :
    commit (fork f owner) u = ⟨fork f owner, .error u⟩ := by
  apply foreign_cut_refused
  intro equal
  have owners := congrArg Cut.owner equal
  exact fresh (by simpa [fork, ObserverOwnership.fork, same] using owners)

theorem old_bank_stale (f : Frontier D O S B) (u : Update D O S B T)
    (yes : admissible f u = true) (fresh : u.next ≠ u.previous) :
    commit (commit f u).current u = ⟨(commit f u).current, .error u⟩ := by
  apply foreign_bank_refused
  simpa [accepted_publishes_both f u yes, published] using fresh

theorem old_revision_stale (f : Frontier D O S B) (u : Update D O S B T)
    (yes : admissible f u = true) (fresh : u.prepared.after ≠ u.reading.state) :
    commit (commit f u).current u = ⟨(commit f u).current, .error u⟩ := by
  apply foreign_cut_refused
  intro equal
  have states := congrArg Cut.state equal
  exact fresh (by simpa [accepted_publishes_both f u yes, published] using states)

/-- A refused transfer returns the same update, still eligible at its owner. -/
theorem rejected_transfer_preserves_authority (source target : Frontier D O S B)
    (u : Update D O S B T) (yes : admissible source u = true)
    (no : admissible target u = false) :
    (commit target u).receipt = .error u ∧ (commit source u).current = published source u := by
  exact ⟨congrArg Result.receipt (rejected_retains_both target u no),
    accepted_publishes_both source u yes⟩

/-- Pairing invariants are preserved only when the completed update establishes
them for both new components. VM-only correctness supplies no such premise. -/
theorem preserves_pairing (relation : Cut D O S → B → Prop)
    (f : Frontier D O S B) (u : Update D O S B T)
    (before : relation f.cut f.bank)
    (after : relation (published f u).cut (published f u).bank) :
    relation (commit f u).current.cut (commit f u).current.bank := by
  cases yes : admissible f u <;> simp [commit, yes, before, after]

end AtomicPublication
