import VerumExternalReplay.AcquiredObserver
import VerumExternalReplay.PhysicalTransducers

/-! Source-owned retained calibration, with explicit bounded arithmetic and
prepared-token ownership. Native grouped support and model lists agree by
membership, never by storage order. Allocation, layer projection, the observer
agenda and evidence admission are separate correspondence obligations. -/
namespace SourceCalibration
open AcquiredObserver
set_option warningAsError true

structure Sample where
  lineage : Lineage
  ordinal : Nat
  deriving DecidableEq

structure Origin where
  lineage : Lineage
  epoch : Nat
  ordinal : Nat
  deriving DecidableEq

structure Context where
  epoch : Nat
  input : Nat
  action : Nat
  deriving DecidableEq

abbrev Support := List (Context × Value)

structure Profile where
  identity : Nat
  source : Option Source
  origin : Option Origin
  revision : Nat
  lastNovel : Option Sample
  ending : Option Sample
  incomplete : Option Sample
  maximum : Nat
  cells : Nat
  support : Support
  deriving DecidableEq

inductive Refusal where
  | capacity | foreignSource | foreignContext | foreignOutcome | beforeOrigin
  | duplicate | outOfOrder | stalePreparation | arithmetic | incomplete
  deriving DecidableEq

structure Prepared where
  predecessor : Nat
  priorEnd : Option Sample
  source : Source
  origin : Origin
  sample : Sample
  context : Context
  value : Value
  cells : Nat
  revision : Nat
  deriving DecidableEq

deriving instance DecidableEq for Except

def member (support : Support) (context : Context) (value : Value) : Bool :=
  support.any (fun row => decide (row = (context, value)))

def newContext (support : Support) (context : Context) : Bool :=
  !support.any (fun row => decide (row.1 = context))

def prepare (limit capacity : Nat) (s : Profile) (id : Sample)
    (context : Context) (value : Value) : Except Refusal Prepared := do
  if s.incomplete.isSome then throw .incomplete
  let some source := s.source | throw .foreignSource
  let some origin := s.origin | throw .foreignSource
  if id.lineage ≠ origin.lineage ∨ source.owner ≠ some id.lineage then throw .foreignSource
  match value with
  | .outcome _ => pure ()
  | _ => throw .foreignOutcome
  if context.epoch ≠ source.epoch then throw .foreignContext
  if id.ordinal < origin.ordinal then throw .beforeOrigin
  if let some prior := s.ending then
    if id = prior then throw .duplicate
    if id.ordinal < prior.ordinal then throw .outOfOrder
  let novel := !member s.support context value
  let cells := s.cells + if novel then 3 + if newContext s.support context then 1 else 0 else 0
  if cells > limit then throw .arithmetic
  if cells > capacity then throw .capacity
  let revision := s.revision + if novel then 1 else 0
  if revision > limit then throw .arithmetic
  return ⟨s.identity, s.ending, source, origin, id, context, value, cells, revision⟩

def commit (fresh : Nat) (s : Profile) (p : Prepared) : Except Refusal Profile := do
  if s.incomplete.isSome then throw .incomplete
  if s.identity ≠ p.predecessor ∨ s.ending ≠ p.priorEnd ∨
      s.source ≠ some p.source ∨ s.origin ≠ some p.origin then throw .stalePreparation
  if member s.support p.context p.value then return { s with ending := some p.sample }
  let support := s.support ++ [(p.context, p.value)]
  let size := (support.filter (fun row => decide (row.1 = p.context))).length
  return { s with
    identity := fresh
    revision := p.revision
    lastNovel := some p.sample
    ending := some p.sample
    cells := p.cells
    maximum := max s.maximum size
    support := support }

def observe (limit capacity fresh : Nat) (s : Profile) (id : Sample)
    (context : Context) (value : Value) : Except Refusal Profile :=
  (prepare limit capacity s id context value).bind (commit fresh s)

def bind (s : Profile) (source : Source) (origin : Origin) : Except Refusal Profile := do
  if source.epoch ≠ origin.epoch ∨ source.owner ≠ some origin.lineage ∨
      source.shape ≠ .event then throw .foreignSource
  if s.source.isSome ∧ s.source ≠ some source then throw .foreignSource
  if s.origin.isSome ∧ s.origin ≠ some origin then throw .foreignSource
  return { s with source := some source, origin := some origin }

/-- Finite executable representation relation; metadata and nominal owners stay exact. -/
def agrees (a b : Profile) : Bool :=
  decide ({ a with support := b.support } = b) &&
  a.support.all (fun row => member b.support row.1 row.2) &&
  b.support.all (fun row => member a.support row.1 row.2)

def resultAgrees (result : Except Refusal Profile) (after : Profile) : Bool :=
  match result with
  | .ok expected => agrees expected after
  | .error _ => false

/-- A representation relation is preserved under insertion independently of
sorting, indexing or linked storage. Duplicate entries are rejected by decoding. -/
def SameMembers {A : Type} (a b : List A) : Prop := ∀ x, x ∈ a ↔ x ∈ b

theorem append_preserves_representation {A : Type} (a b : List A) (x : A)
    (h : SameMembers a b) : SameMembers (a ++ [x]) (b ++ [x]) := by
  intro y
  simp only [List.mem_append, List.mem_cons, List.not_mem_nil, or_false]
  exact or_congr (h y) Iff.rfl

theorem same_representation_same_novelty {A : Type} (a b : List A) (x : A)
    (h : SameMembers a b) : (x ∉ a) ↔ (x ∉ b) := not_congr (h x)

end SourceCalibration
