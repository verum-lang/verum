import VerumExternalReplay.CountBank

namespace CountBankControls
open CountBank CategoricalScore
set_option warningAsError true

def zeroPrefix : Score := contextScore ⟨2, [⟨0, 0⟩]⟩
def onePrefix : Score := contextScore ⟨2, [⟨0, 1⟩]⟩

theorem equal_scores_lose_continuation : zeroPrefix = onePrefix ∧
    equivalent (contextScore ⟨2, [⟨0, 0⟩, ⟨0, 0⟩]⟩).predictive
      (contextScore ⟨2, [⟨0, 1⟩, ⟨0, 0⟩]⟩).predictive = false := by decide_cbv

theorem independent_products_lose_continuation :
    equivalent (mul zeroPrefix.predictive zeroPrefix.predictive)
      (contextScore ⟨2, [⟨0, 0⟩, ⟨0, 0⟩]⟩).predictive = false := by decide_cbv

theorem cell_names_matter :
    contextScore ⟨2, [⟨0, 0⟩]⟩ = contextScore ⟨2, [⟨1, 0⟩]⟩ ∧
    equivalent (contextScore ⟨2, [⟨0, 0⟩, ⟨0, 0⟩]⟩).compression
      (contextScore ⟨2, [⟨1, 0⟩, ⟨0, 0⟩]⟩).compression = false := by decide_cbv

theorem joined_counts_keep_labels :
    merge (rowChild [⟨0,0⟩]) (rowChild [⟨1,0⟩]) (0,0) = 1 ∧
    merge (rowChild [⟨0,0⟩]) (rowChild [⟨1,0⟩]) (1,0) = 1 := by decide_cbv

theorem overlap_is_counted_twice :
    merge (rowChild [⟨0,0⟩]) (rowChild [⟨0,0⟩]) (0,0) = 2 := by decide_cbv

theorem contexts_do_not_collapse :
    ingest id empty ([(0,7,0), (1,7,0)] : List (Nat × Nat × Nat)) (0,7,0) = 1 := by decide_cbv

theorem duplicate_census_changes_score :
    CountBank.score 2 1 [0,0] (rowParent [⟨0,0⟩]) (rowChild [⟨0,0⟩]) ≠
      contextScore ⟨2, [⟨0,0⟩]⟩ := by decide_cbv

theorem frozen_alphabet_matters :
    equivalent (contextScore ⟨2, [⟨0,0⟩]⟩).predictive
      (contextScore ⟨3, [⟨0,0⟩]⟩).predictive = false := by decide_cbv

-- A finite native counter could realize the abstract bank with this lookup.
-- The success simulation makes no assertion that every row is admissible.
def bounded (capacity state : Nat) (_ : Unit) : Except Unit Nat :=
  if state < capacity then .ok (state + 1) else .error ()
def scalar (state : Nat) : Bank Unit := fun _ => state

theorem bounded_step (capacity : Nat) :
    StepCorrespondence (bounded capacity) scalar (fun _ => ()) := by
  intro state row next accepted
  unfold bounded at accepted
  split at accepted
  · cases accepted
    funext key
    cases key
    rfl
  · contradiction

theorem bounded_accepted_count (capacity : Nat) (rows : List Unit) (initial final : Nat)
    (accepted : SharedUpdates.runFrom (bounded capacity) 0 rows initial = .complete final) :
    final = initial + rows.length := by
  have exactBank := accepted_correspondence (bounded capacity) scalar (fun _ => ())
    (bounded_step capacity) rows initial final 0 accepted
  have exactCount := congrFun exactBank ()
  have all : rows.filter (fun _ => true) = rows := List.filter_eq_self.mpr (by simp)
  simpa [scalar, ingest_lookup, count, all] using exactCount

theorem refusal_is_not_skipped :
    SharedUpdates.runFrom (bounded 1) 0 [(),(),()] 0 =
      .refused 1 1 () () := by decide_cbv

-- All split points and arbitrary row values, rather than a finite sample grid.
theorem every_split_scores (alphabet : Nat) (rows : List CategoricalScore.Row) (split : Nat) :
    CountBank.score alphabet rows.length ((rows.map Row.reading).eraseDups)
      (merge (rowParent (rows.take split)) (rowParent (rows.drop split)))
      (merge (rowChild (rows.take split)) (rowChild (rows.drop split))) =
      contextScore ⟨alphabet, rows⟩ := by
  have joined := append_score_correspondence alphabet (rows.take split) (rows.drop split)
  have lengths := congrArg List.length (List.take_append_drop split rows)
  simp only [List.length_append] at lengths
  simpa only [List.take_append_drop, lengths] using joined

example : CountBank.score 2 2 [1,0] (rowParent [⟨0,0⟩,⟨1,1⟩])
    (rowChild [⟨0,0⟩,⟨1,1⟩]) = contextScore ⟨2, [⟨0,0⟩,⟨1,1⟩]⟩ :=
  census_score_correspondence _ _ _ (by decide_cbv)

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : equivalent (contextScore ⟨2, [⟨0,0⟩,⟨0,0⟩]⟩).predictive
    (contextScore ⟨2, [⟨0,1⟩,⟨0,0⟩]⟩).predictive = true := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : equivalent (mul zeroPrefix.predictive zeroPrefix.predictive)
    (contextScore ⟨2, [⟨0,0⟩,⟨0,0⟩]⟩).predictive = true := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : merge (rowChild [⟨0,0⟩]) (rowChild [⟨0,0⟩]) (0,0) = 1 := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : SharedUpdates.runFrom (bounded 1) 0 [(),(),()] 0 = .complete 3 := by decide_cbv

-- Exact dependency rosters exclude admitted, choice and native-oracle proofs.
/-- info: 'CountBank.count_append' depends on axioms: [propext] -/
#guard_msgs in
#print axioms CountBank.count_append

/-- info: 'CountBank.ingest_lookup' depends on axioms: [propext] -/
#guard_msgs in
#print axioms CountBank.ingest_lookup

/-- info: 'CountBank.ingest_append' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms CountBank.ingest_append

/-- info: 'CountBank.collected_merge' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms CountBank.collected_merge

/-- info: 'CountBank.sample_count' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms CountBank.sample_count

/-- info: 'CountBank.merge_associative' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms CountBank.merge_associative

/-- info: 'CountBank.merge_commutative' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms CountBank.merge_commutative

/-- info: 'CountBank.empty_identity' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms CountBank.empty_identity

/-- info: 'CountBank.support_union' depends on axioms: [propext] -/
#guard_msgs in
#print axioms CountBank.support_union

/-- info: 'CountBank.accepted_correspondence' depends on axioms: [propext] -/
#guard_msgs in
#print axioms CountBank.accepted_correspondence

/-- info: 'CountBank.ratio_product_permutation' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms CountBank.ratio_product_permutation

/-- info: 'CountBank.score_cell_order' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms CountBank.score_cell_order

/-- info: 'CountBank.cell_census_append' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms CountBank.cell_census_append

/-- info: 'CountBank.parent_vector' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms CountBank.parent_vector

/-- info: 'CountBank.child_vector' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms CountBank.child_vector

/-- info: 'CountBank.score_correspondence' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms CountBank.score_correspondence

/-- info: 'CountBank.census_score_correspondence' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms CountBank.census_score_correspondence

/-- info: 'CountBank.append_score_correspondence' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms CountBank.append_score_correspondence

/-- info: 'CountBankControls.equal_scores_lose_continuation' depends on axioms: [propext] -/
#guard_msgs in
#print axioms CountBankControls.equal_scores_lose_continuation

/-- info: 'CountBankControls.independent_products_lose_continuation' depends on axioms: [propext] -/
#guard_msgs in
#print axioms CountBankControls.independent_products_lose_continuation

/-- info: 'CountBankControls.cell_names_matter' depends on axioms: [propext] -/
#guard_msgs in
#print axioms CountBankControls.cell_names_matter

/-- info: 'CountBankControls.joined_counts_keep_labels' depends on axioms: [propext] -/
#guard_msgs in
#print axioms CountBankControls.joined_counts_keep_labels

/-- info: 'CountBankControls.overlap_is_counted_twice' depends on axioms: [propext] -/
#guard_msgs in
#print axioms CountBankControls.overlap_is_counted_twice

/-- info: 'CountBankControls.contexts_do_not_collapse' depends on axioms: [propext] -/
#guard_msgs in
#print axioms CountBankControls.contexts_do_not_collapse

/-- info: 'CountBankControls.duplicate_census_changes_score' depends on axioms: [propext] -/
#guard_msgs in
#print axioms CountBankControls.duplicate_census_changes_score

/-- info: 'CountBankControls.frozen_alphabet_matters' depends on axioms: [propext] -/
#guard_msgs in
#print axioms CountBankControls.frozen_alphabet_matters

/-- info: 'CountBankControls.bounded_step' depends on axioms: [Quot.sound] -/
#guard_msgs in
#print axioms CountBankControls.bounded_step

/-- info: 'CountBankControls.bounded_accepted_count' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms CountBankControls.bounded_accepted_count

/-- info: 'CountBankControls.refusal_is_not_skipped' depends on axioms: [propext] -/
#guard_msgs in
#print axioms CountBankControls.refusal_is_not_skipped

/-- info: 'CountBankControls.every_split_scores' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms CountBankControls.every_split_scores

end CountBankControls
