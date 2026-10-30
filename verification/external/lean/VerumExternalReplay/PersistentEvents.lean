import Lean
import VerumExternalReplay.SharedUpdates

/-! Ordered immutable chunk forests. Internal cached lengths count events, not
chunks. Native leaves retain their original event slots; the adapter must bind
those identities to `E`. These laws cover lookup and completed carry semantics,
not source authenticity, allocation, balance/space bounds or a work scheduler.
They do not bypass the existing paid prefix-equality verifier. -/
namespace PersistentEvents
set_option warningAsError true
variable {E S F : Type}

inductive Tree (E : Type) where
  | leaf (events : List E)
  | branch (length : Nat) (left right : Tree E)
  deriving DecidableEq

def Tree.events : Tree E → List E
  | .leaf xs => xs
  | .branch _ left right => left.events ++ right.events

def Tree.length : Tree E → Nat
  | .leaf xs => xs.length
  | .branch length _ _ => length

def Tree.Valid : Tree E → Prop
  | .leaf _ => True
  | .branch length left right =>
      left.Valid ∧ right.Valid ∧ length = left.length + right.length

def Tree.decValid (tree : Tree E) : Decidable tree.Valid :=
  match tree with
  | .leaf _ => isTrue trivial
  | .branch n l r =>
    have : Decidable l.Valid := l.decValid
    have : Decidable r.Valid := r.decValid
    inferInstanceAs (Decidable (l.Valid ∧ r.Valid ∧ n = l.length + r.length))

instance (tree : Tree E) : Decidable tree.Valid := tree.decValid

def Tree.get : Tree E → Nat → Option E
  | .leaf xs, i => xs[i]?
  | .branch _ left right, i =>
      if i < left.length then left.get i else right.get (i - left.length)

def join (left right : Tree E) : Tree E := .branch (left.length + right.length) left right

theorem join_valid (left right : Tree E) (hl : left.Valid) (hr : right.Valid) :
    (join left right).Valid := ⟨hl, hr, rfl⟩

theorem tree_length (tree : Tree E) (valid : tree.Valid) : tree.length = tree.events.length := by
  induction tree with
  | leaf _ => rfl
  | branch _ left right il ir =>
    rcases valid with ⟨hl, hr, size⟩
    simp only [Tree.length, Tree.events, List.length_append, ← il hl, ← ir hr]
    exact size

theorem tree_lookup (tree : Tree E) (valid : tree.Valid) (index : Nat) :
    tree.get index = tree.events[index]? := by
  induction tree generalizing index with
  | leaf _ => rfl
  | branch _ left right il ir =>
    rcases valid with ⟨hl, hr, _⟩
    simp only [Tree.get, Tree.events, tree_length left hl, List.getElem?_append]
    split
    · exact il hl index
    · simpa only [← tree_length left hl] using ir hr (index - left.length)

def flatten (roots : List (Tree E)) : List E := roots.flatMap Tree.events
def length (roots : List (Tree E)) : Nat := (roots.map Tree.length).sum
def Valid (roots : List (Tree E)) : Prop := ∀ t ∈ roots, t.Valid

instance (roots : List (Tree E)) : Decidable (Valid roots) :=
  inferInstanceAs (Decidable (∀ t ∈ roots, t.Valid))

def get : List (Tree E) → Nat → Option E
  | [], _ => none
  | tree :: rest, i => if i < tree.length then tree.get i else get rest (i - tree.length)

theorem forest_length (roots : List (Tree E)) (valid : Valid roots) :
    length roots = (flatten roots).length := by
  induction roots with
  | nil => rfl
  | cons tree rest ih =>
    have ht := valid tree List.mem_cons_self
    have hr : Valid rest := fun t h => valid t (List.mem_cons_of_mem _ h)
    simpa only [length, flatten, List.map_cons, List.sum_cons, List.flatMap_cons,
      List.length_append, tree_length tree ht] using congrArg (tree.events.length + ·) (ih hr)

theorem forest_lookup (roots : List (Tree E)) (valid : Valid roots) (index : Nat) :
    get roots index = (flatten roots)[index]? := by
  induction roots generalizing index with
  | nil => rfl
  | cons tree rest ih =>
    have ht := valid tree List.mem_cons_self
    have hr : Valid rest := fun t h => valid t (List.mem_cons_of_mem _ h)
    simp only [get, flatten, List.flatMap_cons, tree_length tree ht, List.getElem?_append]
    split
    · exact tree_lookup tree ht index
    · simpa only [← tree_length tree ht, flatten] using ih hr (index - tree.length)

/-- The native outer length is an independent cache and must also be checked. -/
structure Snapshot (E : Type) where
  roots : List (Tree E)
  length : Nat
  deriving DecidableEq

def Snapshot.Valid (s : Snapshot E) : Prop :=
  PersistentEvents.Valid s.roots ∧ s.length = PersistentEvents.length s.roots

instance (s : Snapshot E) : Decidable s.Valid :=
  inferInstanceAs (Decidable (PersistentEvents.Valid s.roots ∧ s.length = length s.roots))

def Snapshot.get (s : Snapshot E) (index : Nat) : Option E :=
  if index < s.length then PersistentEvents.get s.roots index else none

theorem snapshot_lookup (s : Snapshot E) (valid : s.Valid) (index : Nat) :
    s.get index = (flatten s.roots)[index]? := by
  unfold Snapshot.get
  split
  · exact forest_lookup s.roots valid.1 index
  · symm
    apply List.getElem?_eq_none
    have := forest_length s.roots valid.1
    have := valid.2
    omega

theorem snapshot_end (s : Snapshot E) (valid : s.Valid) : s.get s.length = none := by
  rw [snapshot_lookup s valid, List.getElem?_eq_none]
  rw [← forest_length s.roots valid.1, valid.2]
  exact Nat.le_refl _

theorem snapshot_same_events (left right : Snapshot E)
    (hl : left.Valid) (hr : right.Valid) (same : flatten left.roots = flatten right.roots)
    (index : Nat) : left.get index = right.get index := by
  rw [snapshot_lookup left hl, snapshot_lookup right hr, same]

theorem copy_partition (roots : List (Tree E)) (copied : Nat) :
    flatten (roots.take copied) ++ flatten (roots.drop copied) = flatten roots := by
  unfold flatten
  rw [← List.flatMap_append, List.take_append_drop]

theorem flatten_append (left right : List (Tree E)) :
    flatten (left ++ right) = flatten left ++ flatten right := List.flatMap_append

/-- Native carries merge equal-rank roots. Content preservation is stronger:
    every ordered merge preserves events, independently of that balance guard. -/
structure Carry (E : Type) where
  roots : List (Tree E)
  pending : Option (Tree E)
  deriving DecidableEq

def Carry.events (s : Carry E) : List E :=
  flatten s.roots ++ (match s.pending with | none => [] | some t => t.events)

def Carry.Valid (s : Carry E) : Prop :=
  PersistentEvents.Valid s.roots ∧ (match s.pending with | none => True | some t => t.Valid)

inductive Step : Carry E → Carry E → Prop where
  | merge (roots : List (Tree E)) (left right : Tree E) :
      Step ⟨roots ++ [left], some right⟩ ⟨roots, some (join left right)⟩
  | insert (roots : List (Tree E)) (tree : Tree E) :
      Step ⟨roots, some tree⟩ ⟨roots ++ [tree], none⟩

theorem step_events (step : Step before after) : after.events = before.events := by
  cases step <;> simp [Carry.events, flatten, join, Tree.events, List.append_assoc]

theorem step_valid (step : Step before after) (valid : before.Valid) : after.Valid := by
  cases step with
  | merge roots left right =>
    have hl := valid.1 left (by simp)
    exact ⟨fun t h => valid.1 t (List.mem_append_left _ h), join_valid left right hl valid.2⟩
  | insert roots tree =>
    refine ⟨?_, trivial⟩
    intro t ht
    rcases List.mem_append.mp ht with old | new
    · exact valid.1 t old
    · simp only [List.mem_singleton] at new
      subst t
      exact valid.2

inductive Trace : Carry E → Carry E → Prop where
  | refl (state : Carry E) : Trace state state
  | step {first before after} (history : Trace first before) (next : Step before after) :
      Trace first after

theorem trace_events (history : Trace first last) : last.events = first.events := by
  induction history with
  | refl => rfl
  | step _ next ih => exact (step_events next).trans ih

theorem trace_valid (history : Trace first last) (valid : first.Valid) : last.Valid := by
  induction history with
  | refl => exact valid
  | step _ next ih => exact step_valid next ih

/-- Empty suffixes allocate no chunk. Copying predecessor roots is a separate
    native phase; `copy_partition` describes the complete ordered copy. -/
def begin (roots : List (Tree E)) (suffix : List E) : Carry E :=
  ⟨roots, if suffix.isEmpty then none else some (.leaf suffix)⟩

theorem begin_events (roots : List (Tree E)) (suffix : List E) :
    (begin roots suffix).events = flatten roots ++ suffix := by
  cases suffix <;> simp [begin, Carry.events, Tree.events]

theorem begin_valid (roots : List (Tree E)) (suffix : List E) (valid : Valid roots) :
    (begin roots suffix).Valid := by
  cases suffix <;> exact ⟨valid, trivial⟩

theorem completed_append (before after : List (Tree E)) (suffix : List E)
    (history : Trace (begin before suffix) ⟨after, none⟩) :
    flatten after = flatten before ++ suffix := by
  have h := trace_events history
  rw [begin_events] at h
  simpa only [Carry.events, List.append_nil] using h

theorem completed_valid (before after : List (Tree E)) (suffix : List E)
    (valid : Valid before) (history : Trace (begin before suffix) ⟨after, none⟩) : Valid after :=
  (trace_valid history (begin_valid before suffix valid)).1

theorem completed_length (before after : List (Tree E)) (suffix : List E)
    (valid : Valid before) (history : Trace (begin before suffix) ⟨after, none⟩) :
    length after = length before + suffix.length := by
  rw [forest_length after (completed_valid before after suffix valid history),
    completed_append before after suffix history, List.length_append, forest_length before valid]

theorem completed_prefix (before after : List (Tree E)) (suffix : List E)
    (history : Trace (begin before suffix) ⟨after, none⟩) : flatten before <+: flatten after := by
  rw [completed_append before after suffix history]
  exact List.prefix_append _ _

theorem old_index_preserved (before after : List (Tree E)) (suffix : List E)
    (valid : Valid before) (history : Trace (begin before suffix) ⟨after, none⟩)
    (index : Nat) (old : index < length before) : get after index = get before index := by
  rw [forest_lookup after (completed_valid before after suffix valid history),
    forest_lookup before valid, completed_append before after suffix history]
  exact List.getElem?_append_left (forest_length before valid ▸ old)

theorem suffix_index (before after : List (Tree E)) (suffix : List E)
    (valid : Valid before) (history : Trace (begin before suffix) ⟨after, none⟩) (index : Nat) :
    get after (length before + index) = suffix[index]? := by
  rw [forest_lookup after (completed_valid before after suffix valid history),
    completed_append before after suffix history, forest_length before valid,
    List.getElem?_append_right (Nat.le_add_right _ _), Nat.add_sub_cancel_left]

theorem continued_execution (before after : List (Tree E)) (suffix : List E)
    (history : Trace (begin before suffix) ⟨after, none⟩)
    (update : S → E → Except F S) (position : Nat) (state : S) :
    SharedUpdates.runFrom update position (flatten after) state =
      SharedUpdates.continueFrom update (position + (flatten before).length) suffix
        (SharedUpdates.runFrom update position (flatten before) state) := by
  rw [completed_append before after suffix history]
  exact SharedUpdates.append update _ _ position state

end PersistentEvents
