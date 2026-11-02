import Std

/-! Replay of the bounded dependency fragment of ordered record telescopes.
    Slot and receiver numbers are identities supplied by the elaborator. -/
namespace Verum.RecordTelescope

private def ascendingFrom (previous : Nat) : List Nat → Bool
  | [] => true
  | next :: rest => decide (previous < next) && ascendingFrom next rest

private def ascending : List Nat → Bool
  | [] => true
  | head :: rest => ascendingFrom head rest

private theorem ascendingFrom_sound {previous : Nat} {rest : List Nat}
    (h : ascendingFrom previous rest = true) : (previous :: rest).Pairwise (· < ·) := by
  induction rest generalizing previous with
  | nil => simp
  | cons next rest ih =>
    have ⟨earlier, remaining⟩ := Bool.and_eq_true_iff.mp h
    have lt : previous < next := of_decide_eq_true earlier
    have tail := ih remaining
    refine List.pairwise_cons.mpr ⟨?_, tail⟩
    intro slot member
    rcases List.mem_cons.mp member with rfl | inRest
    · exact lt
    · exact Nat.lt_trans lt ((List.pairwise_cons.mp tail).1 slot inRest)

private theorem ascending_sound {row : List Nat} (h : ascending row = true) :
    row.Pairwise (· < ·) := by
  cases row with
  | nil => simp
  | cons head rest => exact ascendingFrom_sound h

/-- A row lists distinct earlier slots in ascending order. -/
def form (rows : List (List Nat)) : Bool :=
  decide (rows.length ≤ 4096) && decide ((rows.map List.length).sum ≤ 16384) &&
    rows.zipIdx.all (fun (row, field) => ascending row && row.all (fun slot => decide (slot < field)))

theorem form_sound {rows : List (List Nat)} (h : form rows = true) :
    rows.length ≤ 4096 ∧ (rows.map List.length).sum ≤ 16384 ∧
      ∀ (field : Nat) (row : List Nat), rows[field]? = some row →
        row.Pairwise (· < ·) ∧ ∀ slot ∈ row, slot < field := by
  obtain ⟨limits, checked⟩ := Bool.and_eq_true_iff.mp h
  obtain ⟨fields, edges⟩ := Bool.and_eq_true_iff.mp limits
  refine ⟨of_decide_eq_true fields, of_decide_eq_true edges, ?_⟩
  intro field row atField
  have each := List.all_eq_true.mp checked (row, field) (List.mk_mem_zipIdx_iff_getElem?.mpr atField)
  obtain ⟨ordered, earlier⟩ := Bool.and_eq_true_iff.mp each
  exact ⟨ascending_sound ordered, fun slot member =>
    of_decide_eq_true (List.all_eq_true.mp earlier slot member)⟩

private def bindSlots (witnesses : List (Option Nat)) : List Nat → Option (List (Nat × Nat))
  | [] => some []
  | slot :: rest => do
    let value ← witnesses[slot]?.join
    let tail ← bindSlots witnesses rest
    return (slot, value) :: tail

/-- `none` in the prefix is allowed exactly where this row needs no witness. -/
def construction (rows : List (List Nat)) (field : Nat) (witnesses : List (Option Nat)) :
    Option (List (Nat × Nat)) :=
  if form rows && witnesses.length == field then rows[field]?.bind (bindSlots witnesses) else none

private theorem bindSlots_sound {witnesses : List (Option Nat)} {row : List Nat}
    {args : List (Nat × Nat)} (h : bindSlots witnesses row = some args) :
    args.map Prod.fst = row ∧ ∀ slot value, (slot, value) ∈ args → witnesses[slot]? = some (some value) := by
  induction row generalizing args with
  | nil =>
    simp [bindSlots] at h
    subst args
    simp
  | cons slot rest ih =>
    cases found : witnesses[slot]? with
    | none => simp [bindSlots, found] at h
    | some witness =>
      cases witness with
      | none => simp [bindSlots, found] at h
      | some value =>
        cases remaining : bindSlots witnesses rest with
        | none => simp [bindSlots, found, remaining] at h
        | some tail =>
          simp [bindSlots, found, remaining] at h
          subst args
          obtain ⟨keys, values⟩ := ih remaining
          refine ⟨by simp [keys], ?_⟩
          intro other v member
          rcases List.mem_cons.mp member with equal | inTail
          · cases equal
            exact found
          · exact values other v inTail

theorem construction_sound {rows : List (List Nat)} {field : Nat} {witnesses : List (Option Nat)}
    {args : List (Nat × Nat)} (h : construction rows field witnesses = some args) :
    form rows = true ∧ witnesses.length = field ∧
      rows[field]? = some (args.map Prod.fst) ∧
      ∀ slot value, (slot, value) ∈ args → witnesses[slot]? = some (some value) := by
  unfold construction at h
  split at h
  next admitted =>
    obtain ⟨formed, length⟩ := Bool.and_eq_true_iff.mp admitted
    obtain ⟨row, atField, bound⟩ := Option.bind_eq_some_iff.mp h
    obtain ⟨keys, values⟩ := bindSlots_sound bound
    exact ⟨formed, eq_of_beq length, keys.symm ▸ atField, values⟩
  next => contradiction

def projection (rows : List (List Nat)) (field receiver : Nat) :
    Option (List (Nat × (Nat × Nat))) :=
  if form rows then rows[field]?.map (fun row => row.map (fun slot => (slot, (receiver, slot))))
  else none

theorem projection_sound {rows : List (List Nat)} {field receiver : Nat}
    {args : List (Nat × (Nat × Nat))} (h : projection rows field receiver = some args) :
    form rows = true ∧ ∃ row, rows[field]? = some row ∧
      args = row.map (fun slot => (slot, (receiver, slot))) := by
  unfold projection at h
  split at h
  next formed =>
    obtain ⟨row, atField, equal⟩ := Option.map_eq_some_iff.mp h
    exact ⟨formed, row, atField, equal.symm⟩
  next => contradiction

/-- Any shared projection argument pins both receivers to the same identity.
    An empty dependency row makes no receiver observation. -/
theorem projection_receiver_separation {rows : List (List Nat)} {field left right : Nat}
    {leftArgs rightArgs : List (Nat × (Nat × Nat))} {argument : Nat × (Nat × Nat)}
    (hl : projection rows field left = some leftArgs)
    (hr : projection rows field right = some rightArgs)
    (inLeft : argument ∈ leftArgs) (inRight : argument ∈ rightArgs) : left = right := by
  obtain ⟨_, _, _, rfl⟩ := projection_sound hl
  obtain ⟨_, _, _, rfl⟩ := projection_sound hr
  obtain ⟨slot, _, sameLeft⟩ := List.mem_map.mp inLeft
  obtain ⟨other, _, sameRight⟩ := List.mem_map.mp inRight
  exact congrArg (fun (p : Nat × (Nat × Nat)) => p.2.1) (sameLeft.trans sameRight.symm)

/-- Interpreting checked projections reads exactly the row's ordered slots
    from the selected receiver. -/
theorem projection_arguments {rows : List (List Nat)} {field receiver : Nat} {row : List Nat}
    {args : List (Nat × (Nat × Nat))} (h : projection rows field receiver = some args)
    (atField : rows[field]? = some row) {α : Type} (readSlot : Nat → Nat → α) :
    args.map (fun argument => readSlot argument.2.1 argument.2.2) = row.map (readSlot receiver) := by
  obtain ⟨_, found, atFound, rfl⟩ := projection_sound h
  have same : found = row := Option.some.inj (atFound.symm.trans atField)
  simp [same, List.map_map, Function.comp_def]

def mayMutate (rows : List (List Nat)) (field : Nat) : Option Bool :=
  if form rows && decide (field < rows.length) then
    some ((rows.drop (field + 1)).all (fun row => !row.contains field))
  else none

theorem malformed_schema_rejected {rows : List (List Nat)} (bad : form rows = false)
    (field receiver : Nat) (witnesses : List (Option Nat)) :
    construction rows field witnesses = none ∧ projection rows field receiver = none ∧
      mayMutate rows field = none := by simp [construction, projection, mayMutate, bad]

theorem mayMutate_sound {rows : List (List Nat)} {field : Nat}
    (h : mayMutate rows field = some true) : form rows = true ∧ field < rows.length ∧
      ∀ later row, field < later → rows[later]? = some row → field ∉ row := by
  unfold mayMutate at h
  split at h
  next admitted =>
    obtain ⟨formed, existsField⟩ := Bool.and_eq_true_iff.mp admitted
    refine ⟨formed, of_decide_eq_true existsField, ?_⟩
    intro later row isLater atLater
    have member : row ∈ rows.drop (field + 1) := by
      apply List.mem_iff_getElem?.mpr
      refine ⟨later - (field + 1), ?_⟩
      rw [List.getElem?_drop, show field + 1 + (later - (field + 1)) = later by omega]
      exact atLater
    simpa using List.all_eq_true.mp (Option.some.inj h) row member
  next => contradiction

/-- Changing an admitted field preserves the ordered argument tuple used by
    every later type family. -/
theorem mutation_preserves_later_arguments {rows : List (List Nat)} {field later : Nat}
    {row : List Nat} (h : mayMutate rows field = some true) (isLater : field < later)
    (atLater : rows[later]? = some row) (valueAt : Nat → Nat) (replacement : Nat) :
    row.map (fun slot => if slot = field then replacement else valueAt slot) = row.map valueAt := by
  have absent := (mayMutate_sound h).2.2 later row isLater atLater
  apply List.map_congr_left
  intro slot member
  have distinct : slot ≠ field := fun equal => absent (equal ▸ member)
  simp [distinct]

/-! A dependent field may be represented by any Lean type family over its
    ordered arguments. The witness interpretation below is an explicit premise. -/
universe u v

theorem construction_arguments {rows : List (List Nat)} {field : Nat} {witnesses : List (Option Nat)}
    {args : List (Nat × Nat)} {row : List Nat} (h : construction rows field witnesses = some args)
    (atField : rows[field]? = some row) (valueAt : Nat → Nat)
    (agrees : ∀ slot value, witnesses[slot]? = some (some value) → valueAt slot = value) :
    args.map Prod.snd = row.map valueAt := by
  obtain ⟨_, _, keys, values⟩ := construction_sound h
  have ordered : args.map Prod.fst = row := Option.some.inj (keys.symm.trans atField)
  rw [← ordered, List.map_map]
  apply List.map_congr_left
  intro ⟨slot, value⟩ member
  exact (agrees slot value (values slot value member)).symm

/-- Extend the very prefix that supplied the field witnesses. The type of
    the second component depends on the first component's argument tuple. -/
def checkedExtension {Prefix : Type u} (p : Prefix) (valueAt : Prefix → Nat → Nat)
    (Family : List Nat → Type v) {rows : List (List Nat)} {field : Nat}
    {witnesses : List (Option Nat)} {args : List (Nat × Nat)} {row : List Nat}
    (h : construction rows field witnesses = some args) (atField : rows[field]? = some row)
    (agrees : ∀ slot value, witnesses[slot]? = some (some value) → valueAt p slot = value)
    (value : Family (args.map Prod.snd)) : (prefixValue : Prefix) × Family (row.map (valueAt prefixValue)) :=
  ⟨p, construction_arguments h atField (valueAt p) agrees ▸ value⟩

/-- First-order field syntax. Constructor tags can be encoded as `root` atoms
    at the head of a tree. `slot` is the only substitutable constructor. -/
inductive FieldTerm where
  | root (id : Nat)
  | slot (id : Nat)
  | project (receiver field : Nat)
  | pair (left right : FieldTerm)
  deriving DecidableEq, Repr

def FieldTerm.nodes : FieldTerm → Nat
  | .pair left right => 1 + left.nodes + right.nodes
  | _ => 1

private def expandedNodes (bindings : List (Nat × FieldTerm)) : FieldTerm → Option Nat
  | .slot id => (bindings.lookup id).map FieldTerm.nodes
  | .pair left right => do
    let l ← expandedNodes bindings left
    let r ← expandedNodes bindings right
    return 1 + l + r
  | _ => some 1

private def substituteSlots (bindings : List (Nat × FieldTerm)) : FieldTerm → Option FieldTerm
  | .root id => some (.root id)
  | .project receiver field => some (.project receiver field)
  | .slot id => bindings.lookup id
  | .pair left right => do
    let l ← substituteSlots bindings left
    let r ← substituteSlots bindings right
    return .pair l r

/-- Simultaneous substitution. Replacements have their caller's slot scope.
    The result size is checked before any expanded tree is built. -/
def specialize (bindings : List (Nat × FieldTerm)) (source : FieldTerm) : Option FieldTerm :=
  if bindings.length ≤ 4096 ∧ source.nodes ≤ 4096 ∧
      (bindings.map (fun entry => entry.2.nodes)).sum ≤ 16384 ∧ (bindings.map Prod.fst).Nodup then do
    let size ← expandedNodes bindings source
    if size ≤ 4096 then substituteSlots bindings source else none
  else none

/-- Declarative graph of simultaneous substitution. Each use of a slot must
    resolve to the actual replacement at that identity. -/
inductive Substitutes (bindings : List (Nat × FieldTerm)) : FieldTerm → FieldTerm → Prop where
  | root (id : Nat) : Substitutes bindings (.root id) (.root id)
  | project (receiver field : Nat) : Substitutes bindings (.project receiver field) (.project receiver field)
  | slot {id replacement} (found : bindings.lookup id = some replacement) :
      Substitutes bindings (.slot id) replacement
  | pair {a b x y} : Substitutes bindings a x → Substitutes bindings b y →
      Substitutes bindings (.pair a b) (.pair x y)

private theorem substituteSlots_sound {bindings : List (Nat × FieldTerm)} {source result : FieldTerm}
    (h : substituteSlots bindings source = some result) : Substitutes bindings source result := by
  induction source generalizing result with
  | root id => cases h; exact .root id
  | project receiver field => cases h; exact .project receiver field
  | slot id => exact .slot h
  | pair left right il ir =>
    cases hl : substituteSlots bindings left with
    | none => simp [substituteSlots, hl] at h
    | some l =>
      cases hr : substituteSlots bindings right with
      | none => simp [substituteSlots, hl, hr] at h
      | some r =>
        simp [substituteSlots, hl, hr] at h
        subst result
        exact .pair (il hl) (ir hr)

private theorem Substitutes.size_exact {bindings : List (Nat × FieldTerm)} {source result : FieldTerm}
    (h : Substitutes bindings source result) : expandedNodes bindings source = some result.nodes := by
  induction h with
  | root | project => rfl
  | slot found => simp [expandedNodes, found]
  | pair _ _ il ir => simp [expandedNodes, FieldTerm.nodes, il, ir]

theorem specialize_sound {bindings : List (Nat × FieldTerm)} {source result : FieldTerm}
    (h : specialize bindings source = some result) :
    Substitutes bindings source result ∧ result.nodes ≤ 4096 := by
  unfold specialize at h
  split at h
  next =>
    obtain ⟨size, computed, success⟩ := Option.bind_eq_some_iff.mp h
    split at success
    next within =>
      have graph := substituteSlots_sound success
      have exactSize : size = result.nodes := Option.some.inj (computed.symm.trans graph.size_exact)
      exact ⟨graph, exactSize ▸ within⟩
    next => contradiction
  next => contradiction

/-- Interpret source slots in the field's scope and replacement slots in the
    caller's scope. Roots, receiver projections and tree structure share the
    same interpretation on both sides. -/
def FieldTerm.denote {α : Type u} (root slot : Nat → α) (project : Nat → Nat → α)
    (pair : α → α → α) : FieldTerm → α
  | .root id => root id
  | .slot id => slot id
  | .project receiver field => project receiver field
  | .pair left right => pair (left.denote root slot project pair) (right.denote root slot project pair)

theorem specialize_denotes {bindings : List (Nat × FieldTerm)} {source result : FieldTerm}
    (h : specialize bindings source = some result) {α : Type u} (root before after : Nat → α)
    (project : Nat → Nat → α) (pair : α → α → α)
    (agrees : ∀ id term, bindings.lookup id = some term → before id = term.denote root after project pair) :
    source.denote root before project pair = result.denote root after project pair := by
  have graph := (specialize_sound h).1
  clear h
  induction graph with
  | root | project => rfl
  | slot found => exact agrees _ _ found
  | pair _ _ il ir => simp [FieldTerm.denote, il, ir]

example : form [[], [0], [0, 1]] = true := by decide
example : form [[], [1]] = false := by decide
example : form [[], [], [1, 0]] = false := by decide
example : form [[], [0, 0]] = false := by decide
example : construction [[], [0], [0, 1]] 2 [some 7, some 8] = some [(0, 7), (1, 8)] := by decide
example : construction [[], [0], [0, 1]] 2 [none, some 8] = none := by decide
example : projection [[], [0], [0, 1]] 2 42 = some [(0, (42, 0)), (1, (42, 1))] := by decide
example : mayMutate [[], [0], [0, 1]] 1 = some false := by decide
example : mayMutate [[], [0], [0, 1]] 2 = some true := by decide
example : specialize [(0, .project 42 0), (1, .project 42 1)]
    (.pair (.root 11) (.pair (.slot 0) (.pair (.slot 1) (.slot 1)))) =
    some (.pair (.root 11) (.pair (.project 42 0) (.pair (.project 42 1) (.project 42 1)))) := by decide
example : specialize [(0, .root 9)] (.slot 1) = none := by decide
example : specialize [(0, .root 9), (0, .root 10)] (.slot 0) = none := by decide

end Verum.RecordTelescope
