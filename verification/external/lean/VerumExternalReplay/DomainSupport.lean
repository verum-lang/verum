/-! Pure domain support and one paid step of the exported unary support memo.
The model preserves a child's immutable identity/outputs, exact domain keys,
partial scans and resource refusals. It does not model the producer's other
lanes, FIFO scheduling, allocator failure or arbitrary Rust execution. -/
namespace DomainSupport
set_option warningAsError true

/-- Support is a predicate on cases, not equality of observed outputs. -/
def support (outputs : C → V) (accepts : V → Bool) (c : C) : Bool := accepts (outputs c)

theorem support_extensional (left right : C → V) (accepts : V → Bool)
    (same : ∀ c, left c = right c) (c : C) :
    support left accepts c = support right accepts c := by
  unfold support
  rw [same]

/-- Intersect case predicates, each computed for its own child/domain. -/
theorem intersection (left : C → A) (right : C → B)
    (acceptLeft : A → Bool) (acceptRight : B → Bool) (c : C) :
    support (fun i => (left i, right i)) (fun p => acceptLeft p.1 && acceptRight p.2) c =
      (support left acceptLeft c && support right acceptRight c) := rfl

def scanFrom (domain : Nat) : List Nat → Nat → Nat → Nat
  | [], _, mask => mask
  | output :: rest, position, mask =>
      scanFrom domain rest (position + 1)
        (if output &&& domain == 0 then mask else mask ||| (1 <<< position))

def mask (outputs : List Nat) (domain : Nat) : Nat := scanFrom domain outputs 0 0

theorem scan_append (domain : Nat) (before suffix : List Nat) (position accumulated : Nat) :
    scanFrom domain (before ++ suffix) position accumulated =
      scanFrom domain suffix (position + before.length)
        (scanFrom domain before position accumulated) := by
  induction before generalizing position accumulated with
  | nil => simp [scanFrom]
  | cons output rest ih =>
      simp only [List.cons_append, scanFrom, List.length_cons]
      simpa only [Nat.add_assoc, Nat.add_comm 1] using
        ih (position + 1) (if output &&& domain == 0 then accumulated else accumulated ||| (1 <<< position))

inductive Phase where
  | domain | lookup (position : Nat) | scan | publish | ready
  deriving DecidableEq

structure State where
  language : String
  child : String
  arrival : Nat
  outputs : List Nat
  alternative : Nat
  phase : Phase
  cursor : Nat
  domain : Nat
  compatible : Nat
  cache : List (Nat × Nat)
  deriving DecidableEq

inductive Outcome where
  | advanced | workRefusal | capacityRefusal
  deriving DecidableEq

structure Result where
  outcome : Outcome
  charged : Nat
  cells : Nat
  state : Option State
  deriving DecidableEq

/-- One native support visit costs one unit, including a capacity refusal.
Work refusal happens before any visit. All quantities in the export are
validated as bounded native integers before these Nat definitions are used. -/
def step (requests : List Nat) (s : State) (cells grant capacity : Nat) : Result := Id.run do
  if grant == 0 then return ⟨.workRefusal, 0, cells, some s⟩
  if s.alternative == requests.length then
    return ⟨.advanced, 1, cells - (1 + s.cache.length), none⟩
  let capacityRefusal : Result := ⟨.capacityRefusal, 1, cells, some s⟩
  match s.phase with
  | .domain =>
      return ⟨.advanced, 1, cells, some { s with domain := requests[s.alternative]!, phase := .lookup 0 }⟩
  | .lookup offset =>
      let next := match s.cache[offset]? with
        | none => { s with phase := .scan }
        | some (key, value) =>
            if key == s.domain then { s with phase := .ready, cursor := s.outputs.length, compatible := value }
            else { s with phase := .lookup (offset + 1) }
      return ⟨.advanced, 1, cells, some next⟩
  | .scan =>
      let compatible := if s.outputs[s.cursor]! &&& s.domain == 0 then s.compatible
                        else s.compatible ||| (1 <<< s.cursor)
      let cursor := s.cursor + 1
      return ⟨.advanced, 1, cells, some { s with compatible := compatible, cursor := cursor, phase := if cursor == s.outputs.length then .publish else .scan }⟩
  | .publish =>
      if cells + 1 > capacity then return capacityRefusal
      return ⟨.advanced, 1, cells + 1, some { s with phase := .ready, cache := s.cache ++ [(s.domain, s.compatible)] }⟩
  | .ready =>
      let allocated := if s.compatible == 0 then 0 else 1
      if cells + allocated > capacity then return capacityRefusal
      return ⟨.advanced, 1, cells + allocated, some { s with alternative := s.alternative + 1, phase := .domain, cursor := 0, domain := 0, compatible := 0 }⟩

/-- This invariant concerns the captured memo, independently of its status fields. -/
def Sound (s : State) : Prop :=
  (∀ entry ∈ s.cache, entry.2 = mask s.outputs entry.1) ∧
  match s.phase with
  | .domain | .lookup _ => s.cursor = 0 ∧ s.compatible = 0
  | .scan => s.compatible = mask (s.outputs.take s.cursor) s.domain
  | .publish | .ready => s.compatible = mask s.outputs s.domain

instance (s : State) : Decidable (Sound s) := by
  unfold Sound
  cases s.phase <;> infer_instance

/-- A correctly keyed retained entry supplies exactly the original support. -/
theorem reuse (s : State) (sound : Sound s) (entry : Nat × Nat)
    (present : entry ∈ s.cache) (sameDomain : entry.1 = domain) :
    entry.2 = mask s.outputs domain := by
  rw [← sameDomain]
  exact sound.1 entry present

/-- Refusal preserves the full owned state; no empty success is manufactured. -/
theorem zero_work (requests : List Nat) (s : State) (cells capacity : Nat) :
    step requests s cells 0 capacity = ⟨.workRefusal, 0, cells, some s⟩ := rfl

end DomainSupport
