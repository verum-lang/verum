import VerumExternalReplay.CountBank

/-! Physical observation ownership for exact count summaries. An owner identifies
one immutable observation ledger and one compatible classifier/partition revision.
The native adapter must establish that identity and the input correspondence.
Intervals are half-open; validation compares endpoints, not materialized rows.
The summary join preserves its input interval order and grants no replay authority.
-/
namespace CountBankOwnership
set_option warningAsError true

structure Interval where
  start : Nat
  length : Nat
  deriving DecidableEq, Repr

def Interval.positions (span : Interval) : List Nat := List.range' span.start span.length

def Interval.contains (span : Interval) (position : Nat) : Prop :=
  span.start ≤ position ∧ position < span.start + span.length

def Interval.disjoint (left right : Interval) : Prop :=
  left.length = 0 ∨ right.length = 0 ∨
    left.start + left.length ≤ right.start ∨ right.start + right.length ≤ left.start

instance (left right : Interval) : Decidable (left.disjoint right) := by
  unfold Interval.disjoint; infer_instance

def positions (spans : List Interval) : List Nat := spans.flatMap Interval.positions

theorem interval_unique (span : Interval) : span.positions.Nodup := by
  rcases span with ⟨start, length⟩
  change (List.range' start length).Nodup
  induction length generalizing start with
  | zero => simp
  | succ length ih =>
      rw [List.range'_succ, List.nodup_cons]
      refine ⟨?_, ih (start + 1)⟩
      simp only [List.mem_range'_1]
      omega

theorem interval_disjoint (left right : Interval) (apart : left.disjoint right) :
    ∀ position ∈ left.positions, position ∉ right.positions := by
  intro position inLeft inRight
  simp only [Interval.positions, List.mem_range'_1] at inLeft inRight
  unfold Interval.disjoint at apart
  omega

theorem unique_positions (spans : List Interval) (valid : spans.Pairwise Interval.disjoint) :
    (positions spans).Nodup := by
  induction spans with
  | nil => simp [positions]
  | cons span spans ih =>
      rw [List.pairwise_cons] at valid
      simp only [positions, List.flatMap_cons, List.nodup_append]
      refine ⟨interval_unique span, ih valid.2, ?_⟩
      intro position inSpan otherPosition inRest same
      subst otherPosition
      obtain ⟨other, member, hit⟩ := List.mem_flatMap.mp inRest
      exact interval_disjoint span other (valid.1 other member) position inSpan hit

structure Summary (Owner Key : Type) where
  owner : Owner
  spans : List Interval
  bank : CountBank.Bank Key

inductive Refusal where
  | foreignOwner
  | overlap
  | invalidCut
  deriving DecidableEq, Repr

def splitChecked (span : Interval) (offset : Nat) : Except Refusal (Interval × Interval) :=
  if offset ≤ span.length then
    .ok (⟨span.start, offset⟩, ⟨span.start + offset, span.length - offset⟩)
  else .error .invalidCut

variable {Owner Key Row : Type} [DecidableEq Owner]

def joined (left right : Summary Owner Key) : Summary Owner Key :=
  ⟨left.owner, left.spans ++ right.spans, CountBank.merge left.bank right.bank⟩

def mergeChecked (left right : Summary Owner Key) : Except Refusal (Summary Owner Key) :=
  if left.owner ≠ right.owner then .error .foreignOwner
  else if (left.spans ++ right.spans).Pairwise Interval.disjoint then .ok (joined left right)
  else .error .overlap

def rows (ledger : Owner → Nat → Row) (owner : Owner) (spans : List Interval) : List Row :=
  (positions spans).map (ledger owner)

def Corresponds [DecidableEq Key] (ledger : Owner → Nat → Row) (classify : Owner → Row → Key)
    (summary : Summary Owner Key) : Prop :=
  summary.bank = CountBank.ingest (classify summary.owner) CountBank.empty
    (rows ledger summary.owner summary.spans)

def capture [DecidableEq Key] (ledger : Owner → Nat → Row) (classify : Owner → Row → Key)
    (owner : Owner) (spans : List Interval) : Summary Owner Key :=
  ⟨owner, spans, CountBank.ingest (classify owner) CountBank.empty (rows ledger owner spans)⟩

omit [DecidableEq Owner] in
theorem capture_correspondence [DecidableEq Key] (ledger : Owner → Nat → Row)
    (classify : Owner → Row → Key) (owner : Owner) (spans : List Interval) :
    Corresponds ledger classify (capture ledger classify owner spans) := rfl

theorem merge_acceptance (left right : Summary Owner Key) :
    (∃ result, mergeChecked left right = .ok result) ↔
      left.owner = right.owner ∧ (left.spans ++ right.spans).Pairwise Interval.disjoint := by
  by_cases owner : left.owner = right.owner <;>
    by_cases valid : (left.spans ++ right.spans).Pairwise Interval.disjoint <;>
      simp [mergeChecked, owner, valid]

theorem accepted_shape (left right result : Summary Owner Key)
    (accepted : mergeChecked left right = .ok result) : result = joined left right := by
  unfold mergeChecked at accepted
  split at accepted
  · contradiction
  · split at accepted
    · cases accepted; rfl
    · contradiction

theorem accepted_unique (left right result : Summary Owner Key)
    (accepted : mergeChecked left right = .ok result) : (positions result.spans).Nodup := by
  rw [accepted_shape left right result accepted]
  exact unique_positions _ ((merge_acceptance left right).mp ⟨result, accepted⟩).2

theorem accepted_correspondence [DecidableEq Key] (ledger : Owner → Nat → Row) (classify : Owner → Row → Key)
    (left right result : Summary Owner Key)
    (leftExact : Corresponds ledger classify left) (rightExact : Corresponds ledger classify right)
    (accepted : mergeChecked left right = .ok result) : Corresponds ledger classify result := by
  have owner := ((merge_acceptance left right).mp ⟨result, accepted⟩).1
  rw [accepted_shape left right result accepted]
  unfold Corresponds at *
  simp only [joined, leftExact, rightExact, rows, positions, List.flatMap_append,
    List.map_append, CountBank.collected_merge, owner]

theorem foreign_owner_refused (left right : Summary Owner Key) (foreign : left.owner ≠ right.owner) :
    mergeChecked left right = .error .foreignOwner := by simp [mergeChecked, foreign]

theorem overlapping_position_refused (left right : Summary Owner Key)
    (owner : left.owner = right.owner) (position : Nat)
    (inLeft : position ∈ positions left.spans) (inRight : position ∈ positions right.spans) :
    mergeChecked left right = .error .overlap := by
  have invalid : ¬ (left.spans ++ right.spans).Pairwise Interval.disjoint := by
    intro valid
    have unique := unique_positions _ valid
    simp only [positions, List.flatMap_append, List.nodup_append] at unique
    exact unique.2.2 position inLeft position inRight rfl
  simp [mergeChecked, owner, invalid]

/-- Both branches contain the same physical prefix. Even matching row values,
counts or scores cannot authorize their addition. -/
theorem shared_prefix_refused (left right : Summary Owner Key) (base : List Interval)
    (leftDelta rightDelta : List Interval) (owner : left.owner = right.owner)
    (leftShape : left.spans = base ++ leftDelta) (rightShape : right.spans = base ++ rightDelta)
    (position : Nat) (present : position ∈ positions base) :
    mergeChecked left right = .error .overlap := by
  apply overlapping_position_refused left right owner position
  · simp only [leftShape, positions, List.flatMap_append, List.mem_append]
    exact Or.inl present
  · simp only [rightShape, positions, List.flatMap_append, List.mem_append]
    exact Or.inl present

theorem adjacent_intervals (start leftLength rightLength : Nat) :
    (Interval.mk start leftLength).disjoint ⟨start + leftLength, rightLength⟩ := by
  simp [Interval.disjoint]

theorem split_positions (start length split : Nat) (within : split ≤ length) :
    positions [⟨start, split⟩, ⟨start + split, length - split⟩] =
      (Interval.mk start length).positions := by
  simp only [positions, List.flatMap_cons, List.flatMap_nil, List.append_nil, Interval.positions]
  simp [Nat.add_sub_cancel' within]

theorem adjacent_banks_accepted [DecidableEq Key] (ledger : Owner → Nat → Row)
    (classify : Owner → Row → Key) (owner : Owner) (start leftLength rightLength : Nat) :
    let left := capture ledger classify owner [⟨start, leftLength⟩]
    let right := capture ledger classify owner [⟨start + leftLength, rightLength⟩]
    mergeChecked left right = .ok (joined left right) := by
  simp [mergeChecked, capture, adjacent_intervals]

omit [DecidableEq Owner] in
/-- At any valid cut, a checked prefix/delta join has precisely the unsplit
count bank. The cut bound is essential: Nat subtraction must not hide overflow. -/
theorem split_bank [DecidableEq Key] (ledger : Owner → Nat → Row)
    (classify : Owner → Row → Key) (owner : Owner) (start length split : Nat)
    (within : split ≤ length) :
    (joined (capture ledger classify owner [⟨start, split⟩])
      (capture ledger classify owner [⟨start + split, length - split⟩])).bank =
      (capture ledger classify owner [⟨start, length⟩]).bank := by
  simp only [joined, capture, ← CountBank.collected_merge]
  congr 1
  simp only [rows, positions, List.flatMap_cons, List.flatMap_nil, List.append_nil,
    ← List.map_append]
  congr 1
  simpa only [positions, List.flatMap_cons, List.flatMap_nil, List.append_nil] using
    split_positions start length split within

theorem checked_split_exact (span left right : Interval) (offset : Nat)
    (accepted : splitChecked span offset = .ok (left, right)) :
    left.disjoint right ∧ positions [left, right] = span.positions := by
  unfold splitChecked at accepted
  split at accepted
  · rename_i within
    cases accepted
    exact ⟨adjacent_intervals _ _ _, split_positions _ _ _ within⟩
  · contradiction

theorem invalid_cut_refused (span : Interval) (offset : Nat) (outside : span.length < offset) :
    splitChecked span offset = .error .invalidCut := by
  simp [splitChecked, Nat.not_le.mpr outside]

/-- One physical observation contributes once, even if other observations have
the same classified value. This is a multiset of values, not value deduplication. -/
theorem observation_multiplicity (left right result : Summary Owner Key)
    (accepted : mergeChecked left right = .ok result) (position : Nat) :
    (positions result.spans).count position = if position ∈ positions result.spans then 1 else 0 :=
  (accepted_unique left right result accepted).count

end CountBankOwnership
