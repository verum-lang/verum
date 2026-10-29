/-! Immutable bindings with observed owner lifetimes. Keys denote retained
allocation identities; a payload can contain a revision, inputs and other
owners. This accepts checkpoints, not unobserved native operations, and proves
neither allocation freshness nor the private constructors of foreign code.
-/
namespace RetainedOwnership
set_option warningAsError true
variable {Owner Key Payload : Type}

structure State (Owner Key Payload : Type) where
  active : Owner → Option Key
  known : Key → Option (Owner × Payload)

def empty : State Owner Key Payload := ⟨fun _ => none, fun _ => none⟩

def release [DecidableEq Owner] (s : State Owner Key Payload) (owner : Owner) : State Owner Key Payload :=
  { s with active := fun o => if o = owner then none else s.active o }

def register [DecidableEq Owner] [DecidableEq Key]
    (s : State Owner Key Payload) (owner : Owner) (key : Key) (payload : Payload) : State Owner Key Payload :=
  ⟨fun o => if o = owner then some key else s.active o,
   fun k => if k = key then some (owner, payload) else s.known k⟩

def observe [DecidableEq Owner] [DecidableEq Key] [DecidableEq Payload]
    (s : State Owner Key Payload) (owner : Owner) (binding : Option (Key × Payload)) :
    Option (State Owner Key Payload) :=
  match binding with
  | none => some (release s owner)
  | some (key, payload) =>
      match s.known key with
      | none => some (register s owner key payload)
      | some original =>
          if original = (owner, payload) ∧ s.active owner = some key then some s else none

def valid [DecidableEq Owner] [DecidableEq Key] [DecidableEq Payload]
    (events : List (Owner × Option (Key × Payload))) : Bool :=
  (events.foldlM (fun (s : State Owner Key Payload) event => observe s event.1 event.2) empty).isSome

theorem known_repeat_iff [DecidableEq Owner] [DecidableEq Key] [DecidableEq Payload]
    (s : State Owner Key Payload) (owner : Owner) (key : Key) (payload : Payload)
    (original : Owner × Payload) (known : s.known key = some original) :
    (observe s owner (some (key,payload))).isSome = true ↔
      original = (owner,payload) ∧ s.active owner = some key := by
  simp [observe, known]

theorem known_preserved [DecidableEq Owner] [DecidableEq Key] [DecidableEq Payload]
    (before after : State Owner Key Payload) (owner : Owner) (binding : Option (Key × Payload))
    (key : Key) (original : Owner × Payload) (known : before.known key = some original)
    (accepted : observe before owner binding = some after) : after.known key = some original := by
  cases binding with
  | none => cases Option.some.inj accepted; exact known
  | some binding =>
      rcases binding with ⟨offered, payload⟩
      unfold observe at accepted
      cases found : before.known offered with
      | none =>
          simp only [found, Option.some.injEq] at accepted
          subst after
          have different : key ≠ offered := by intro equal; subst key; rw [found] at known; cases known
          simpa [register, different] using known
      | some prior =>
          simp only [found] at accepted
          split at accepted
          · cases Option.some.inj accepted; exact known
          · cases accepted

theorem changed_binding_refused [DecidableEq Owner] [DecidableEq Key] [DecidableEq Payload]
    (s : State Owner Key Payload) (owner : Owner) (key : Key) (payload : Payload)
    (original : Owner × Payload) (known : s.known key = some original)
    (different : original ≠ (owner,payload)) : observe s owner (some (key,payload)) = none := by
  simp [observe, known, different]

theorem inactive_refused [DecidableEq Owner] [DecidableEq Key] [DecidableEq Payload]
    (s : State Owner Key Payload) (owner : Owner) (key : Key) (payload : Payload)
    (original : Owner × Payload) (known : s.known key = some original)
    (inactive : s.active owner ≠ some key) : observe s owner (some (key,payload)) = none := by
  simp [observe, known, inactive]

theorem inactive_preserved [DecidableEq Owner] [DecidableEq Key] [DecidableEq Payload]
    (before after : State Owner Key Payload) (holder actor : Owner) (key : Key)
    (original : Owner × Payload) (binding : Option (Key × Payload))
    (known : before.known key = some original) (inactive : before.active holder ≠ some key)
    (accepted : observe before actor binding = some after) : after.active holder ≠ some key := by
  cases binding with
  | none =>
      cases Option.some.inj accepted
      simp only [release]
      split
      · simp
      · exact inactive
  | some binding =>
      rcases binding with ⟨offered, payload⟩
      unfold observe at accepted
      cases found : before.known offered with
      | none =>
          simp only [found, Option.some.injEq] at accepted
          subst after
          have different : key ≠ offered := by intro equal; subst key; rw [found] at known; cases known
          simp only [register]
          split
          · exact fun equal => different (Option.some.inj equal).symm
          · exact inactive
      | some prior =>
          simp only [found] at accepted
          split at accepted
          · cases Option.some.inj accepted; exact inactive
          · cases accepted

/-- Every known binding, including a still-active one, survives an accepted trace. -/
theorem trace_preserves_binding [DecidableEq Owner] [DecidableEq Key] [DecidableEq Payload]
    (events : List (Owner × Option (Key × Payload))) (before after : State Owner Key Payload)
    (key : Key) (original : Owner × Payload) (known : before.known key = some original)
    (accepted : events.foldlM (fun s event => observe s event.1 event.2) before = some after) :
    after.known key = some original := by
  induction events generalizing before with
  | nil => cases Option.some.inj accepted; exact known
  | cons event rest ih =>
      cases first : observe before event.1 event.2 with
      | none => simp [List.foldlM,first] at accepted
      | some middle =>
          apply ih middle (known_preserved before middle _ _ key original known first)
          simpa only [List.foldlM,first,bind,Option.bind] using accepted

/-- Both immutable ownership and retirement compose over the entire accepted
checkpoint sequence, including observations of unrelated owners. -/
theorem trace_preserves_retirement [DecidableEq Owner] [DecidableEq Key] [DecidableEq Payload]
    (events : List (Owner × Option (Key × Payload))) (before after : State Owner Key Payload)
    (holder : Owner) (key : Key) (original : Owner × Payload)
    (known : before.known key = some original) (inactive : before.active holder ≠ some key)
    (accepted : events.foldlM (fun s event => observe s event.1 event.2) before = some after) :
    after.known key = some original ∧ after.active holder ≠ some key := by
  induction events generalizing before with
  | nil => cases Option.some.inj accepted; exact ⟨known,inactive⟩
  | cons event rest ih =>
      cases first : observe before event.1 event.2 with
      | none => simp [List.foldlM,first] at accepted
      | some middle =>
          apply ih middle (known_preserved before middle _ _ key original known first)
            (inactive_preserved before middle holder _ key original _ known inactive first)
          simpa only [List.foldlM,first,bind,Option.bind] using accepted

theorem release_prevents_resurrection [DecidableEq Owner] [DecidableEq Key] [DecidableEq Payload]
    (s : State Owner Key Payload) (owner : Owner) (key : Key) (payload : Payload)
    (original : Owner × Payload) (known : s.known key = some original) :
    observe (release s owner) owner (some (key,payload)) = none := by
  simp [observe, release, known]

theorem replacement_prevents_resurrection [DecidableEq Owner] [DecidableEq Key] [DecidableEq Payload]
    (s : State Owner Key Payload) (owner : Owner) (old fresh : Key) (payload next : Payload)
    (original : Owner × Payload) (known : s.known old = some original) (different : old ≠ fresh) :
    observe (register s owner fresh next) owner (some (old,payload)) = none := by
  simp [observe, register, different, known, Ne.symm different]

end RetainedOwnership
