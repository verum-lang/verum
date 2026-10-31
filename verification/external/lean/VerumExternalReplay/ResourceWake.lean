import VerumExternalReplay.ResourceLedger

/-!
A parked continuation waits on an observation boundary and, optionally, one
specific allocation refusal. Eligibility is not a reservation: resumed work
may consume the capacity that made a wait ready. Clear an eligible gate before
that work; only an explicit new parking decision can install another gate.

The boundary may be a pair of roster/evidence counts or another decidable
observation. This projection neither runs the continuation nor proves native
ownership, eventual service, allocation success or Rust refinement.
-/
namespace ResourceWake
set_option autoImplicit false
set_option warningAsError true
open ResourceLedger
variable {Boundary Identity : Type} [DecidableEq Boundary]

structure Gate (Boundary Identity : Type) where
  boundary : Boundary
  memory : Option (Wait Identity)

def Gate.waiting (gate : Gate Boundary Identity) (ledger : Ledger) (current : Boundary) : Bool :=
  decide (gate.boundary = current) && !gate.memory.any (ready ledger)

def blocked (ledger : Ledger) (current : Boundary) : Option (Gate Boundary Identity) → Bool
  | none => false
  | some gate => gate.waiting ledger current

def consume (ledger : Ledger) (current : Boundary) : Option (Gate Boundary Identity) → Option (Gate Boundary Identity)
  | none => none
  | some gate => if gate.waiting ledger current then some gate else none

theorem absent_stays_absent (ledger : Ledger) (current : Boundary) :
    consume (Identity := Identity) ledger current none = none := rfl

theorem blocked_gate_retained (ledger : Ledger) (current : Boundary) (gate : Gate Boundary Identity)
    (waiting : gate.waiting ledger current = true) :
    consume ledger current (some gate) = some gate := by simp [consume, waiting]

theorem eligible_gate_consumed (ledger : Ledger) (current : Boundary) (gate : Gate Boundary Identity)
    (eligible : gate.waiting ledger current = false) :
    consume ledger current (some gate) = none := by simp [consume, eligible]

theorem consumption_idempotent (ledger : Ledger) (current : Boundary)
    (pending : Option (Gate Boundary Identity)) :
    consume ledger current (consume ledger current pending) = consume ledger current pending := by
  cases pending with
  | none => rfl
  | some gate => cases waiting : gate.waiting ledger current <;> simp [consume, waiting]

/-- Arbitrary future ledger changes cannot restore an old gate. This does not
forbid the caller from explicitly parking on a newly observed refusal. -/
theorem consumed_gate_stays_open (before after : Ledger) (current later : Boundary)
    (gate : Gate Boundary Identity) (eligible : gate.waiting before current = false) :
    blocked after later (consume before current (some gate)) = false := by
  rw [eligible_gate_consumed before current gate eligible]
  rfl

theorem evidence_only_waits (ledger : Ledger) (boundary : Boundary) :
    (Gate.mk boundary (none : Option (Wait Identity))).waiting ledger boundary = true := by
  simp [Gate.waiting]

theorem changed_boundary_wakes (ledger : Ledger) (current : Boundary) (gate : Gate Boundary Identity)
    (different : gate.boundary ≠ current) : gate.waiting ledger current = false := by
  simp [Gate.waiting, different]

theorem allocation_readiness_wakes (ledger : Ledger) (boundary : Boundary) (wait : Wait Identity)
    (eligible : ready ledger wait = true) :
    (Gate.mk boundary (some wait)).waiting ledger boundary = false := by
  simp [Gate.waiting, eligible]

theorem nonresumable_waits (ledger : Ledger) (boundary : Boundary) (wait : Wait Identity)
    (permanent : resumable wait.reason = false) :
    (Gate.mk boundary (some wait)).waiting ledger boundary = true := by
  simp [Gate.waiting, nonresumable_never_ready ledger wait permanent]

theorem pair_boundary_waiting (ledger : Ledger) (rows members currentRows currentMembers : Nat)
    (wait : Option (Wait Identity)) :
    (Gate.mk (rows, members) wait).waiting ledger (currentRows, currentMembers) =
      (decide (rows = currentRows) && decide (members = currentMembers) && !wait.any (ready ledger)) := by
  by_cases sameRows : rows = currentRows <;>
    by_cases sameMembers : members = currentMembers <;> simp_all [Gate.waiting]

end ResourceWake
