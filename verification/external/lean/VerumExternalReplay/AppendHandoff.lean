import Std

/-! Observable append transactions. This model checks the recorded projection,
not the Rust heap, scheduler, allocator or omitted wake cursor. `ready` is an
observed flag; deriving it requires a separate wake-completion argument. -/
namespace AppendHandoff

/-- Exact snapshot bytes in bounded leaves. A balanced representation avoids
linear reduction depth without replacing equality by a digest or host result. -/
inductive Snapshot where
  | leaf (bytes : String)
  | branch (left right : Snapshot)
  deriving DecidableEq

def Snapshot.bytes : Snapshot → String
  | .leaf bytes => bytes
  | .branch left right => left.bytes ++ right.bytes

theorem snapshot_equality_preserves_bytes (left right : Snapshot)
    (h : left = right) : left.bytes = right.bytes := congrArg Snapshot.bytes h

structure State where
  catalogue : Nat
  indexed : Nat
  pending : Option Nat
  emit : Bool
  ready : Bool
  ordinary : Nat
  constructor : Nat
  deriving DecidableEq, Repr

def valid (s : State) : Prop :=
  s.indexed ≤ s.catalogue ∧ s.catalogue ≤ s.indexed + 1 ∧
  (match s.pending with
    | none => s.indexed = s.catalogue ∧ s.emit = false ∧ s.ready = false
    | some arrival => arrival + 1 = s.catalogue) ∧
  (s.ready = true → s.indexed = s.catalogue)

instance (s : State) : Decidable (valid s) := by
  unfold valid
  cases s.pending <;> infer_instance

/-- Pure handoff of the fixed pending index, after commitment and observed
readiness. No expression is owned by this projection. -/
def handoff (s : State) : Option (State × Option Nat) :=
  match s.pending with
  | none => none
  | some arrival =>
    if s.ready && s.indexed == s.catalogue then
      some ({ s with pending := none, emit := false, ready := false },
        if s.emit then some arrival else none)
    else none

theorem handoff_committed (s after : State) (result : Option Nat)
    (h : handoff s = some (after, result)) :
    s.indexed = s.catalogue ∧ after.indexed = s.indexed ∧
    after.catalogue = s.catalogue ∧ after.pending = none ∧
    after.ordinary = s.ordinary ∧ after.constructor = s.constructor := by
  cases pending : s.pending with
  | none =>
    simp only [handoff, pending] at h
    cases h
  | some index =>
    simp only [handoff, pending] at h
    split at h
    next condition =>
      have committed := Nat.eq_of_beq_eq_true (Bool.and_eq_true_iff.mp condition).2
      have state_eq := congrArg Prod.fst (Option.some.inj h)
      cases state_eq
      exact ⟨committed, rfl, rfl, rfl, rfl, rfl⟩
    next _ => contradiction

theorem emitted_pending_identity (s after : State) (arrival : Nat)
    (h : handoff s = some (after, some arrival)) :
    s.pending = some arrival ∧ s.emit = true ∧ s.ready = true := by
  cases pending : s.pending with
  | none =>
    simp only [handoff, pending] at h
    cases h
  | some index =>
    simp only [handoff, pending] at h
    split at h
    next condition =>
      have ready := (Bool.and_eq_true_iff.mp condition).1
      split at h
      next emitted =>
        have identity := Option.some.inj (congrArg Prod.snd (Option.some.inj h))
        exact ⟨congrArg some identity, emitted, ready⟩
      next _ =>
        have impossible := congrArg Prod.snd (Option.some.inj h)
        cases impossible
    next _ => contradiction

/-- Compare retained transaction identity and both ordinary scheduling cursors
at every recorded step, including charged capacity refusals. -/
def preservesTransaction (before after : State) : Prop :=
  before.catalogue = after.catalogue ∧
  before.ordinary = after.ordinary ∧ before.constructor = after.constructor ∧
  (after.pending ≠ none → after.pending = before.pending ∧ after.emit = before.emit)

instance (before after : State) : Decidable (preservesTransaction before after) := by
  unfold preservesTransaction; infer_instance

end AppendHandoff
