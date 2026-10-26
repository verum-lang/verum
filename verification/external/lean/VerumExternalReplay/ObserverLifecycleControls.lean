import VerumExternalReplay.ObserverOwnership
import VerumExternalReplay.ExactNames
import VerumExternalReplay.ObserverInstallation

namespace ObserverLifecycleControls

open ObserverOwnership
set_option warningAsError true

def before : Cut Nat Nat Nat := ⟨7, 1, 10⟩
def prepared : Prepared Nat Nat Nat := ⟨before, 11⟩
def advanced : Cut Nat Nat Nat := ⟨7, 1, 11⟩
def foreign : Cut Nat Nat Nat := ⟨7, 2, 12⟩

theorem foreign_before_stale :
    commit foreign prepared = ⟨foreign, .error (.foreignInstance, prepared)⟩ := rfl

theorem stale_keeps_prepared :
    commit advanced prepared = ⟨advanced, .error (.staleState, prepared)⟩ := rfl

theorem successful_transition :
    commit before prepared = ⟨advanced, .ok ⟨before, advanced⟩⟩ := rfl

theorem fork_owner_is_distinct :
    commit before ⟨fork before 2, 11⟩ =
      ⟨before, .error (.foreignInstance, ⟨fork before 2, 11⟩)⟩ := rfl

-- Equal payloads do not identify immutable state allocations.
def payload (_ : Nat) : Nat := 0
theorem equal_payload_still_stale :
    payload before.state = payload advanced.state ∧
    (commit advanced prepared).receipt = .error (.staleState, prepared) := ⟨rfl, rfl⟩

theorem abandoned_preparation_retains_cut :
    prepare (E := Nat) before (.ok 11) = .ok prepared ∧
    read (E := Nat) before (.ok 42) = .ok (before, 42) := ⟨rfl, rfl⟩

theorem failed_operations_retain_error :
    prepare before (.error 3) = .error 3 ∧
    read (V := Nat) before (.error 4) = .error 4 := ⟨rfl, rfl⟩

-- Full product keys preserve both input and output components.
def named := ExactNames.intern 4 ([] : List (Nat × Nat)) (5, 300)
def forkNamed := ExactNames.intern 4 named.1 (5, 556)

theorem full_value_not_low_byte : named.2 = some 0 ∧ forkNamed.2 = some 1 := ⟨rfl, rfl⟩
theorem full_input_not_only_value :
    (ExactNames.intern 4 named.1 (6, 300)).2 = some 1 := rfl
theorem shared_namespace_reuses_name :
    (ExactNames.intern 4 forkNamed.1 (5, 300)).2 = some 0 := rfl
theorem full_table_retains_existing :
    ExactNames.intern 1 named.1 (5, 300) = (named.1, some 0) := rfl
theorem capacity_refuses_new_key :
    ExactNames.intern 1 named.1 (5, 556) = (named.1, none) := rfl
theorem independent_tables_can_collide :
    named.2 = (ExactNames.intern 4 ([] : List (Nat × Nat)) (5, 556)).2 := rfl

def receiver : ObserverInstallation.History Nat Nat := ⟨3, [some 2, some 4]⟩
def retained : ObserverInstallation.History Nat Nat := ⟨3, [some 2]⟩
def increment (state event : Nat) : Option Nat := some (state + event)
theorem receiver_replays_from_initial :
    ObserverInstallation.install increment 10 [retained] receiver 2 = some 16 := rfl
theorem conflicting_history_is_refused :
    ObserverInstallation.install increment 10 [⟨3, [some 9]⟩] receiver 2 = none := rfl
theorem gap_is_not_an_identity_event :
    ObserverInstallation.install increment 10 ([] : List (ObserverInstallation.History Nat Nat))
      ⟨3, [some 2, none, some 4]⟩ 3 = none := rfl
theorem omitted_suffix_is_not_replayed :
    ObserverInstallation.install increment 10 [retained] receiver 1 = some 12 := rfl
theorem unrelated_origin_is_not_compared :
    ObserverInstallation.install increment 10 [⟨4, [some 9]⟩] receiver 2 = some 16 := rfl
theorem global_cut_cannot_replace_relative :
    ObserverInstallation.install increment 10 ([] : List (ObserverInstallation.History Nat Nat))
      ⟨3, [some 4]⟩ 2 = none := rfl

end ObserverLifecycleControls
