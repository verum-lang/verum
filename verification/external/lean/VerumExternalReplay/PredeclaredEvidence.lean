import VerumExternalReplay.EnumerableMass
import VerumExternalReplay.PhysicalTransducers

/-! Reservations precede sample collection. Materialization cannot allocate a
new ordinal, replace the frozen declaration, or refund a refused reservation.
The ordered-buffer laws are representation independent. Exact source/order/
capacity guards below are a reference transition, not a native refinement.
No statistical independence or probability theorem follows from this ledger. -/
namespace PredeclaredEvidence
set_option warningAsError true

abbrev Reservation (D : Type) := Nat × D
abbrev Buffer (D R : Type) := Reservation D × List R

def reserve (next : Nat) (declaration : D) : Nat × Reservation D :=
  (next + 1, (next, declaration))

/-- Completion carries an existing reservation; it never touches the ledger. -/
def complete (next : Nat) (reservation : Reservation D) (result : Option G) :
    Nat × Option (Reservation D × G) :=
  (next, result.map (fun grammar => (reservation, grammar)))

theorem reserved_ordinal (next : Nat) (declaration : D) :
    (reserve next declaration).2.1 = next := rfl

theorem reserved_declaration (next : Nat) (declaration : D) :
    (reserve next declaration).2.2 = declaration := rfl

theorem reservation_advance (next : Nat) (declaration : D) :
    (reserve next declaration).1 = next + 1 := rfl

theorem completion_keeps_ledger (next : Nat) (reservation : Reservation D) (result : Option G) :
    (complete next reservation result).1 = next := rfl

theorem refusal_cannot_refund (next : Nat) (declaration : D) :
    (complete (reserve next declaration).1 (reserve next declaration).2 (none : Option G)).1 =
      next + 1 := rfl

theorem materialization_keeps_reservation (next : Nat) (reservation : Reservation D) (grammar : G) :
    (complete next reservation (some grammar)).2 = some (reservation, grammar) := rfl

def appendBatch (buffer : Buffer D R) (rows : List R) : Buffer D R :=
  (buffer.1, buffer.2 ++ rows)

def pause (buffer : Buffer D R) : Buffer D R := buffer

/-- The result type distinguishes refusal from an accepted unchanged buffer. -/
def admit (allowed : Bool) (buffer : Buffer D R) (rows : List R) :
    Except (Buffer D R) (Buffer D R) :=
  if allowed then .ok (appendBatch buffer rows) else .error buffer

theorem pause_preserves_prefix (buffer : Buffer D R) : pause buffer = buffer := rfl

theorem append_keeps_reservation (buffer : Buffer D R) (rows : List R) :
    (appendBatch buffer rows).1 = buffer.1 := rfl

theorem append_exact_prefix (buffer : Buffer D R) (rows : List R) :
    (appendBatch buffer rows).2 = buffer.2 ++ rows := rfl

theorem append_concatenation (buffer : Buffer D R) (first second : List R) :
    appendBatch (appendBatch buffer first) second = appendBatch buffer (first ++ second) := by
  simp only [appendBatch, List.append_assoc]

theorem refused_keeps_owner_and_prefix (buffer : Buffer D R) (rows : List R) :
    admit false buffer rows = .error buffer := rfl

/-- Transfer takes the whole declared prefix, never a suffix chosen at completion. -/
def transfer (buffer : Buffer D R) (grammar : G) : Reservation D × G × List R :=
  (buffer.1, grammar, buffer.2)

theorem transfer_exact_prefix (buffer : Buffer D R) (grammar : G) :
    (transfer buffer grammar).2.2 = buffer.2 := rfl

/-- Selection supplies a new end marker. Training rows are not validation rows. -/
def beginValidation (selected : C) (selectionEnd : Nat) : C × Nat × List R :=
  (selected, selectionEnd, [])

theorem validation_starts_empty (selected : C) (selectionEnd : Nat) :
    (beginValidation (R := R) selected selectionEnd).2.2 = [] := rfl

theorem reservation_indices_injective (start i j : Nat) (same : start + i = start + j) : i = j :=
  Nat.add_left_cancel same

theorem reservation_indices_strict (start i j : Nat) (before : i < j) : start + i < start + j :=
  Nat.add_lt_add_left before start

-- Arithmetic and exact admission laws extend the portable structural core.
-- These are Lean kernel proofs, not claims that the portable native term
-- library already implements exact rational arithmetic.
namespace Exact
open ExactMass EnumerableMass

theorem skipped_reservations_bounded (keep : Nat → Bool) (count : Nat) :
    AtMost (partialSum (fun j => if keep j then lookWeight j else zero) count) one := by
  apply le_trans _ (prefix_bounded paidLooks count)
  apply prefix_mono
  intro j
  split
  · exact le_refl _
  · exact zero_le _

theorem risk_prefix_balance (count : Nat) :
    Equal (add (partialSum lookWeight count) (lookReserve count)) one :=
  paid_looks_partial_sum count

structure Declaration (S Signature Family Criterion : Type) where
  source : S
  origin : Nat
  calibrationEnd : Nat
  signature : Signature
  family : Family
  criterion : Criterion
  deriving DecidableEq

abbrev OwnedBuffer (S Signature Family Criterion R : Type) :=
  Buffer (Declaration S Signature Family Criterion) (Nat × R)

def lastOrdinal (buffer : OwnedBuffer S Signature Family Criterion R) : Nat :=
  (buffer.2.getLast?).map Prod.fst |>.getD buffer.1.2.calibrationEnd

/-- Source and calibration end come from the reserved declaration. The next
bound comes from the retained prefix, never from completion time or a caller's
replacement owner. Capacity refusal returns the retained state as an error. -/
def receive [DecidableEq S] (capacity : Nat)
    (buffer : OwnedBuffer S Signature Family Criterion R) (rowSource : S) (ordinal : Nat) (row : R) :
    Except (OwnedBuffer S Signature Family Criterion R) (OwnedBuffer S Signature Family Criterion R) :=
  admit (decide (rowSource = buffer.1.2.source ∧ lastOrdinal buffer < ordinal ∧ buffer.2.length < capacity))
    buffer [(ordinal, row)]

theorem source_mismatch_refused [DecidableEq S] (rowSource : S)
    (capacity ordinal : Nat) (buffer : OwnedBuffer S Signature Family Criterion R) (row : R)
    (different : rowSource ≠ buffer.1.2.source) :
    receive capacity buffer rowSource ordinal row = .error buffer := by
  simp [receive, different, admit]

theorem old_rows_refused [DecidableEq S] (capacity ordinal : Nat)
    (buffer : OwnedBuffer S Signature Family Criterion R) (row : R) (old : ordinal ≤ lastOrdinal buffer) :
    receive capacity buffer buffer.1.2.source ordinal row = .error buffer := by
  simp [receive, Nat.not_lt.mpr old, admit]

theorem capacity_refused [DecidableEq S] (capacity ordinal : Nat)
    (buffer : OwnedBuffer S Signature Family Criterion R) (row : R) (full : capacity ≤ buffer.2.length) :
    receive capacity buffer buffer.1.2.source ordinal row = .error buffer := by
  simp [receive, Nat.not_lt.mpr full, admit]

theorem accepted_row_exact [DecidableEq S] (capacity ordinal : Nat)
    (buffer : OwnedBuffer S Signature Family Criterion R) (row : R)
    (fresh : lastOrdinal buffer < ordinal) (space : buffer.2.length < capacity) :
    receive capacity buffer buffer.1.2.source ordinal row =
      .ok (buffer.1, buffer.2 ++ [(ordinal, row)]) := by
  simp [receive, fresh, space, admit, appendBatch]

theorem appended_ordinal_exact (buffer : OwnedBuffer S Signature Family Criterion R) (ordinal : Nat) (row : R) :
    lastOrdinal (appendBatch buffer [(ordinal, row)]) = ordinal := by
  simp [lastOrdinal, appendBatch]

/-- Selection records the next physical ordinal, whereas calibration records
the last consumed ordinal. Validation therefore has an inclusive lower bound;
the ordinary source, strict ordering and capacity guards still apply. -/
def receiveValidation [DecidableEq S] (capacity selectionFrom : Nat)
    (buffer : OwnedBuffer S Signature Family Criterion R) (rowSource : S) (ordinal : Nat) (row : R) :
    Except (OwnedBuffer S Signature Family Criterion R) (OwnedBuffer S Signature Family Criterion R) :=
  if selectionFrom ≤ ordinal then receive capacity buffer rowSource ordinal row else .error buffer

theorem preselection_rows_refused [DecidableEq S] (capacity selectionFrom ordinal : Nat)
    (buffer : OwnedBuffer S Signature Family Criterion R) (rowSource : S) (row : R)
    (old : ordinal < selectionFrom) :
    receiveValidation capacity selectionFrom buffer rowSource ordinal row = .error buffer := by
  simp [receiveValidation, Nat.not_le.mpr old]

theorem validation_uses_original_guards [DecidableEq S] (capacity selectionFrom ordinal : Nat)
    (buffer : OwnedBuffer S Signature Family Criterion R) (rowSource : S) (row : R)
    (fresh : selectionFrom ≤ ordinal) :
    receiveValidation capacity selectionFrom buffer rowSource ordinal row =
      receive capacity buffer rowSource ordinal row := by
  simp [receiveValidation, fresh]

theorem accepted_validation_exact [DecidableEq S] (capacity selectionFrom ordinal : Nat)
    (buffer : OwnedBuffer S Signature Family Criterion R) (row : R)
    (selected : selectionFrom ≤ ordinal) (ordered : lastOrdinal buffer < ordinal)
    (space : buffer.2.length < capacity) :
    receiveValidation capacity selectionFrom buffer buffer.1.2.source ordinal row =
      .ok (appendBatch buffer [(ordinal, row)]) := by
  rw [validation_uses_original_guards capacity selectionFrom ordinal buffer _ row selected]
  exact accepted_row_exact capacity ordinal buffer row ordered space

end Exact
end PredeclaredEvidence
