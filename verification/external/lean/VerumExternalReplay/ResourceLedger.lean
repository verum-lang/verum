import Std.Tactic

/-!
A finite allocation-owner model for shared shallow-byte accounting. Slots are
allocation identities, never aliases: adding an alias updates one slot. Dead
slots retain their identity and contribute zero charge. Native Rust maps an
allocation slot to its non-Clone lease and Arc references to that allocation.

The model does not describe allocator metadata, nested payloads, process RSS,
host allocation success, arbitrary callback termination, or Rust refinement.
Nat arithmetic is exact; `maximum` makes the native representable-byte premise
explicit. Receipt/control identity is retained separately from byte admission.
-/
namespace ResourceLedger
set_option autoImplicit false
set_option warningAsError true
variable {Identity Payload : Type}

structure Slot where
  kind : Nat
  bytes : Nat
  refs : Nat
  deriving DecidableEq, Repr

def charge (slot : Slot) : Nat := if slot.refs = 0 then 0 else slot.bytes

def total : List Slot → Nat
  | [] => 0
  | slot :: rest => charge slot + total rest

def classTotal (kind : Nat) : List Slot → Nat
  | [] => 0
  | slot :: rest => (if slot.kind = kind then charge slot else 0) + classTotal kind rest

structure Ledger where
  account : Nat
  maximum : Nat
  quota : Option Nat
  peak : Nat
  poisoned : Bool
  slots : List Slot
  deriving DecidableEq, Repr

def fitsQuota (quota : Option Nat) (bytes : Nat) : Bool :=
  match quota with
  | none => true
  | some limit => decide (bytes ≤ limit)

def valid (ledger : Ledger) : Prop :=
  total ledger.slots ≤ ledger.peak ∧ total ledger.slots ≤ ledger.maximum ∧
    fitsQuota ledger.quota (total ledger.slots) = true ∧ ledger.peak ≤ ledger.maximum

inductive Refusal where
  | poisoned | overflow | capacity | foreign | dead | allocation | notReady | shared
  deriving DecidableEq, Repr

def admissible (ledger : Ledger) (bytes : Nat) : Bool :=
  !ledger.poisoned && decide (total ledger.slots + bytes ≤ ledger.maximum) &&
    fitsQuota ledger.quota (total ledger.slots + bytes)

/-- A successful admission owns a new slot. Existing slots and their references
remain intact; coexistence with old storage is charged before replacement. -/
def admitted (ledger : Ledger) (kind bytes : Nat) : Ledger :=
  { ledger with
    slots := ledger.slots ++ [⟨kind, bytes, 1⟩]
    peak := max ledger.peak (total ledger.slots + bytes) }

def reserve (ledger : Ledger) (kind bytes : Nat) : Except Refusal Ledger :=
  if ledger.poisoned then .error .poisoned
  else if total ledger.slots + bytes > ledger.maximum then .error .overflow
  else if fitsQuota ledger.quota (total ledger.slots + bytes) then .ok (admitted ledger kind bytes)
  else .error .capacity

/-- A refused operation retains the exact original ownership/byte projection.
Native diagnostic refusal counters are intentionally outside this projection:
capacity refusal increments the native saturating counter. -/
def commit (before : Ledger) (result : Except Refusal Ledger) : Ledger :=
  match result with
  | .ok after => after
  | .error _ => before

/-- Focus is an exact existing allocation slot, at stable index `leftSlots.length`.
No other slot is changed by alias/release. Native token authority is a separate
representation premise, not something an untrusted numeric index proves. -/
def focused (ledger : Ledger) (leftSlots : List Slot) (slot : Slot) (suffix : List Slot) : Prop :=
  ledger.slots = leftSlots ++ slot :: suffix

instance (ledger : Ledger) (leftSlots : List Slot) (slot : Slot) (suffix : List Slot) :
    Decidable (focused ledger leftSlots slot suffix) := inferInstanceAs (Decidable (_ = _))

def replaced (ledger : Ledger) (leftSlots : List Slot) (slot : Slot) (suffix : List Slot)
    (refs : Nat) : Ledger :=
  { ledger with slots := leftSlots ++ { slot with refs := refs } :: suffix }

def alias (ledger : Ledger) (leftSlots : List Slot) (slot : Slot) (suffix : List Slot) :
    Except Refusal Ledger :=
  if ¬ focused ledger leftSlots slot suffix then .error .foreign
  else if slot.refs = 0 then .error .dead
  else .ok (replaced ledger leftSlots slot suffix (slot.refs + 1))

/-- Drop remains legal on a poisoned account. It cannot admit new storage. -/
def release (ledger : Ledger) (leftSlots : List Slot) (slot : Slot) (suffix : List Slot) :
    Except Refusal Ledger :=
  if ¬ focused ledger leftSlots slot suffix then .error .foreign
  else if slot.refs = 0 then .error .dead
  else .ok (replaced ledger leftSlots slot suffix (slot.refs - 1))

def grant (ledger : Ledger) (quota : Option Nat) : Except Refusal Ledger :=
  if ledger.poisoned then .error .poisoned
  else if fitsQuota quota (total ledger.slots) then .ok { ledger with quota := quota }
  else .error .capacity

structure Wait (Identity : Type) where
  account : Nat
  reason : Refusal
  requested : Nat
  computation : Identity

def resumable : Refusal → Bool
  | .capacity | .allocation => true
  | _ => false

def ready (ledger : Ledger) (wait : Wait Identity) : Bool :=
  decide (wait.account = ledger.account) && resumable wait.reason &&
    admissible ledger wait.requested

/-- Eligibility hands back the same computation identity, never a fresh name,
look, history boundary, program or paid-work balance. It is not a reservation. -/
def resume (ledger : Ledger) (wait : Wait Identity) : Option Identity :=
  if ready ledger wait then some wait.computation else none

theorem total_append (left right : List Slot) : total (left ++ right) = total left + total right := by
  induction left with
  | nil => simp [total]
  | cons head tail ih => simp [total, ih, Nat.add_assoc]

theorem class_append (kind : Nat) (left right : List Slot) :
    classTotal kind (left ++ right) = classTotal kind left + classTotal kind right := by
  induction left with
  | nil => simp [classTotal]
  | cons head tail ih => simp [classTotal, ih, Nat.add_assoc]

theorem admitted_total (ledger : Ledger) (kind bytes : Nat) :
    total (admitted ledger kind bytes).slots = total ledger.slots + bytes := by
  simp [admitted, total_append, total, charge]

theorem admitted_class (ledger : Ledger) (kind bytes : Nat) :
    classTotal kind (admitted ledger kind bytes).slots = classTotal kind ledger.slots + bytes := by
  simp [admitted, class_append, classTotal, charge]

theorem other_class_preserved (ledger : Ledger) (kind other bytes : Nat) (different : kind ≠ other) :
    classTotal other (admitted ledger kind bytes).slots = classTotal other ledger.slots := by
  simp [admitted, class_append, classTotal, different]

theorem reserve_acceptance (ledger : Ledger) (kind bytes : Nat) :
    reserve ledger kind bytes = .ok (admitted ledger kind bytes) ↔ admissible ledger bytes = true := by
  unfold reserve admissible
  by_cases poisoned : ledger.poisoned = true <;>
    by_cases maximum : total ledger.slots + bytes ≤ ledger.maximum <;>
    by_cases quota : fitsQuota ledger.quota (total ledger.slots + bytes) = true <;>
    simp_all [Nat.not_lt_of_ge]

/-- A word-bound premise stays symbolic; checking it need not normalize a
unary proof of the difference up to the machine maximum. -/
theorem reserve_at_capacity (ledger : Ledger) (kind bytes : Nat)
    (healthy : ledger.poisoned = false)
    (addressable : total ledger.slots + bytes ≤ ledger.maximum)
    (denied : fitsQuota ledger.quota (total ledger.slots + bytes) = false) :
    reserve ledger kind bytes = .error .capacity := by
  simp [reserve, healthy, Nat.not_lt_of_ge addressable, denied]

theorem admitted_preserves_valid (ledger : Ledger) (kind bytes : Nat)
    (before : valid ledger) (accepted : admissible ledger bytes = true) : valid (admitted ledger kind bytes) := by
  simp only [admissible, Bool.and_eq_true, decide_eq_true_eq] at accepted
  unfold valid
  rw [admitted_total]
  exact ⟨Nat.le_max_right _ _, accepted.1.2, accepted.2,
    Nat.max_le.mpr ⟨before.2.2.2, accepted.1.2⟩⟩

theorem original_slots_preserved (ledger : Ledger) (kind bytes : Nat) :
    (admitted ledger kind bytes).slots.take ledger.slots.length = ledger.slots := by
  simp [admitted]

theorem refused_ownership_projection_unchanged (before : Ledger) (reason : Refusal) :
    commit before (.error reason) = before := rfl

theorem alias_charge (slot : Slot) (live : slot.refs ≠ 0) :
    charge { slot with refs := slot.refs + 1 } = charge slot := by simp [charge, live]

theorem alias_total (ledger : Ledger) (leftSlots : List Slot) (slot : Slot) (suffix : List Slot)
    (owned : focused ledger leftSlots slot suffix) (live : slot.refs ≠ 0) :
    total (replaced ledger leftSlots slot suffix (slot.refs + 1)).slots = total ledger.slots := by
  rw [owned]
  simp [replaced, total_append, total, alias_charge slot live]

theorem nonfinal_release_total (ledger : Ledger) (leftSlots : List Slot) (slot : Slot) (suffix : List Slot)
    (owned : focused ledger leftSlots slot suffix) (shared : 1 < slot.refs) :
    total (replaced ledger leftSlots slot suffix (slot.refs - 1)).slots = total ledger.slots := by
  have live : slot.refs ≠ 0 := by omega
  have stays : slot.refs - 1 ≠ 0 := by omega
  rw [owned]
  simp [replaced, total_append, total, charge, live, stays]

theorem final_release_total (ledger : Ledger) (leftSlots : List Slot) (slot : Slot) (suffix : List Slot)
    (owned : focused ledger leftSlots slot suffix) (last : slot.refs = 1) :
    total (replaced ledger leftSlots slot suffix (slot.refs - 1)).slots + slot.bytes = total ledger.slots := by
  rw [owned]
  simp [replaced, total_append, total, charge, last, Nat.add_left_comm, Nat.add_comm]

theorem final_release_class (ledger : Ledger) (leftSlots : List Slot) (slot : Slot) (suffix : List Slot)
    (owned : focused ledger leftSlots slot suffix) (last : slot.refs = 1) :
    classTotal slot.kind (replaced ledger leftSlots slot suffix (slot.refs - 1)).slots + slot.bytes =
      classTotal slot.kind ledger.slots := by
  rw [owned]
  simp [replaced, class_append, classTotal, charge, last, Nat.add_left_comm, Nat.add_comm]

theorem release_cannot_increase (ledger : Ledger) (leftSlots : List Slot) (slot : Slot) (suffix : List Slot)
    (owned : focused ledger leftSlots slot suffix) :
    total (replaced ledger leftSlots slot suffix (slot.refs - 1)).slots ≤ total ledger.slots := by
  rw [owned]
  simp only [replaced, total_append, total]
  by_cases live : slot.refs = 0 <;> by_cases last : slot.refs - 1 = 0 <;> simp_all [charge]

theorem release_keeps_peak (ledger : Ledger) (leftSlots : List Slot) (slot : Slot) (suffix : List Slot) :
    (replaced ledger leftSlots slot suffix (slot.refs - 1)).peak = ledger.peak := rfl

theorem poisoned_cannot_reserve (ledger : Ledger) (kind bytes : Nat) (poisoned : ledger.poisoned = true) :
    reserve ledger kind bytes = .error .poisoned := by simp [reserve, poisoned]

theorem poisoned_can_release (ledger : Ledger) (leftSlots : List Slot) (slot : Slot) (suffix : List Slot)
    (owned : focused ledger leftSlots slot suffix) (live : slot.refs ≠ 0) :
    release ledger leftSlots slot suffix = .ok (replaced ledger leftSlots slot suffix (slot.refs - 1)) := by
  simp [release, owned, live]

theorem grant_cannot_revoke (ledger : Ledger) (quota : Option Nat) (after : Ledger)
    (accepted : grant ledger quota = .ok after) :
    after.slots = ledger.slots ∧ after.peak = ledger.peak ∧ fitsQuota quota (total after.slots) = true := by
  unfold grant at accepted
  split at accepted
  · contradiction
  · split at accepted
    · rename_i fits
      cases accepted
      exact ⟨rfl, rfl, fits⟩
    · contradiction

/-- Old storage already belongs to the same ledger. A replacement must admit
all new bytes in addition to that old owner, not merely their difference. -/
theorem growth_coexistence (ledger : Ledger) (kind newBytes : Nat) :
    (admitted ledger kind newBytes).peak ≥ total ledger.slots + newBytes := by
  exact Nat.le_max_right _ _

theorem rollback_total (ledger : Ledger) (kind bytes : Nat) :
    total (replaced (admitted ledger kind bytes) ledger.slots ⟨kind, bytes, 1⟩ [] 0).slots =
      total ledger.slots := by simp [replaced, total_append, total, charge]

theorem rollback_keeps_peak (ledger : Ledger) (kind bytes : Nat) :
    (replaced (admitted ledger kind bytes) ledger.slots ⟨kind, bytes, 1⟩ [] 0).peak =
      max ledger.peak (total ledger.slots + bytes) := rfl

theorem nonresumable_never_ready (ledger : Ledger) (wait : Wait Identity)
    (permanent : resumable wait.reason = false) : ready ledger wait = false := by
  simp [ready, permanent]

theorem foreign_wait_never_ready (ledger : Ledger) (wait : Wait Identity)
    (foreign : wait.account ≠ ledger.account) : ready ledger wait = false := by
  simp [ready, foreign]

theorem resume_preserves_computation (ledger : Ledger) (wait : Wait Identity) (result : Identity)
    (resumed : resume ledger wait = some result) : result = wait.computation := by
  unfold resume at resumed
  split at resumed
  · cases resumed; rfl
  · contradiction

/-- Unboxing releases the allocation, not arbitrary nested payloads. Moving a
computation or payload creates no new allocation slot or alias by itself. -/
def moveAccounting (ledger : Ledger) : Ledger := ledger

/-- Only the accounting observation of a move, not a theorem that an arbitrary
native driver transferred a linear lease, name, look or work balance. -/
theorem move_does_not_allocate (ledger : Ledger) : moveAccounting ledger = ledger := rfl


theorem quota_downward (quota : Option Nat) (small large : Nat) (bound : small ≤ large)
    (accepted : fitsQuota quota large = true) : fitsQuota quota small = true := by
  cases quota with
  | none => rfl
  | some limit => simp only [fitsQuota, decide_eq_true_eq] at *; omega

theorem admissible_downward (ledger : Ledger) (small large : Nat) (bound : small ≤ large)
    (accepted : admissible ledger large = true) : admissible ledger small = true := by
  simp only [admissible, Bool.and_eq_true, decide_eq_true_eq] at *
  exact ⟨⟨accepted.1.1, by omega⟩, quota_downward _ _ _ (by omega) accepted.2⟩

theorem release_preserves_valid (ledger : Ledger) (leftSlots : List Slot) (slot : Slot) (suffix : List Slot)
    (before : valid ledger) (owned : focused ledger leftSlots slot suffix) :
    valid (replaced ledger leftSlots slot suffix (slot.refs - 1)) := by
  have bound := release_cannot_increase ledger leftSlots slot suffix owned
  unfold valid at *
  exact ⟨Nat.le_trans bound before.1, Nat.le_trans bound before.2.1,
    quota_downward _ _ _ bound before.2.2.1, before.2.2.2⟩

theorem alias_preserves_valid (ledger : Ledger) (leftSlots : List Slot) (slot : Slot) (suffix : List Slot)
    (before : valid ledger) (owned : focused ledger leftSlots slot suffix) (live : slot.refs ≠ 0) :
    valid (replaced ledger leftSlots slot suffix (slot.refs + 1)) := by
  unfold valid
  rw [alias_total ledger leftSlots slot suffix owned live]
  exact before

/-- This is conditional eligibility, not a promise of eventual service or of
allocator success. Releasing some bytes may still leave too little capacity. -/
theorem release_preserves_eligibility (ledger : Ledger) (leftSlots : List Slot) (slot : Slot)
    (suffix : List Slot) (wait : Wait Identity) (owned : focused ledger leftSlots slot suffix)
    (eligible : ready ledger wait = true) :
    ready (replaced ledger leftSlots slot suffix (slot.refs - 1)) wait = true := by
  have bound := release_cannot_increase ledger leftSlots slot suffix owned
  simp only [ready, admissible, Bool.and_eq_true, decide_eq_true_eq] at *
  simp only [replaced] at bound ⊢
  exact ⟨eligible.1, ⟨⟨eligible.2.1.1, by omega⟩,
    quota_downward _ _ _ (by omega) eligible.2.2⟩⟩


/-- An owned MemoryBox is exclusive. Its borrowed readiness predicate cannot
receive the allocation token. On success the container slot dies and the same
post-predicate payload is returned, including separately owned nested allocation
handles. A native borrowed predicate may use interior mutability or unwind: this
model begins after it returns and does not assert pre-callback content equality. -/
def unbox (ledger : Ledger) (leftSlots : List Slot) (slot : Slot) (suffix : List Slot)
    (payload : Payload) (isReady : Bool) : Except Refusal (Ledger × Payload) :=
  if !isReady then .error .notReady
  else if slot.refs ≠ 1 then .error .shared
  else match release ledger leftSlots slot suffix with
    | .error reason => .error reason
    | .ok after => .ok (after, payload)

theorem unbox_moves_same_payload (ledger : Ledger) (leftSlots : List Slot) (slot : Slot)
    (suffix : List Slot) (payload result : Payload) (isReady : Bool) (after : Ledger)
    (accepted : unbox ledger leftSlots slot suffix payload isReady = .ok (after, result)) :
    result = payload := by
  unfold unbox at accepted
  split at accepted
  · contradiction
  · split at accepted
    · contradiction
    · split at accepted
      · contradiction
      · cases accepted; rfl

theorem unbox_releases_container (ledger : Ledger) (leftSlots : List Slot) (slot : Slot)
    (suffix : List Slot) (payload : Payload) (owned : focused ledger leftSlots slot suffix)
    (exclusive : slot.refs = 1) :
    unbox ledger leftSlots slot suffix payload true =
      .ok (replaced ledger leftSlots slot suffix 0, payload) := by
  simp [unbox, release, owned, exclusive]

theorem unready_cannot_unbox (ledger : Ledger) (leftSlots : List Slot) (slot : Slot)
    (suffix : List Slot) (payload : Payload) :
    unbox ledger leftSlots slot suffix payload false = .error .notReady := by simp [unbox]


/-- Removing one chosen class partitions every occupied slot exactly once. -/
def withoutClass (kind : Nat) : List Slot → List Slot
  | [] => []
  | slot :: rest => if slot.kind = kind then withoutClass kind rest else slot :: withoutClass kind rest

theorem class_partition (kind : Nat) (slots : List Slot) :
    total slots = classTotal kind slots + total (withoutClass kind slots) := by
  induction slots with
  | nil => simp [total, classTotal, withoutClass]
  | cons slot rest ih =>
      by_cases same : slot.kind = kind <;>
        simp [total, classTotal, withoutClass, same, ih, Nat.add_assoc, Nat.add_left_comm]

/-- Repeated labels cannot double-charge a class. A finite caller's complete
class list accounts for all bytes when its complete class list leaves no remaining slots. -/
def partition (classes : List Nat) (slots : List Slot) : Nat × List Slot :=
  match classes with
  | [] => (0, slots)
  | kind :: rest =>
      let result := partition rest (withoutClass kind slots)
      (classTotal kind slots + result.1, result.2)

theorem finite_class_partition (classes : List Nat) (slots : List Slot) :
    total slots = (partition classes slots).1 + total (partition classes slots).2 := by
  induction classes generalizing slots with
  | nil => simp [partition]
  | cons kind rest ih =>
      have first := class_partition kind slots
      have later := ih (withoutClass kind slots)
      simp only [partition]
      omega

theorem complete_classes_conserve_bytes (classes : List Nat) (slots : List Slot)
    (covered : (partition classes slots).2 = []) :
    total slots = (partition classes slots).1 := by
  have all := finite_class_partition classes slots
  rw [covered] at all
  simpa [total] using all

def occupied (slot : Slot) : Nat := if slot.refs = 0 ∨ slot.bytes = 0 then 0 else 1

def reservations : List Slot → Nat
  | [] => 0
  | slot :: rest => occupied slot + reservations rest

theorem reservations_append (left right : List Slot) :
    reservations (left ++ right) = reservations left + reservations right := by
  induction left with
  | nil => simp [reservations]
  | cons slot rest ih => simp [reservations, ih, Nat.add_assoc]

theorem admitted_reservations (ledger : Ledger) (kind bytes : Nat) :
    reservations (admitted ledger kind bytes).slots = reservations ledger.slots + if bytes = 0 then 0 else 1 := by
  simp [admitted, reservations_append, reservations, occupied]

theorem alias_keeps_reservations (ledger : Ledger) (leftSlots : List Slot) (slot : Slot) (suffix : List Slot)
    (owned : focused ledger leftSlots slot suffix) (live : slot.refs ≠ 0) :
    reservations (replaced ledger leftSlots slot suffix (slot.refs + 1)).slots = reservations ledger.slots := by
  rw [owned]
  simp [replaced, reservations_append, reservations, occupied, live]

/-- Compose a successful fresh reservation with release of that fresh slot.
Every prior slot survives unchanged, even when payload construction failed. -/
theorem checked_rollback_preserves_old_slots (ledger grown : Ledger) (kind bytes : Nat)
    (accepted : reserve ledger kind bytes = .ok grown) :
    release grown ledger.slots ⟨kind,bytes,1⟩ [] =
      .ok { grown with slots := ledger.slots ++ [⟨kind,bytes,0⟩] } := by
  unfold reserve at accepted
  split at accepted
  · contradiction
  · split at accepted
    · contradiction
    · split at accepted
      · cases accepted
        simp [release, focused, admitted, replaced]
      · contradiction

/-- Replacement releases exactly the old exclusive slot after reserving the
whole new allocation. Unrelated slots retain their order and accounting fields; application payloads
are outside this model. -/
theorem checked_growth_replaces_only_old_slot (ledger grown : Ledger) (leftSlots suffix : List Slot)
    (old : Slot) (kind bytes : Nat) (owned : focused ledger leftSlots old suffix) (exclusive : old.refs = 1)
    (accepted : reserve ledger kind bytes = .ok grown) :
    release grown leftSlots old (suffix ++ [⟨kind,bytes,1⟩]) =
      .ok { grown with slots := leftSlots ++ { old with refs := 0 } :: (suffix ++ [⟨kind,bytes,1⟩]) } := by
  unfold focused at owned
  unfold reserve at accepted
  split at accepted
  · contradiction
  · split at accepted
    · contradiction
    · split at accepted
      · cases accepted
        simp [release, focused, admitted, replaced, owned, exclusive, List.append_assoc]
      · contradiction

theorem occupied_bounded_by_charge (slot : Slot) : occupied slot ≤ charge slot := by
  by_cases dead : slot.refs = 0 <;> by_cases empty : slot.bytes = 0 <;>
    simp [occupied, charge, dead, empty] <;> omega

theorem reservations_bounded_by_bytes (slots : List Slot) : reservations slots ≤ total slots := by
  induction slots with
  | nil => simp [reservations, total]
  | cons slot rest ih =>
      exact Nat.add_le_add (occupied_bounded_by_charge slot) ih

theorem valid_reservations_fit_word (ledger : Ledger) (bounded : valid ledger) :
    reservations ledger.slots ≤ ledger.maximum :=
  Nat.le_trans (reservations_bounded_by_bytes ledger.slots) bounded.2.1

end ResourceLedger
