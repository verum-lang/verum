/- Semantic projection of an append-only exact interner. Keys can be full
   (input, structured value) pairs; neither a hash nor a truncated encoding
   substitutes for equality. Names have meaning only within one retained shared
   namespace. Foreign comparison/clone fidelity, mutex serialization, work and
   allocation admission are separate implementation obligations. -/

namespace ExactNames

set_option warningAsError true

variable {K : Type} [DecidableEq K]

-- Existing entries remain available at capacity. New entries receive the next
-- index only after admission; a capacity refusal leaves the table unchanged.
def intern (capacity : Nat) (keys : List K) (key : K) : List K × Option Nat :=
  if key ∈ keys then (keys, some (keys.idxOf key))
  else if keys.length < capacity then (keys ++ [key], some keys.length)
  else (keys, none)

theorem existing (capacity : Nat) (keys : List K) (key : K) (present : key ∈ keys) :
    intern capacity keys key = (keys, some (keys.idxOf key)) := by
  simp [intern, present]

theorem capacity_preserves (capacity : Nat) (keys : List K) (key : K)
    (absent : key ∉ keys) (full : capacity ≤ keys.length) :
    intern capacity keys key = (keys, none) := by
  simp [intern, absent, Nat.not_lt_of_ge full]

theorem monotone (capacity : Nat) (keys : List K) (key : K) :
    ∃ suffix, (intern capacity keys key).1 = keys ++ suffix := by
  unfold intern
  split
  · exact ⟨[], by simp⟩
  · split
    · exact ⟨[key], rfl⟩
    · exact ⟨[], by simp⟩

theorem sound (capacity : Nat) (keys : List K) (key : K) (name : Nat)
    (accepted : (intern capacity keys key).2 = some name) :
    (intern capacity keys key).1[name]? = some key := by
  unfold intern at *
  split at *
  · rename_i present
    have nameEq : keys.idxOf key = name := by simpa using accepted
    subst name
    simp only [ite_eq_left present]
    rw [List.getElem?_eq_getElem (List.idxOf_lt_length_of_mem present)]
    exact congrArg some (List.getElem_idxOf (List.idxOf_lt_length_of_mem present))
  · split at *
    · simp_all
    · simp at accepted

theorem preserves_lookup (capacity : Nat) (keys : List K) (key old : K) (name : Nat)
    (retained : keys[name]? = some old) :
    (intern capacity keys key).1[name]? = some old := by
  obtain ⟨suffix, grown⟩ := monotone capacity keys key
  rw [grown, List.getElem?_append_left]
  · exact retained
  · exact (List.getElem?_eq_some_iff.mp retained).1

theorem preserves_unique (capacity : Nat) (keys : List K) (key : K)
    (unique : keys.Nodup) : (intern capacity keys key).1.Nodup := by
  unfold intern
  split
  · exact unique
  · rename_i absent
    split
    · simp [List.nodup_append, unique]
      intro a member
      intro same
      exact absent (same ▸ member)
    · exact unique

theorem fresh_index (keys : List K) (key : K) (absent : key ∉ keys) :
    (keys ++ [key]).idxOf key = keys.length := by
  induction keys with
  | nil => simp
  | cons head tail ih =>
    simp only [List.mem_cons, not_or] at absent
    simp only [List.cons_append, List.idxOf_cons, beq_iff_eq,
      ite_eq_right (Ne.symm absent.1), List.length_cons]
    rw [ih absent.2]

theorem repeated_name (capacity : Nat) (keys : List K) (key : K) (name : Nat)
    (accepted : (intern capacity keys key).2 = some name) :
    intern capacity (intern capacity keys key).1 key =
      ((intern capacity keys key).1, some name) := by
  unfold intern at *
  split at *
  · simp_all
  · rename_i absent
    split at *
    · simp_all [fresh_index]
    · simp at accepted

omit [DecidableEq K] in
theorem same_name_same_key (keys : List K) (name : Nat) (left right : K)
    (a : keys[name]? = some left) (b : keys[name]? = some right) : left = right := by
  exact Option.some.inj (a.symm.trans b)

-- Distinct full inputs or values cannot collapse to one index, including reads
-- performed by different forks sharing this same append-only namespace.
theorem sequential_injective (capacity : Nat) (keys : List K) (left right : K) (name : Nat)
    (a : (intern capacity keys left).2 = some name)
    (b : (intern capacity (intern capacity keys left).1 right).2 = some name) :
    left = right := by
  exact same_name_same_key _ name left right
    (preserves_lookup _ _ right left name (sound _ _ _ _ a)) (sound _ _ _ _ b)

inductive Refusal where
  | work | capacity | poisoned
  deriving DecidableEq

-- The native meter charges one unit before each comparison, then one before
-- admitting a new entry. A shortage consumes the available comparison units;
-- its failed charge does not mutate the table. Allocation failure is separate.
def metered (capacity : Nat) (keys : List K) (key : K) (remaining : Nat)
    (poisoned : Bool) : List K × Except Refusal Nat × Nat :=
  if poisoned then (keys, .error .poisoned, remaining) else
  let cost := if key ∈ keys then keys.idxOf key + 1 else keys.length + 1
  if remaining < cost then (keys, .error .work, 0) else
  let (after, name) := intern capacity keys key
  (after, match name with | some n => .ok n | none => .error .capacity, remaining - cost)

theorem metered_success_projects (capacity : Nat) (keys : List K) (key : K)
    (remaining name : Nat) (poisoned : Bool)
    (success : (metered capacity keys key remaining poisoned).2.1 = .ok name) :
    intern capacity keys key = ((metered capacity keys key remaining poisoned).1, some name) := by
  cases poisoned with
  | true => simp [metered] at success
  | false =>
    by_cases short : remaining < (if key ∈ keys then keys.idxOf key + 1 else keys.length + 1)
    · simp [metered, short] at success
    · cases result : intern capacity keys key with
      | mk after resultName =>
        cases resultName <;> simp_all [metered]
        simp [Nat.not_lt_of_ge short]

end ExactNames
