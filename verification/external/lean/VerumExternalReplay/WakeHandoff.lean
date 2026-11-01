import VerumExternalReplay.AppendHandoff

/-! Paid visits to a fixed constructor bank. The retained frame can be an exact
trace snapshot or a source-derived record. Callers must separately establish
that Rust heap operations and scheduling implement the model. -/
namespace WakeHandoff

def ready (pending : Option Nat) (indexed catalogue cursor count : Nat) : Bool :=
  pending.isSome && indexed == catalogue && cursor == count

theorem ready_completed (pending : Option Nat) (indexed catalogue cursor count : Nat)
    (h : ready pending indexed catalogue cursor count = true) :
    pending.isSome = true ∧ indexed = catalogue ∧ cursor = count := by
  have outer := Bool.and_eq_true_iff.mp h
  have inner := Bool.and_eq_true_iff.mp outer.1
  exact ⟨inner.1, Nat.eq_of_beq_eq_true inner.2, Nat.eq_of_beq_eq_true outer.2⟩

/-- `rest` retains every field outside the wake projection, without inspection. -/
structure Constructor (Frame : Type) where
  phase : String
  scan : Nat
  newest : Option Nat
  watermark : Nat
  observed : Nat
  sleeping : Bool
  rest : Frame
  deriving DecidableEq

variable {Frame : Type}

def merge (indexed : Nat) (c : Constructor Frame) : Constructor Frame :=
  { c with phase := "Merge", scan := 0, newest := none, watermark := indexed, sleeping := false }

def needsWake (indexed : Nat) (c : Constructor Frame) : Bool :=
  c.sleeping && c.observed < indexed

def wake (indexed : Nat) (c : Constructor Frame) : Constructor Frame :=
  if needsWake indexed c then
    merge indexed c
  else c

theorem wake_preserves_retained (indexed : Nat) (c : Constructor Frame) :
    (wake indexed c).observed = c.observed ∧ (wake indexed c).rest = c.rest := by
  unfold wake
  split <;> exact ⟨rfl, rfl⟩

/-- Ordered actual visit indices, with a guard checked at every paid step. -/
def visits (limit start : Nat) : List Nat → Nat → Prop
  | [], finish => finish = start
  | index :: rest, finish =>
      index = start ∧ start < limit ∧ visits limit (start + 1) rest finish

def decideVisits (limit start : Nat) (indices : List Nat) (finish : Nat) :
    Decidable (visits limit start indices finish) :=
  match indices with
  | [] => inferInstanceAs (Decidable (finish = start))
  | index :: rest =>
      letI := decideVisits limit (start + 1) rest finish
      inferInstanceAs (Decidable (index = start ∧ start < limit ∧ visits limit (start + 1) rest finish))

instance (limit start : Nat) (indices : List Nat) (finish : Nat) :
    Decidable (visits limit start indices finish) := decideVisits limit start indices finish

theorem visits_exact (limit start finish : Nat) (indices : List Nat)
    (h : visits limit start indices finish) :
    finish = start + indices.length ∧ indices = List.range' start indices.length := by
  induction indices generalizing start with
  | nil => exact ⟨h, rfl⟩
  | cons index rest ih =>
      obtain ⟨head, _, tail⟩ := h
      obtain ⟨count, exactTail⟩ := ih (start + 1) tail
      constructor
      · exact count.trans (Nat.add_right_comm start 1 rest.length)
      · subst index
        change start :: rest = start :: List.range' (start + 1) rest.length
        exact congrArg (List.cons start) exactTail

theorem completed_visits (limit : Nat) (indices : List Nat)
    (h : visits limit 0 indices limit) : indices = List.range' 0 limit := by
  obtain ⟨count, exactIndices⟩ := visits_exact limit 0 limit indices h
  have length : indices.length = limit := (count.trans (Nat.zero_add indices.length)).symm
  rw [length] at exactIndices
  exact exactIndices

end WakeHandoff
