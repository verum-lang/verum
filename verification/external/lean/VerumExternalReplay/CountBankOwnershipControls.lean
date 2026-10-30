import VerumExternalReplay.CountBankOwnership

namespace CountBankOwnershipControls
open CountBankOwnership
set_option warningAsError true

-- Owner equality includes both partition identity and frozen revision. A native
-- mapping must additionally bind all classifier/profile and input-ledger identity.
def summary (owner : Nat × Nat) (spans : List Interval) : Summary (Nat × Nat) Nat :=
  capture (fun _ _ => 7) (fun _ => id) owner spans

def outcome (result : Except Refusal Value) : Option Refusal :=
  match result with
  | .ok _ => none
  | .error reason => some reason

theorem adjacent_accepted :
    outcome (mergeChecked (summary (1,3) [⟨0,4⟩]) (summary (1,3) [⟨4,2⟩])) = none := by decide_cbv

theorem partition_refused :
    outcome (mergeChecked (summary (1,3) [⟨0,4⟩]) (summary (2,3) [⟨4,2⟩])) =
      some .foreignOwner := by decide_cbv

theorem revision_refused :
    outcome (mergeChecked (summary (1,3) [⟨0,4⟩]) (summary (1,4) [⟨4,2⟩])) =
      some .foreignOwner := by decide_cbv

theorem shared_base_refused :
    outcome (mergeChecked (summary (1,3) [⟨0,4⟩,⟨4,2⟩]) (summary (1,3) [⟨0,4⟩,⟨6,2⟩])) =
      some .overlap := by decide_cbv

theorem partial_overlap_refused :
    outcome (mergeChecked (summary (1,3) [⟨0,4⟩]) (summary (1,3) [⟨3,2⟩])) =
      some .overlap := by decide_cbv

theorem invalid_input_refused :
    outcome (mergeChecked (summary (1,3) [⟨0,4⟩,⟨2,2⟩]) (summary (1,3) [⟨4,2⟩])) =
      some .overlap := by decide_cbv

theorem empty_inside_interval_accepted :
    outcome (mergeChecked (summary (1,3) [⟨0,4⟩]) (summary (1,3) [⟨2,0⟩])) = none := by decide_cbv

theorem out_of_range_cut_refused :
    outcome (splitChecked ⟨10,4⟩ 5) = some .invalidCut := by decide_cbv

theorem equal_values_keep_physical_multiplicity :
    (joined (summary (1,3) [⟨0,4⟩]) (summary (1,3) [⟨4,2⟩])).bank 7 = 6 := by decide_cbv

theorem summary_merge_does_not_sort_events :
    positions (joined (summary (1,3) [⟨4,2⟩]) (summary (1,3) [⟨0,4⟩])).spans =
      [4,5,0,1,2,3] := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : outcome (mergeChecked (summary (1,3) [⟨0,4⟩,⟨4,2⟩])
    (summary (1,3) [⟨0,4⟩,⟨6,2⟩])) = none := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : outcome (mergeChecked (summary (1,3) [⟨0,4⟩])
    (summary (2,3) [⟨4,2⟩])) = none := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : outcome (mergeChecked (summary (1,3) [⟨0,4⟩])
    (summary (1,4) [⟨4,2⟩])) = none := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : positions (joined (summary (1,3) [⟨4,2⟩]) (summary (1,3) [⟨0,4⟩])).spans =
    [0,1,2,3,4,5] := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : outcome (splitChecked ⟨10,4⟩ 5) = none := by decide_cbv

-- Pin every theorem dependency; no admitted or native-oracle proofs.
/-- info: 'CountBankOwnership.interval_unique' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms CountBankOwnership.interval_unique

/-- info: 'CountBankOwnership.interval_disjoint' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms CountBankOwnership.interval_disjoint

/-- info: 'CountBankOwnership.unique_positions' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms CountBankOwnership.unique_positions

/-- info: 'CountBankOwnership.capture_correspondence' does not depend on any axioms -/
#guard_msgs in
#print axioms CountBankOwnership.capture_correspondence

/-- info: 'CountBankOwnership.merge_acceptance' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms CountBankOwnership.merge_acceptance

/-- info: 'CountBankOwnership.accepted_shape' does not depend on any axioms -/
#guard_msgs in
#print axioms CountBankOwnership.accepted_shape

/-- info: 'CountBankOwnership.accepted_unique' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms CountBankOwnership.accepted_unique

/-- info: 'CountBankOwnership.accepted_correspondence' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms CountBankOwnership.accepted_correspondence

/-- info: 'CountBankOwnership.foreign_owner_refused' depends on axioms: [propext] -/
#guard_msgs in
#print axioms CountBankOwnership.foreign_owner_refused

/-- info: 'CountBankOwnership.overlapping_position_refused' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms CountBankOwnership.overlapping_position_refused

/-- info: 'CountBankOwnership.shared_prefix_refused' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms CountBankOwnership.shared_prefix_refused

/-- info: 'CountBankOwnership.adjacent_intervals' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms CountBankOwnership.adjacent_intervals

/-- info: 'CountBankOwnership.split_positions' depends on axioms: [propext] -/
#guard_msgs in
#print axioms CountBankOwnership.split_positions

/-- info: 'CountBankOwnership.adjacent_banks_accepted' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms CountBankOwnership.adjacent_banks_accepted

/-- info: 'CountBankOwnership.split_bank' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms CountBankOwnership.split_bank

/-- info: 'CountBankOwnership.checked_split_exact' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms CountBankOwnership.checked_split_exact

/-- info: 'CountBankOwnership.invalid_cut_refused' depends on axioms: [propext] -/
#guard_msgs in
#print axioms CountBankOwnership.invalid_cut_refused

/-- info: 'CountBankOwnership.observation_multiplicity' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms CountBankOwnership.observation_multiplicity

/-- info: 'CountBankOwnershipControls.adjacent_accepted' depends on axioms: [propext] -/
#guard_msgs in
#print axioms CountBankOwnershipControls.adjacent_accepted

/-- info: 'CountBankOwnershipControls.partition_refused' depends on axioms: [propext] -/
#guard_msgs in
#print axioms CountBankOwnershipControls.partition_refused

/-- info: 'CountBankOwnershipControls.revision_refused' depends on axioms: [propext] -/
#guard_msgs in
#print axioms CountBankOwnershipControls.revision_refused

/-- info: 'CountBankOwnershipControls.shared_base_refused' depends on axioms: [propext] -/
#guard_msgs in
#print axioms CountBankOwnershipControls.shared_base_refused

/-- info: 'CountBankOwnershipControls.partial_overlap_refused' depends on axioms: [propext] -/
#guard_msgs in
#print axioms CountBankOwnershipControls.partial_overlap_refused

/-- info: 'CountBankOwnershipControls.invalid_input_refused' depends on axioms: [propext] -/
#guard_msgs in
#print axioms CountBankOwnershipControls.invalid_input_refused

/-- info: 'CountBankOwnershipControls.empty_inside_interval_accepted' depends on axioms: [propext] -/
#guard_msgs in
#print axioms CountBankOwnershipControls.empty_inside_interval_accepted

/-- info: 'CountBankOwnershipControls.out_of_range_cut_refused' depends on axioms: [propext] -/
#guard_msgs in
#print axioms CountBankOwnershipControls.out_of_range_cut_refused

/-- info: 'CountBankOwnershipControls.equal_values_keep_physical_multiplicity' depends on axioms: [propext] -/
#guard_msgs in
#print axioms CountBankOwnershipControls.equal_values_keep_physical_multiplicity

/-- info: 'CountBankOwnershipControls.summary_merge_does_not_sort_events' depends on axioms: [propext] -/
#guard_msgs in
#print axioms CountBankOwnershipControls.summary_merge_does_not_sort_events

end CountBankOwnershipControls
