import VerumExternalReplay.ExactNames
import VerumExternalReplay.GuideFallback

/-! Finite ordinal registries of complete native actions. A list is the canonical
ordinal-indexed representation, not the iteration order of a native map. The
map/list bijection, faithful equality, machine-width conversions, storage
admission, concurrency and all-or-nothing publication are native correspondence
obligations. Natural-number bounds are checked before publication; this model
neither promises unbounded storage nor imposes a benchmark alphabet cap.
The native sorted fresh-action order must match the prepared batch order here;
set equality alone does not establish ordinal correspondence.
Context fields are kept separate. Registry ownership remains external to a
Context: every transport validates the complete action under the donor first. -/
namespace ActionRegistry
set_option warningAsError true

variable {A : Type} [DecidableEq A]

def name (keys : List A) (action : A) : Option Nat :=
  if action ∈ keys then some (keys.idxOf action) else none

theorem name_exact (keys : List A) (action : A) (present : action ∈ keys) :
    name keys action = some (keys.idxOf action) := by simp [name, present]

theorem name_sound (keys : List A) (action : A) (rank : Nat)
    (named : name keys action = some rank) : keys[rank]? = some action := by
  unfold name at named
  split at named
  · rename_i present
    have same : keys.idxOf action = rank := Option.some.inj named
    subst rank
    rw [List.getElem?_eq_getElem (List.idxOf_lt_length_of_mem present)]
    exact congrArg some (List.getElem_idxOf (List.idxOf_lt_length_of_mem present))
  · cases named

/-- In a unique registry, lookup identifies the actual naming result, not only
an occurrence of the action at an arbitrary duplicate position. -/
theorem lookup_name (keys : List A) (action : A) (rank : Nat) (valid : keys.Nodup)
    (lookup : keys[rank]? = some action) : name keys action = some rank := by
  have present := List.mem_of_getElem? lookup
  have named := name_exact keys action present
  have same := valid.eq_of_getElem?_eq (List.idxOf_lt_length_of_mem present)
    ((name_sound keys action _ named).trans lookup.symm)
  simpa only [same] using named

theorem distinct_names (keys : List A) (a b : A) (i j : Nat)
    (left : name keys a = some i) (right : name keys b = some j)
    (distinct : a ≠ b) : i ≠ j := by
  intro same
  subst j
  exact distinct (ExactNames.same_name_same_key keys i a b
    (name_sound keys a i left) (name_sound keys b i right))

theorem name_extension (keys suffix : List A) (action : A)
    (present : action ∈ keys) : name (keys ++ suffix) action = name keys action := by
  simp [name, present, List.mem_append_left suffix present, List.idxOf_append]

def fresh (keys batch : List A) : List A := GuideFallback.emit keys batch

theorem fresh_membership (keys batch : List A) (action : A) :
    action ∈ fresh keys batch ↔ action ∈ batch ∧ action ∉ keys :=
  GuideFallback.emit_mem action keys batch

theorem extended_unique (keys batch : List A) (valid : keys.Nodup) :
    (keys ++ fresh keys batch).Nodup := by
  simp only [List.nodup_append]
  refine ⟨valid, GuideFallback.emit_unique keys batch, ?_⟩
  intro a old b added same
  subst b
  exact (fresh_membership keys batch a).mp added |>.2 old

theorem batch_covered (keys batch : List A) (action : A) (present : action ∈ batch) :
    action ∈ keys ++ fresh keys batch := by
  by_cases old : action ∈ keys
  · exact List.mem_append_left _ old
  · exact List.mem_append_right _ ((fresh_membership keys batch action).mpr ⟨present, old⟩)

def freshNames (keys batch : List A) : List Nat :=
  List.range' keys.length (fresh keys batch).length

theorem fresh_interval (keys batch : List A) (rank : Nat) :
    rank ∈ freshNames keys batch ↔
      keys.length ≤ rank ∧ rank < keys.length + (fresh keys batch).length := by
  simp only [freshNames, List.mem_range', Nat.one_mul]
  constructor
  · rintro ⟨i, hi, rfl⟩; omega
  · rintro ⟨lo, hi⟩
    exact ⟨rank - keys.length, by omega, by omega⟩

theorem fresh_disjoint (keys batch : List A) (rank : Nat)
    (old : rank < keys.length) : rank ∉ freshNames keys batch := by
  intro h
  have := (fresh_interval keys batch rank).mp h
  omega

theorem fresh_lookup (keys batch : List A) (index : Nat) :
    (keys ++ fresh keys batch)[keys.length + index]? = (fresh keys batch)[index]? := by
  rw [List.getElem?_append_right (by omega)]
  simp

/-- Fresh interval positions are the actual assigned names. -/
theorem fresh_name (keys batch : List A) (valid : keys.Nodup) (index : Nat) (action : A)
    (located : (fresh keys batch)[index]? = some action) :
    name (keys ++ fresh keys batch) action = some (keys.length + index) :=
  lookup_name _ action _ (extended_unique keys batch valid)
    ((fresh_lookup keys batch index).trans located)

theorem issued_iff_fresh_name (keys batch : List A) (valid : keys.Nodup) (rank : Nat) :
    rank ∈ freshNames keys batch ↔ ∃ action ∈ fresh keys batch,
      name (keys ++ fresh keys batch) action = some rank := by
  constructor
  · intro issued
    have limits := (fresh_interval keys batch rank).mp issued
    have indexBound : rank - keys.length < (fresh keys batch).length := by omega
    have located := List.getElem?_eq_getElem indexBound
    refine ⟨(fresh keys batch)[rank - keys.length], List.mem_of_getElem? located, ?_⟩
    have named := fresh_name keys batch valid (rank - keys.length) _ located
    have same : keys.length + (rank - keys.length) = rank := by omega
    simpa only [same] using named
  · rintro ⟨action, member, named⟩
    obtain ⟨index, located⟩ := List.getElem?_of_mem member
    have same := Option.some.inj ((fresh_name keys batch valid index action located).symm.trans named)
    have indexBound := (List.getElem?_eq_some_iff.mp located).1
    apply (fresh_interval keys batch rank).mpr
    omega

inductive Refusal where
  | malformedRegistry | addressability | admission
  deriving DecidableEq

/-- `bound` is an exclusive ordinal bound AND an admitted list-count bound;
the native caller must choose it to satisfy every machine representation. A
separate resource admission decision is supplied for the entire prepared batch.
All failure branches return the original registry, never a partial append. -/
def extend (bound : Nat) (admitted : Bool) (keys batch : List A) :
    List A × Except Refusal (List Nat) :=
  if ¬ keys.Nodup then (keys, .error .malformedRegistry)
  else if keys.length + (fresh keys batch).length > bound then
    (keys, .error .addressability)
  else if admitted = false then (keys, .error .admission)
  else (keys ++ fresh keys batch, .ok (freshNames keys batch))

theorem extension_accepts (bound : Nat) (keys batch : List A) (valid : keys.Nodup)
    (fits : keys.length + (fresh keys batch).length ≤ bound) :
    extend bound true keys batch =
      (keys ++ fresh keys batch, .ok (freshNames keys batch)) := by
  simp [extend, valid, Nat.not_lt_of_ge fits]

theorem success_facts (bound : Nat) (admitted : Bool) (keys batch : List A)
    (ranks : List Nat) (success : (extend bound admitted keys batch).2 = .ok ranks) :
    keys.Nodup ∧ keys.length + (fresh keys batch).length ≤ bound ∧ admitted = true ∧
      (extend bound admitted keys batch).1 = keys ++ fresh keys batch ∧
      ranks = freshNames keys batch := by
  by_cases valid : keys.Nodup
  · by_cases fits : keys.length + (fresh keys batch).length ≤ bound
    · cases admitted <;> simp_all [extend, Nat.not_lt_of_ge fits]
    · have excess : bound < keys.length + (fresh keys batch).length := by omega
      simp [extend, valid, excess] at success
  · simp [extend, valid] at success

theorem refusal_preserves (bound : Nat) (admitted : Bool) (keys batch : List A)
    (error : Refusal) (failure : (extend bound admitted keys batch).2 = .error error) :
    (extend bound admitted keys batch).1 = keys := by
  by_cases valid : keys.Nodup
  · by_cases fits : keys.length + (fresh keys batch).length ≤ bound
    · cases admitted <;> simp_all [extend, Nat.not_lt_of_ge fits]
    · have excess : bound < keys.length + (fresh keys batch).length := by omega
      simp [extend, valid, excess]
  · simp [extend, valid]

theorem address_refusal (bound : Nat) (admitted : Bool) (keys batch : List A)
    (valid : keys.Nodup) (excess : bound < keys.length + (fresh keys batch).length) :
    extend bound admitted keys batch = (keys, .error .addressability) := by
  simp [extend, valid, excess]

theorem admission_refusal (bound : Nat) (keys batch : List A) (valid : keys.Nodup)
    (fits : keys.length + (fresh keys batch).length ≤ bound) :
    extend bound false keys batch = (keys, .error .admission) := by
  simp [extend, valid, Nat.not_lt_of_ge fits]

theorem success_addressable (bound : Nat) (admitted : Bool) (keys batch : List A)
    (ranks : List Nat) (success : (extend bound admitted keys batch).2 = .ok ranks)
    (rank : Nat) (issued : rank ∈ ranks) : rank < bound := by
  obtain ⟨_, fits, _, _, rfl⟩ := success_facts bound admitted keys batch ranks success
  have h := (fresh_interval keys batch rank).mp issued
  omega

/-- Every address in the published registry fits, including pre-existing names.
The count bound is established by the transaction, not by truncating a rank. -/
theorem success_all_addressable (bound : Nat) (admitted : Bool) (keys batch : List A)
    (ranks : List Nat) (success : (extend bound admitted keys batch).2 = .ok ranks)
    (rank : Nat) (action : A)
    (lookup : (extend bound admitted keys batch).1[rank]? = some action) : rank < bound := by
  obtain ⟨_, fits, _, grown, _⟩ := success_facts bound admitted keys batch ranks success
  rw [grown] at lookup
  have h := (List.getElem?_eq_some_iff.mp lookup).1
  simp only [List.length_append] at h
  omega

theorem success_preserves_names (bound : Nat) (admitted : Bool) (keys batch : List A)
    (ranks : List Nat) (success : (extend bound admitted keys batch).2 = .ok ranks)
    (action : A) (present : action ∈ keys) :
    name (extend bound admitted keys batch).1 action = name keys action := by
  rw [(success_facts bound admitted keys batch ranks success).2.2.2.1]
  exact name_extension keys (fresh keys batch) action present

abbrev StateId := Fin (2 ^ 64)

structure Context where
  state : StateId
  ordinal : Nat
  deriving DecidableEq

theorem context_injective (s t : StateId) (i j : Nat) :
    (Context.mk s i = Context.mk t j) ↔ s = t ∧ i = j := by simp only [Context.mk.injEq]

/-- Both the complete native action and its rank must agree with one valid
registry. An in-range rank by itself is not an ownership receipt. -/
def ownsAt (keys : List A) (rank : Nat) (action : A) : Bool :=
  decide (keys.Nodup ∧ keys[rank]? = some action)

def bind (keys : List A) (state : StateId) (action : A) : Option Context :=
  if keys.Nodup ∧ action ∈ keys then some ⟨state, keys.idxOf action⟩ else none

theorem bind_exact (keys : List A) (state : StateId) (action : A)
    (valid : keys.Nodup) (present : action ∈ keys) :
    bind keys state action = some ⟨state, keys.idxOf action⟩ := by simp [bind, valid, present]

theorem fresh_bind (keys batch : List A) (valid : keys.Nodup) (state : StateId)
    (index : Nat) (action : A) (located : (fresh keys batch)[index]? = some action) :
    bind (keys ++ fresh keys batch) state action = some ⟨state, keys.length + index⟩ := by
  have present := List.mem_append_right keys (List.mem_of_getElem? located)
  have named := fresh_name keys batch valid index action located
  rw [name_exact _ _ present] at named
  rw [bind_exact _ _ _ (extended_unique keys batch valid) present, Option.some.inj named]

theorem bind_sound (keys : List A) (state : StateId) (action : A) (context : Context)
    (bound : bind keys state action = some context) :
    context.state = state ∧ ownsAt keys context.ordinal action = true := by
  unfold bind at bound
  split at bound
  · rename_i valid
    cases Option.some.inj bound
    refine ⟨rfl, ?_⟩
    simp only [ownsAt, decide_eq_true_eq]
    exact ⟨valid.1, name_sound keys action _ (name_exact keys action valid.2)⟩
  · cases bound

theorem bind_missing (keys : List A) (state : StateId) (action : A)
    (missing : action ∉ keys) : bind keys state action = none := by simp [bind, missing]

theorem bind_malformed (keys : List A) (state : StateId) (action : A)
    (malformed : ¬ keys.Nodup) : bind keys state action = none := by simp [bind, malformed]

theorem context_extension (keys batch : List A) (state : StateId) (action : A)
    (valid : keys.Nodup) (present : action ∈ keys) :
    bind (keys ++ fresh keys batch) state action = bind keys state action := by
  rw [bind_exact _ _ _ (extended_unique keys batch valid) (List.mem_append_left _ present),
    bind_exact _ _ _ valid present]
  simp [List.idxOf_append, present]

theorem wrong_owner_refused (keys : List A) (rank : Nat) (action : A)
    (wrong : keys[rank]? ≠ some action) : ownsAt keys rank action = false := by
  simp [ownsAt, wrong]

/-- Two distinct full actions cannot both own one ordinal. This checks alias
claims at the import boundary; a canonical ordinal list cannot itself encode a
many-to-one map. Native import must establish that canonical representation. -/
theorem alias_claims_refused (keys : List A) (rank : Nat) (a b : A) (distinct : a ≠ b) :
    (ownsAt keys rank a && ownsAt keys rank b) = false := by
  simp only [ownsAt, Bool.and_eq_false_iff, decide_eq_false_iff_not]
  by_cases left : keys.Nodup ∧ keys[rank]? = some a
  · right
    intro right
    exact distinct (ExactNames.same_name_same_key keys rank a b left.2 right.2)
  · exact Or.inl left

/-- Decode donor ownership through the supplied full native action; then bind
that same action in the receiver. Situation identity is transported unchanged. -/
def rebind (donor receiver : List A) (context : Context) (action : A) : Option Context :=
  if ownsAt donor context.ordinal action then bind receiver context.state action else none

theorem rebind_commutes (donor receiver : List A) (state : StateId) (action : A)
    (before after : Context) (source : bind donor state action = some before)
    (target : bind receiver state action = some after) :
    rebind donor receiver before action = some after := by
  obtain ⟨same, owned⟩ := bind_sound donor state action before source
  simp [rebind, owned, same, target]

theorem rebind_wrong_owner (donor receiver : List A) (context : Context) (action : A)
    (wrong : donor[context.ordinal]? ≠ some action) :
    rebind donor receiver context action = none := by
  simp [rebind, wrong_owner_refused donor context.ordinal action wrong]

theorem rebind_missing (donor receiver : List A) (context : Context) (action : A)
    (missing : action ∉ receiver) : rebind donor receiver context action = none := by
  simp [rebind, bind_missing receiver context.state action missing]

theorem rebind_sound (donor receiver : List A) (context after : Context) (action : A)
    (success : rebind donor receiver context action = some after) :
    ownsAt donor context.ordinal action = true ∧ after.state = context.state ∧
      ownsAt receiver after.ordinal action = true := by
  unfold rebind at success
  split at success
  · rename_i owned
    exact ⟨owned, bind_sound receiver context.state action after success⟩
  · cases success

/-- A successful union makes each donor action available under its receiver
ordinal. Copying the donor ordinal is neither required nor justified. -/
theorem merge_rebind (bound : Nat) (admitted : Bool) (donor receiver : List A)
    (ranks : List Nat) (accepted : (extend bound admitted receiver donor).2 = .ok ranks)
    (state : StateId) (action : A) (before : Context)
    (source : bind donor state action = some before) :
    ∃ after, rebind donor (extend bound admitted receiver donor).1 before action = some after ∧
      after.state = state ∧
      ownsAt (extend bound admitted receiver donor).1 after.ordinal action = true := by
  obtain ⟨valid, _, _, grown, _⟩ := success_facts bound admitted receiver donor ranks accepted
  have present : action ∈ donor := by
    unfold bind at source
    split at source
    · rename_i h; exact h.2
    · cases source
  let after : Context := ⟨state, (receiver ++ fresh receiver donor).idxOf action⟩
  have target := bind_exact (receiver ++ fresh receiver donor) state action
    (extended_unique receiver donor valid) (batch_covered receiver donor action present)
  rw [grown]
  exact ⟨after, rebind_commutes donor _ state action before after source target,
    bind_sound _ state action after target⟩

end ActionRegistry
