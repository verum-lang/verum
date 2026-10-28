import VerumExternalReplay.PredictiveReplay

/-! An incremental, private row scorer. Each admitted step consumes one bit or
the separate row end. A denied atomic step preserves the complete cursor.
This models successful exact arithmetic, not native overflow, allocation,
callback purity or the VM preparation that may precede a scoring step. -/
namespace ResidualContinuation
open PredictiveReplay ResidualPrediction
set_option warningAsError true

structure Frame where
  bank : Bank
  context : List Nat
  mask : List Bool
  position : Nat
  bytes : Nat
  deriving DecidableEq

inductive Cursor where
  | pending (frame : Frame) (remaining : List Bool)
  | ready (bank : Bank)
  deriving DecidableEq

def begin (bank : Bank) (prediction : Prediction) (target : List Nat) : Cursor :=
  .pending ⟨bank, prediction.context, wireBits prediction.mask, 0, target.length⟩
    (wireBits target)

def nextBit (f : Frame) (bit : Bool) : Frame :=
  { f with
    bank := { f.bank with body := step f.bank.body f.context f.position (f.mask.headD false) bit }
    mask := f.mask.tail
    position := f.position + 1 }

/-- These row-level values change only at the explicit row end. Private bit
counts and the likelihood ratio may already have advanced. -/
def rowFrontier (bank : Bank) : Nat × Nat × AdaptiveLength.Counts :=
  (bank.body.samples, bank.body.bits, bank.lengths)

theorem bit_retains_row_frontier (f : Frame) (bit : Bool) :
    rowFrontier (nextBit f bit).bank = rowFrontier f.bank := rfl

def closeRow (f : Frame) : Bank :=
  ⟨{ f.bank.body with samples := f.bank.body.samples + 1,
                       bits := f.bank.body.bits + 8 * f.bytes },
    AdaptiveLength.commit f.bank.lengths f.bytes⟩

def advance : Cursor → Cursor
  | .ready bank => .ready bank
  | .pending frame [] => .ready (closeRow frame)
  | .pending frame (bit :: rest) => .pending (nextBit frame bit) rest

def finish : Cursor → Option Bank
  | .ready bank => some bank
  | .pending _ _ => none

def run : Nat → Cursor → Cursor
  | 0, cursor => cursor
  | n + 1, cursor => run n (advance cursor)

/-- The semantic result of the remaining work, without discarding the private
prefix already earned. This is used in proofs, not by the incremental step. -/
def completion (f : Frame) (remaining : List Bool) : Bank :=
  let result := body f.bank.body f.context f.position f.mask remaining
  ⟨{ result with samples := f.bank.body.samples + 1,
                 bits := f.bank.body.bits + 8 * f.bytes },
    AdaptiveLength.commit f.bank.lengths f.bytes⟩

theorem completion_step (f : Frame) (bit : Bool) (rest : List Bool) :
    completion (nextBit f bit) rest = completion f (bit :: rest) := rfl

theorem run_complete (remaining : List Bool) (f : Frame) :
    run (remaining.length + 1) (.pending f remaining) = .ready (completion f remaining) := by
  induction remaining generalizing f with
  | nil => rfl
  | cons bit rest ih =>
    change run (rest.length + 1) (.pending (nextBit f bit) rest) = _
    rw [ih, completion_step]

theorem run_incomplete (remaining : List Bool) (f : Frame) (n : Nat)
    (short : n ≤ remaining.length) :
    ∃ next rest, run n (.pending f remaining) = .pending next rest ∧
      rowFrontier next.bank = rowFrontier f.bank := by
  induction n generalizing remaining f with
  | zero => exact ⟨f, remaining, rfl, rfl⟩
  | succ n ih =>
    cases remaining with
    | nil => simp at short
    | cons bit rest =>
      obtain ⟨next, rest', same, counts⟩ :=
        ih rest (nextBit f bit) (Nat.le_of_succ_le_succ short)
      exact ⟨next, rest', same, counts.trans (bit_retains_row_frontier f bit)⟩

theorem wire_bits_length (target : List Nat) : (wireBits target).length = 8 * target.length := by
  induction target with
  | nil => rfl
  | cons byte rest ih =>
    change (byteBits byte ++ wireBits rest).length = 8 * (rest.length + 1)
    rw [List.length_append, ih]
    simp [byteBits, Nat.mul_add, Nat.add_comm]

theorem begin_complete (bank : Bank) (prediction : Prediction) (target : List Nat) :
    run (8 * target.length + 1) (begin bank prediction target) =
      .ready (learn bank prediction target) := by
  rw [← wire_bits_length]
  exact run_complete (wireBits target) _

theorem begin_incomplete (bank : Bank) (prediction : Prediction) (target : List Nat)
    (n : Nat) (short : n ≤ 8 * target.length) :
    finish (run n (begin bank prediction target)) = none := by
  rw [← wire_bits_length] at short
  obtain ⟨next, rest, same, _⟩ := run_incomplete (wireBits target) _ n short
  rw [begin, same]
  rfl

theorem empty_needs_row_end (bank : Bank) (prediction : Prediction) :
    finish (begin bank prediction []) = none ∧
    finish (advance (begin bank prediction [])) = some (learn bank prediction []) :=
  ⟨rfl, rfl⟩

theorem run_add (m n : Nat) (cursor : Cursor) :
    run (m + n) cursor = run n (run m cursor) := by
  induction m generalizing cursor with
  | zero => simp [run]
  | succ m ih => simpa [Nat.succ_add, run] using ih (advance cursor)

theorem run_ready (n : Nat) (bank : Bank) : run n (.ready bank) = .ready bank := by
  induction n with
  | zero => rfl
  | succ n ih => exact ih

theorem completion_counted_once (bank : Bank) (prediction : Prediction)
    (target : List Nat) (extra : Nat) :
    run (8 * target.length + 1 + extra) (begin bank prediction target) =
      .ready (learn bank prediction target) := by
  rw [run_add, begin_complete, run_ready]

/-- Only the admission decision for this atomic scoring step is represented.
A VM preparation call can have made progress before its own resource refusal. -/
def grant (admitted : Bool) (cursor : Cursor) : Cursor :=
  if admitted then advance cursor else cursor

def drive : List Bool → Cursor → Cursor
  | [], cursor => cursor
  | admitted :: rest, cursor => drive rest (grant admitted cursor)

def paid : List Bool → Nat
  | [] => 0
  | admitted :: rest => (if admitted then 1 else 0) + paid rest

theorem denied_retains_cursor (cursor : Cursor) : grant false cursor = cursor := rfl

theorem drive_eq_run (grants : List Bool) (cursor : Cursor) :
    drive grants cursor = run (paid grants) cursor := by
  induction grants generalizing cursor with
  | nil => rfl
  | cons admitted rest ih =>
    cases admitted with
    | false => simpa [drive, grant, paid] using ih cursor
    | true =>
      change drive rest (advance cursor) = run (1 + paid rest) cursor
      rw [ih, Nat.add_comm 1, run]

theorem paused_completion (grants : List Bool) (bank : Bank) (prediction : Prediction)
    (target : List Nat) (enough : paid grants = 8 * target.length + 1) :
    finish (drive grants (begin bank prediction target)) = some (learn bank prediction target) := by
  rw [drive_eq_run, enough, begin_complete]
  rfl

theorem paused_incomplete (grants : List Bool) (bank : Bank) (prediction : Prediction)
    (target : List Nat) (short : paid grants ≤ 8 * target.length) :
    finish (drive grants (begin bank prediction target)) = none := by
  rw [drive_eq_run]
  exact begin_incomplete bank prediction target _ short

end ResidualContinuation
