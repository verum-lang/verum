/- Reference admission policy for execution credit. A confirmed receipt carries
   evidence for an explicit execution relation, not a Boolean success label.
   Instantiating that relation with actual application commits and authenticating
   receipts are external obligations; this is not a proof about Rust execution. -/

namespace ReceiptCredit

set_option warningAsError true

structure Head (Owner Revision : Type) where
  owner : Owner
  revision : Revision
  cursor : Nat
  deriving DecidableEq

structure State (Owner Revision : Type) where
  head : Head Owner Revision
  pending : Option (Head Owner Revision)
  credits : Nat

inductive Receipt {Owner Revision : Type} (Executed : Head Owner Revision → Prop) where
  | confirmed (head : Head Owner Revision) (evidence : Executed head)
  | unconfirmed (head : Head Owner Revision)

variable {Owner Revision : Type} [DecidableEq Owner] [DecidableEq Revision]
  {Executed : Head Owner Revision → Prop}

def Receipt.head : Receipt Executed → Head Owner Revision
  | .confirmed head _ => head
  | .unconfirmed head => head

def clear (state : State Owner Revision) : State Owner Revision :=
  { state with pending := none }

def advance (state : State Owner Revision) : State Owner Revision :=
  { head := { state.head with cursor := state.head.cursor + 1 }
    pending := none, credits := state.credits + 1 }

-- Selection alone spends nothing. Dispatch records the exact current head.
def dispatch (state : State Owner Revision) (selected : Head Owner Revision) : State Owner Revision :=
  { state with pending := if selected = state.head then some selected else none }

def settle (state : State Owner Revision) : Receipt Executed → State Owner Revision
  | .confirmed head _ =>
    if state.pending = some state.head ∧ head = state.head then advance state else clear state
  | .unconfirmed _ => clear state

def replace (state : State Owner Revision) (owner : Owner) (revision : Revision) : State Owner Revision :=
  { head := ⟨owner, revision, 0⟩, pending := none, credits := state.credits }

theorem dispatch_spends_nothing (state : State Owner Revision) (selected : Head Owner Revision) :
    (dispatch state selected).credits = state.credits ∧
    (dispatch state selected).head.cursor = state.head.cursor := by
  exact ⟨rfl, rfl⟩

theorem credit_requires_matching_execution (state : State Owner Revision) (receipt : Receipt Executed)
    (credited : (settle state receipt).credits = state.credits + 1) :
    Executed state.head ∧ state.pending = some state.head ∧ receipt.head = state.head := by
  cases receipt with
  | unconfirmed head =>
    exact False.elim ((Nat.ne_of_lt (Nat.lt_succ_self state.credits)) credited)
  | confirmed head evidence =>
    by_cases admissible : state.pending = some state.head ∧ head = state.head
    · exact ⟨admissible.2 ▸ evidence, admissible.1, admissible.2⟩
    · have unchanged : (settle state (.confirmed head evidence)).credits = state.credits := by
        simp only [settle, ite_eq_right admissible, clear]
      exact False.elim ((Nat.ne_of_lt (Nat.lt_succ_self state.credits)) (unchanged.symm.trans credited))

theorem no_dispatch_no_credit (state : State Owner Revision) (receipt : Receipt Executed)
    (noPending : state.pending = none) :
    (settle state receipt).credits = state.credits ∧
    (settle state receipt).head = state.head := by
  cases receipt <;> simp [settle, noPending, clear]

theorem stale_revision_preserves_credit (state : State Owner Revision) (receipt : Receipt Executed)
    (stale : receipt.head.revision ≠ state.head.revision) :
    (settle state receipt).credits = state.credits ∧
    (settle state receipt).head = state.head := by
  cases receipt with
  | unconfirmed head => exact ⟨rfl, rfl⟩
  | confirmed head evidence =>
    have different : head ≠ state.head := by
      intro equal
      exact stale (congrArg Head.revision equal)
    simp [settle, different, clear]

theorem duplicate_receipt_is_idempotent (state : State Owner Revision) (receipt : Receipt Executed) :
    settle (settle state receipt) receipt = settle state receipt := by
  cases receipt with
  | unconfirmed head => rfl
  | confirmed head evidence =>
    by_cases admissible : state.pending = some state.head ∧ head = state.head
    · simp [settle, admissible, advance, clear]
    · simp [settle, admissible, clear]

theorem replacement_requires_new_dispatch (state : State Owner Revision) (owner : Owner)
    (revision : Revision) (receipt : Receipt Executed) :
    (settle (replace state owner revision) receipt).credits = state.credits := by
  exact (no_dispatch_no_credit (replace state owner revision) receipt rfl).1

-- Executable mathematical controls for this abstract policy, not receipt
-- authentication or certificate rejection tests against an application.
namespace Controls
def first : Head Nat Nat := ⟨4, 2, 0⟩
def occurred (head : Head Nat Nat) : Prop := head = first
def receipt : Receipt occurred := .confirmed first rfl
def ready : State Nat Nat := ⟨first, some first, 0⟩

example : (settle ready receipt).credits = 1 := by decide
example : (settle (clear ready) receipt).credits = 0 := by decide
example : (settle ready (.unconfirmed first : Receipt occurred)).credits = 0 := by decide
example : (settle ⟨⟨9, 2, 0⟩, some ⟨9, 2, 0⟩, 0⟩ receipt).credits = 0 := by decide
example : (settle ⟨⟨4, 3, 0⟩, some ⟨4, 3, 0⟩, 0⟩ receipt).credits = 0 := by decide
example : (settle ⟨⟨4, 2, 1⟩, some ⟨4, 2, 1⟩, 0⟩ receipt).credits = 0 := by decide
example : (settle (settle ready receipt) receipt).credits = 1 := by decide
end Controls

#print axioms credit_requires_matching_execution
#print axioms dispatch_spends_nothing
#print axioms no_dispatch_no_credit
#print axioms stale_revision_preserves_credit
#print axioms duplicate_receipt_is_idempotent
#print axioms replacement_requires_new_dispatch

end ReceiptCredit
