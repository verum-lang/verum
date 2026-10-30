import VerumExternalReplay.CategoricalScore
import VerumExternalReplay.SharedUpdates

/-! Representation-independent exact count banks. A bank is its lookup law,
not a Rust map, serialization or runtime. Labels retain their identity at joins.
Successful intake simulation preserves counts; it does not prove admission,
failure precedence, allocation, work, source provenance or publication authority.
-/
namespace CountBank
set_option warningAsError true

variable {Key Row : Type} [DecidableEq Key]

abbrev Bank (Key : Type) := Key → Nat

def empty : Bank Key := fun _ => 0
def merge (left right : Bank Key) : Bank Key := fun key => left key + right key
def record (bank : Bank Key) (key : Key) : Bank Key :=
  fun query => bank query + if key = query then 1 else 0

def ingest (classify : Row → Key) (bank : Bank Key) (rows : List Row) : Bank Key :=
  rows.foldl (fun bank row => record bank (classify row)) bank

def count (classify : Row → Key) (rows : List Row) (key : Key) : Nat :=
  (rows.filter (fun row => decide (classify row = key))).length

theorem count_append (classify : Row → Key) (xs ys : List Row) (key : Key) :
    count classify (xs ++ ys) key = count classify xs key + count classify ys key := by
  simp [count]

theorem ingest_lookup (classify : Row → Key) (bank : Bank Key)
    (rows : List Row) (key : Key) :
    ingest classify bank rows key = bank key + count classify rows key := by
  induction rows generalizing bank with
  | nil => simp [ingest, count]
  | cons row rows ih =>
      simp only [ingest, List.foldl_cons] at *
      rw [ih]
      by_cases same : classify row = key <;>
        simp [record, count, same, Nat.add_assoc, Nat.add_comm]

theorem ingest_append (classify : Row → Key) (bank : Bank Key) (xs ys : List Row) :
    ingest classify bank (xs ++ ys) = ingest classify (ingest classify bank xs) ys := by
  simp [ingest, List.foldl_append]

theorem collected_merge (classify : Row → Key) (xs ys : List Row) :
    ingest classify empty (xs ++ ys) =
      merge (ingest classify empty xs) (ingest classify empty ys) := by
  funext key
  simp [merge, ingest_lookup, count_append, empty]

omit [DecidableEq Key] in
theorem sample_count (rows : List Row) :
    ingest (fun _ : Row => ()) empty rows () = rows.length := by
  simp [ingest_lookup, empty, count]

omit [DecidableEq Key] in
theorem merge_associative (a b c : Bank Key) : merge (merge a b) c = merge a (merge b c) := by
  funext key; simp [merge, Nat.add_assoc]

omit [DecidableEq Key] in
theorem merge_commutative (a b : Bank Key) : merge a b = merge b a := by
  funext key; simp [merge, Nat.add_comm]

omit [DecidableEq Key] in
theorem empty_identity (bank : Bank Key) : merge empty bank = bank ∧ merge bank empty = bank := by
  constructor <;> funext key <;> simp [merge, empty]

omit [DecidableEq Key] in
theorem support_union (left right : Bank Key) (key : Key) :
    0 < merge left right key ↔ 0 < left key ∨ 0 < right key := by
  unfold merge
  cases left key <;> simp [Nat.succ_add]

/-- A storage adapter supplies this law for each successful actual row commit.
It cannot substitute successful mathematical rows for a native refusal. -/
def StepCorrespondence (update : State → Row → Except Error State)
    (lookup : State → Bank Key) (classify : Row → Key) : Prop :=
  ∀ state row next, update state row = .ok next →
    lookup next = record (lookup state) (classify row)

theorem accepted_correspondence (update : State → Row → Except Error State)
    (lookup : State → Bank Key) (classify : Row → Key)
    (step : StepCorrespondence update lookup classify)
    (rows : List Row) (initial final : State) (position : Nat)
    (accepted : SharedUpdates.runFrom update position rows initial = .complete final) :
    lookup final = ingest classify (lookup initial) rows := by
  induction rows generalizing initial position with
  | nil => cases accepted; rfl
  | cons row rows ih =>
      simp only [SharedUpdates.runFrom] at accepted
      cases transition : update initial row with
      | error error => simp [transition] at accepted
      | ok next =>
          simp only [transition] at accepted
          rw [ih next (position + 1) accepted, step initial row next transition]
          rfl

-- The following adapter joins the generic count laws to the existing exact
-- categorical arithmetic. Cells are named within one declared context; a
-- multi-context adapter must preserve its context→alphabet association.
open CategoricalScore in
def rowParent (rows : List CategoricalScore.Row) : Bank Nat :=
  ingest CategoricalScore.Row.label empty rows

def rowChild (rows : List CategoricalScore.Row) : Bank (Nat × Nat) :=
  ingest (fun row => (row.reading, row.label)) empty rows

def vector (size : Nat) (bank : Bank Nat) : List Nat :=
  (List.range size).map bank

def score (alphabet samples : Nat) (cells : List Nat)
    (parent : Bank Nat) (child : Bank (Nat × Nat)) : CategoricalScore.Score :=
  CategoricalScore.fromCounts alphabet (vector alphabet parent)
    (cells.map (fun cell => vector alphabet (fun label => child (cell, label)))) samples

theorem ratio_product_permutation (xs ys : List CategoricalScore.Ratio) (order : xs.Perm ys) :
    xs.foldr CategoricalScore.mul CategoricalScore.one =
      ys.foldr CategoricalScore.mul CategoricalScore.one := by
  induction order with
  | nil => rfl
  | cons _ _ ih => simp only [List.foldr_cons, ih]
  | swap a b rest =>
      simp only [List.foldr_cons, CategoricalScore.mul]
      congr 1 <;> ac_rfl
  | trans _ _ ih₁ ih₂ => exact ih₁.trans ih₂

theorem score_cell_order (alphabet samples : Nat) (xs ys : List Nat)
    (parent : Bank Nat) (child : Bank (Nat × Nat)) (order : xs.Perm ys) :
    score alphabet samples xs parent child = score alphabet samples ys parent child := by
  unfold score CategoricalScore.fromCounts
  rw [ratio_product_permutation _ _ ((order.map _).map _)]

theorem cell_census_append (xs ys : List CategoricalScore.Row) (cell : Nat) :
    cell ∈ (((xs ++ ys).map CategoricalScore.Row.reading).eraseDups) ↔
      cell ∈ ((xs.map CategoricalScore.Row.reading).eraseDups) ∨
      cell ∈ ((ys.map CategoricalScore.Row.reading).eraseDups) := by simp

theorem parent_vector (alphabet : Nat) (rows : List CategoricalScore.Row) :
    vector alphabet (rowParent rows) = CategoricalScore.counts alphabet rows := by
  simp only [vector, rowParent, CategoricalScore.counts]
  congr 1
  funext label
  simp only [ingest_lookup, count, empty, Nat.zero_add]
  congr 1

theorem child_vector (alphabet cell : Nat) (rows : List CategoricalScore.Row) :
    vector alphabet (fun label => rowChild rows (cell, label)) =
      CategoricalScore.counts alphabet (rows.filter (fun row => row.reading == cell)) := by
  simp only [vector, rowChild, ingest_lookup, empty, Nat.zero_add, count,
    CategoricalScore.counts, List.filter_filter]
  congr 1
  funext label
  congr 1
  apply List.filter_congr
  intro row _
  apply Bool.eq_iff_iff.mpr
  simp [Prod.mk.injEq, and_comm]

theorem score_correspondence (alphabet : Nat) (rows : List CategoricalScore.Row) :
    score alphabet rows.length ((rows.map CategoricalScore.Row.reading).eraseDups)
      (rowParent rows) (rowChild rows) =
      CategoricalScore.contextScore ⟨alphabet, rows⟩ := by
  simp [score, parent_vector, child_vector, CategoricalScore.contextScore]

theorem census_score_correspondence (alphabet : Nat) (rows : List CategoricalScore.Row)
    (cells : List Nat) (census : cells.Perm ((rows.map CategoricalScore.Row.reading).eraseDups)) :
    score alphabet rows.length cells (rowParent rows) (rowChild rows) =
      CategoricalScore.contextScore ⟨alphabet, rows⟩ := by
  rw [score_cell_order _ _ _ _ _ _ census, score_correspondence]

/-- Append the banks, then score the union under the same frozen declaration.
Independent batch scores cannot be multiplied to obtain this result. The
exact cell census is a premise, not inferred from sample count or a ratio. -/
theorem append_score_correspondence (alphabet : Nat) (xs ys : List CategoricalScore.Row) :
    score alphabet (xs.length + ys.length)
      (((xs ++ ys).map CategoricalScore.Row.reading).eraseDups)
      (merge (rowParent xs) (rowParent ys))
      (merge (rowChild xs) (rowChild ys)) =
      CategoricalScore.contextScore ⟨alphabet, xs ++ ys⟩ := by
  rw [← score_correspondence]
  simp only [List.length_append, rowParent, rowChild, collected_merge]

end CountBank
