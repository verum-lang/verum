import VerumExternalReplay.AcquiredObserver

/-! Consumer-specific projection of immutable signatures. Publication is separate
from selecting compatible initial/update/read calls. This is a mathematical
catalogue model, not a proof of Rust allocation, VM evaluation or admission truth.
The finite cursor model accounts for logical work and retention; allocation
failures require their own observed transition and are not treated as capacity.
-/
namespace SignatureCatalogues
open AcquiredObserver FiniteCode
set_option warningAsError true

deriving instance DecidableEq for InputSource
deriving instance DecidableEq for Except

structure Triple (A : Type) where
  initial : A
  update : A
  read : A
  deriving DecidableEq

def Triple.get (xs : Triple A) (phase : Nat) : A :=
  if phase = 0 then xs.initial else if phase = 1 then xs.update else xs.read

def Triple.map (f : A → B) (xs : Triple A) : Triple B :=
  ⟨f xs.initial, f xs.update, f xs.read⟩

structure Entry where
  identity : Nat
  version : Nat
  parent : Option Nat
  bodies : Triple Nat
  updateSource : Source
  readSource : Source
  maxTrace : Nat
  updateWidth : Nat
  readWidth : Nat
  updateSlots : List InputSource
  readSlots : List InputSource
  deriving DecidableEq

structure Config where
  source : Source
  maxTrace : Nat
  predictive : Bool
  deriving DecidableEq

inductive Exclusion where
  | source | semantics | ports
  deriving DecidableEq

structure Selection where
  version : Nat
  included : Triple Bool
  exclusion : Option Exclusion
  deriving DecidableEq

def select (config : Config) (entry : Entry) : Selection :=
  let query := { config.source with shape := .query }
  if entry.updateSource ≠ config.source ∨ entry.readSource ≠ query then
    ⟨entry.version, ⟨false, false, false⟩, some .source⟩
  else if entry.maxTrace ≠ config.maxTrace then
    ⟨entry.version, ⟨false, false, false⟩, some .semantics⟩
  else
    let width := if config.predictive then 2 else 1
    let slots := if config.predictive then [.state, .external 0, .external 1]
      else [.state, .external 0]
    let update := decide (entry.updateWidth = width ∧ entry.updateSlots = slots)
    let read := decide (entry.readWidth = width ∧ entry.readSlots = slots)
    ⟨entry.version, ⟨true, update, read⟩, if update && read then none else some .ports⟩

structure Call where
  arity : Nat
  body : Nat
  deriving DecidableEq

def call (entry : Entry) (phase : Nat) : Call :=
  ⟨if phase = 0 then 0 else if phase = 1 then entry.updateSlots.length
    else entry.readSlots.length, entry.bodies.get phase⟩

/-- Retain the declaration and body owner while assigning a consumer index. -/
def project (config : Config) (phase : Nat) (entries : List Entry) : List (Nat × Call) :=
  entries.filterMap fun entry =>
    if (select config entry).included.get phase then some (entry.identity, call entry phase)
    else none

theorem project_append (config : Config) (phase : Nat) (new old : List Entry) :
    project config phase (new ++ old) = project config phase new ++ project config phase old := by
  simp [project, List.filterMap_append]

theorem excluded_entry (config : Config) (phase : Nat) (entry : Entry) (rest : List Entry)
    (excluded : (select config entry).included.get phase = false) :
    project config phase (entry :: rest) = project config phase rest := by
  simp [project, excluded]

theorem included_entry (config : Config) (phase : Nat) (entry : Entry) (rest : List Entry)
    (included : (select config entry).included.get phase = true) :
    project config phase (entry :: rest) =
      (entry.identity, call entry phase) :: project config phase rest := by
  simp [project, included]

theorem compatible_initial (config : Config) (entry : Entry)
    (update : entry.updateSource = config.source)
    (read : entry.readSource = { config.source with shape := .query })
    (semantics : entry.maxTrace = config.maxTrace) :
    (select config entry).included.initial = true := by
  simp [select, update, read, semantics]

structure Pending where
  signature : Nat
  part : Nat
  included : Triple Bool
  bodies : Triple Nat
  deriving DecidableEq

structure Library where
  next : Option Nat
  pending : Option Pending
  calls : Triple (List Call)
  frozen : Option (Triple (List Call))
  cells : Nat
  taken : Bool
  selections : List Selection
  deriving DecidableEq

def Library.new (root : Nat) : Library :=
  ⟨some root, none, ⟨[], [], []⟩, none, 1, false, []⟩

def appendCall (calls : Triple (List Call)) (phase : Nat) (value : Call) : Triple (List Call) :=
  if phase = 0 then { calls with initial := calls.initial ++ [value] }
  else if phase = 1 then { calls with update := calls.update ++ [value] }
  else { calls with read := calls.read ++ [value] }

inductive Error where
  | taken | retention | callRetention | selectionRetention | frozenRetention
  | missingSignature | invalidCursor
  deriving DecidableEq

def fits (used extra capacity : Nat) : Bool :=
  decide (used + extra ≤ capacity ∧ used + extra < 2^64)

/-- A step is paid before inspecting capacity. The root table supplies nominal
owners; missing identities never become an empty or successful catalogue. -/
def step (entries : List Entry) (config : Config) (capacity : Nat)
    (state : Library) : Except Error (Library × Bool) := do
  if state.cells > capacity then throw .retention
  if let some pending := state.pending then
    let entry ← match entries.find? (fun entry => entry.identity == pending.signature) with
      | some entry => pure entry
      | none => throw .missingSignature
    if pending.part = 3 then
      return ({ state with next := entry.parent, pending := none }, false)
    if pending.part > 3 then throw .invalidCursor
    let state ← if pending.included.get pending.part then do
      if !fits state.cells 2 capacity then throw .callRetention
      pure { state with calls := appendCall state.calls pending.part (call entry pending.part),
                        cells := state.cells + 2 }
      else pure state
    return ({ state with pending := some { pending with part := pending.part + 1 } }, false)
  else if let some next := state.next then
    if !fits state.cells 5 capacity then throw .selectionRetention
    let entry ← match entries.find? (fun entry => entry.identity == next) with
      | some entry => pure entry
      | none => throw .missingSignature
    let selection := select config entry
    return ({ state with
      cells := state.cells + 5
      selections := state.selections ++ [selection]
      pending := some ⟨entry.identity, 0, selection.included, entry.bodies⟩ }, false)
  else if state.frozen.isSome then return (state, true)
  else
    if !fits state.cells 3 capacity then throw .frozenRetention
    return ({ state with frozen := some state.calls, calls := ⟨[], [], []⟩,
                         cells := state.cells + 3 }, true)

/-- Result includes unused work and the exact retained owner after refusal. -/
def advance (entries : List Entry) (config : Config) (capacity : Nat) :
    Nat → Library → Library × Nat × Except Error Bool
  | work, state =>
    if state.taken then (state, work, .error .taken)
    else match work with
    | 0 => (state, 0, .ok state.frozen.isSome)
    | remaining + 1 => match step entries config capacity state with
      | .error error => (state, remaining, .error error)
      | .ok (next, true) => (next, remaining, .ok true)
      | .ok (next, false) => advance entries config capacity remaining next
termination_by work => work

theorem zero_work_retains_state (entries : List Entry) (config : Config) (capacity : Nat)
    (state : Library) : (advance entries config capacity 0 state).1 = state := by
  simp [advance]; split <;> rfl

theorem consumed_retains_state (entries : List Entry) (config : Config) (capacity work : Nat)
    (state : Library) (consumed : state.taken = true) :
    advance entries config capacity work state = (state, work, .error .taken) := by
  cases work <;> simp [advance, consumed]

end SignatureCatalogues
