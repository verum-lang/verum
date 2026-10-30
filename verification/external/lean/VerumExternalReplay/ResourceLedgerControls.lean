import VerumExternalReplay.ResourceLedger

namespace ResourceLedgerControls
open ResourceLedger
set_option warningAsError true
deriving instance DecidableEq for Except

def original : Ledger := ⟨7, 255, some 24, 16, false, [⟨1,16,1⟩]⟩
def shared : Ledger := { original with slots := [⟨1,16,2⟩] }
def noQuota : Ledger := { original with quota := none }
def twoOwners : Ledger := { original with slots := [⟨1,8,1⟩,⟨0,8,1⟩] }
def pending : Wait (Nat × Nat × Nat) := ⟨7, .capacity, 32, (11,13,17)⟩

theorem admitted_includes_other_owners :
    total (admitted twoOwners 2 8).slots = 24 ∧ classTotal 2 (admitted twoOwners 2 8).slots = 8 := by decide_cbv

theorem foreign_class_unchanged : classTotal 1 (admitted twoOwners 2 8).slots = 8 := by decide_cbv

theorem exact_quota_accepted : reserve original 0 8 = .ok (admitted original 0 8) := by decide_cbv

theorem quota_plus_one_refused : reserve original 0 9 = .error .capacity := by decide_cbv

theorem growth_needs_full_replacement : reserve original 1 32 = .error .capacity := by decide_cbv

theorem no_quota_does_not_remove_word_bound : reserve noQuota 0 240 = .error .overflow := by decide_cbv

theorem poison_not_hidden_by_unlimited_quota :
    reserve { noQuota with poisoned := true } 0 0 = .error .poisoned := by decide_cbv

theorem zero_bytes_do_not_raise_total : total (admitted original 0 0).slots = 16 := by decide_cbv

theorem alias_counts_one_allocation :
    total (commit original (alias original [] ⟨1,16,1⟩ [])).slots = 16 := by decide_cbv

theorem nonfinal_drop_keeps_charge :
    total (commit shared (release shared [] ⟨1,16,2⟩ [])).slots = 16 := by decide_cbv

theorem final_drop_releases_charge :
    total (commit original (release original [] ⟨1,16,1⟩ [])).slots = 0 := by decide_cbv

theorem foreign_slot_refused : release original [] ⟨1,17,1⟩ [] = .error .foreign := by decide_cbv

theorem released_slot_cannot_drop_twice :
    release { original with slots := [⟨1,16,0⟩] } [] ⟨1,16,0⟩ [] = .error .dead := by decide_cbv

theorem failed_release_preserves_owner :
    commit original (release original [] ⟨1,17,1⟩ []) = original := by decide_cbv

theorem low_quota_cannot_revoke_live_bytes : grant original (some 15) = .error .capacity := by decide_cbv

theorem sufficient_grant_wakes_same_computation :
    resume (commit original (grant original (some 48))) pending = some (11,13,17) := by decide_cbv

theorem insufficient_grant_still_waits :
    resume (commit original (grant original (some 47))) pending = none := by decide_cbv

theorem foreign_account_cannot_wake :
    ready { original with quota := none, account := 8 } pending = false := by decide_cbv

theorem permanent_refusal_does_not_wake :
    ready noQuota { pending with reason := .overflow } = false := by decide_cbv

theorem rollback_retains_peak :
    let grown := admitted { original with quota := some 48 } 1 32
    let rolledBack := commit grown (release grown [⟨1,16,1⟩] ⟨1,32,1⟩ [])
    total rolledBack.slots = 16 ∧ rolledBack.peak = 48 := by decide_cbv

theorem poison_does_not_destroy_existing_alias_authority :
    alias { original with poisoned := true } [] ⟨1,16,1⟩ [] =
      .ok { original with poisoned := true, slots := [⟨1,16,2⟩] } := by decide_cbv

theorem exclusive_unbox_moves_post_predicate_payload :
    unbox original [] ⟨1,16,1⟩ [] (13,17) true =
      .ok ({ original with slots := [⟨1,16,0⟩] }, (13,17)) := by decide_cbv

theorem unready_unbox_is_refused :
    unbox original [] ⟨1,16,1⟩ [] (13,17) false = .error .notReady := by decide_cbv

theorem shared_unbox_is_refused :
    unbox shared [] ⟨1,16,2⟩ [] (13,17) true = .error .shared := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : reserve original 1 32 = .ok (admitted original 1 32) := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : total (commit shared (release shared [] ⟨1,16,2⟩ [])).slots = 0 := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : total (commit original (alias original [] ⟨1,16,1⟩ [])).slots = 32 := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : resume (commit original (grant original (some 48))) pending = some (11,13,18) := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : ready noQuota { pending with reason := .poisoned } = true := by decide_cbv

end ResourceLedgerControls
