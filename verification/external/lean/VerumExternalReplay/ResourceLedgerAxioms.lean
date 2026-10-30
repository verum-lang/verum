import VerumExternalReplay.ResourceLedgerControls

namespace ResourceLedgerAxioms
set_option warningAsError true

/-- info: 'ResourceLedger.total_append' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedger.total_append

/-- info: 'ResourceLedger.class_append' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedger.class_append

/-- info: 'ResourceLedger.admitted_total' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedger.admitted_total

/-- info: 'ResourceLedger.admitted_class' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedger.admitted_class

/-- info: 'ResourceLedger.other_class_preserved' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedger.other_class_preserved

/-- info: 'ResourceLedger.reserve_acceptance' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedger.reserve_acceptance

/-- info: 'ResourceLedger.admitted_preserves_valid' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedger.admitted_preserves_valid

/-- info: 'ResourceLedger.original_slots_preserved' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedger.original_slots_preserved

/-- info: 'ResourceLedger.refused_ownership_projection_unchanged' does not depend on any axioms -/
#guard_msgs in
#print axioms ResourceLedger.refused_ownership_projection_unchanged

/-- info: 'ResourceLedger.alias_charge' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedger.alias_charge

/-- info: 'ResourceLedger.alias_total' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedger.alias_total

/-- info: 'ResourceLedger.nonfinal_release_total' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ResourceLedger.nonfinal_release_total

/-- info: 'ResourceLedger.final_release_total' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedger.final_release_total

/-- info: 'ResourceLedger.final_release_class' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedger.final_release_class

/-- info: 'ResourceLedger.release_cannot_increase' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ResourceLedger.release_cannot_increase

/-- info: 'ResourceLedger.release_keeps_peak' does not depend on any axioms -/
#guard_msgs in
#print axioms ResourceLedger.release_keeps_peak

/-- info: 'ResourceLedger.poisoned_cannot_reserve' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedger.poisoned_cannot_reserve

/-- info: 'ResourceLedger.poisoned_can_release' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedger.poisoned_can_release

/-- info: 'ResourceLedger.grant_cannot_revoke' does not depend on any axioms -/
#guard_msgs in
#print axioms ResourceLedger.grant_cannot_revoke

/-- info: 'ResourceLedger.growth_coexistence' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedger.growth_coexistence

/-- info: 'ResourceLedger.rollback_total' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedger.rollback_total

/-- info: 'ResourceLedger.rollback_keeps_peak' does not depend on any axioms -/
#guard_msgs in
#print axioms ResourceLedger.rollback_keeps_peak

/-- info: 'ResourceLedger.nonresumable_never_ready' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedger.nonresumable_never_ready

/-- info: 'ResourceLedger.foreign_wait_never_ready' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedger.foreign_wait_never_ready

/-- info: 'ResourceLedger.resume_preserves_computation' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedger.resume_preserves_computation

/-- info: 'ResourceLedger.move_does_not_allocate' does not depend on any axioms -/
#guard_msgs in
#print axioms ResourceLedger.move_does_not_allocate

/-- info: 'ResourceLedger.quota_downward' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ResourceLedger.quota_downward

/-- info: 'ResourceLedger.admissible_downward' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ResourceLedger.admissible_downward

/-- info: 'ResourceLedger.release_preserves_valid' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ResourceLedger.release_preserves_valid

/-- info: 'ResourceLedger.alias_preserves_valid' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedger.alias_preserves_valid

/-- info: 'ResourceLedger.release_preserves_eligibility' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ResourceLedger.release_preserves_eligibility

/-- info: 'ResourceLedger.unbox_moves_same_payload' does not depend on any axioms -/
#guard_msgs in
#print axioms ResourceLedger.unbox_moves_same_payload

/-- info: 'ResourceLedger.unbox_releases_container' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedger.unbox_releases_container

/-- info: 'ResourceLedger.unready_cannot_unbox' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedger.unready_cannot_unbox

/-- info: 'ResourceLedger.class_partition' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedger.class_partition

/-- info: 'ResourceLedger.finite_class_partition' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ResourceLedger.finite_class_partition

/-- info: 'ResourceLedger.complete_classes_conserve_bytes' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ResourceLedger.complete_classes_conserve_bytes

/-- info: 'ResourceLedger.reservations_append' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedger.reservations_append

/-- info: 'ResourceLedger.admitted_reservations' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedger.admitted_reservations

/-- info: 'ResourceLedger.alias_keeps_reservations' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedger.alias_keeps_reservations

/-- info: 'ResourceLedger.checked_rollback_preserves_old_slots' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedger.checked_rollback_preserves_old_slots

/-- info: 'ResourceLedger.checked_growth_replaces_only_old_slot' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedger.checked_growth_replaces_only_old_slot

/-- info: 'ResourceLedger.occupied_bounded_by_charge' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ResourceLedger.occupied_bounded_by_charge

/-- info: 'ResourceLedger.reservations_bounded_by_bytes' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ResourceLedger.reservations_bounded_by_bytes

/-- info: 'ResourceLedger.valid_reservations_fit_word' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ResourceLedger.valid_reservations_fit_word

/-- info: 'ResourceLedgerControls.admitted_includes_other_owners' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedgerControls.admitted_includes_other_owners

/-- info: 'ResourceLedgerControls.foreign_class_unchanged' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedgerControls.foreign_class_unchanged

/-- info: 'ResourceLedgerControls.exact_quota_accepted' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedgerControls.exact_quota_accepted

/-- info: 'ResourceLedgerControls.quota_plus_one_refused' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedgerControls.quota_plus_one_refused

/-- info: 'ResourceLedgerControls.growth_needs_full_replacement' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedgerControls.growth_needs_full_replacement

/-- info: 'ResourceLedgerControls.no_quota_does_not_remove_word_bound' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedgerControls.no_quota_does_not_remove_word_bound

/-- info: 'ResourceLedgerControls.poison_not_hidden_by_unlimited_quota' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedgerControls.poison_not_hidden_by_unlimited_quota

/-- info: 'ResourceLedgerControls.zero_bytes_do_not_raise_total' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedgerControls.zero_bytes_do_not_raise_total

/-- info: 'ResourceLedgerControls.alias_counts_one_allocation' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedgerControls.alias_counts_one_allocation

/-- info: 'ResourceLedgerControls.nonfinal_drop_keeps_charge' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedgerControls.nonfinal_drop_keeps_charge

/-- info: 'ResourceLedgerControls.final_drop_releases_charge' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedgerControls.final_drop_releases_charge

/-- info: 'ResourceLedgerControls.foreign_slot_refused' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedgerControls.foreign_slot_refused

/-- info: 'ResourceLedgerControls.released_slot_cannot_drop_twice' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedgerControls.released_slot_cannot_drop_twice

/-- info: 'ResourceLedgerControls.failed_release_preserves_owner' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedgerControls.failed_release_preserves_owner

/-- info: 'ResourceLedgerControls.low_quota_cannot_revoke_live_bytes' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedgerControls.low_quota_cannot_revoke_live_bytes

/-- info: 'ResourceLedgerControls.sufficient_grant_wakes_same_computation' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedgerControls.sufficient_grant_wakes_same_computation

/-- info: 'ResourceLedgerControls.insufficient_grant_still_waits' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedgerControls.insufficient_grant_still_waits

/-- info: 'ResourceLedgerControls.foreign_account_cannot_wake' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedgerControls.foreign_account_cannot_wake

/-- info: 'ResourceLedgerControls.permanent_refusal_does_not_wake' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedgerControls.permanent_refusal_does_not_wake

/-- info: 'ResourceLedgerControls.rollback_retains_peak' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedgerControls.rollback_retains_peak

/-- info: 'ResourceLedgerControls.poison_does_not_destroy_existing_alias_authority' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedgerControls.poison_does_not_destroy_existing_alias_authority

/-- info: 'ResourceLedgerControls.exclusive_unbox_moves_post_predicate_payload' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedgerControls.exclusive_unbox_moves_post_predicate_payload

/-- info: 'ResourceLedgerControls.unready_unbox_is_refused' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedgerControls.unready_unbox_is_refused

/-- info: 'ResourceLedgerControls.shared_unbox_is_refused' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedgerControls.shared_unbox_is_refused

/-- info: 'ResourceLedger.reserve_at_capacity' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ResourceLedger.reserve_at_capacity

end ResourceLedgerAxioms
