import VerumExternalReplay.AcquiredObserver
import VerumExternalReplay.NumeralPrograms

/-! Exact connection between the observer VM's checked UInt64 successor and
the partial natural-number semantics used by typed numeral programs. This
proves the model connection, not correctness of a foreign VM or enumerator. -/
namespace CheckedNumerals
open AcquiredObserver NumeralPrograms.Typed
set_option warningAsError true

def naturalResult : Except Error Value → Option Nat
  | .ok (.nat n) => some n.toNat
  | _ => none

theorem successor_correspondence (maxTrace : Nat) (n : UInt64) :
    naturalResult (apply maxTrace .succ [.nat n]) =
      boundedSuccessor u64Maximum n.toNat := by
  by_cases h : n.toNat + 1 < 2^64
  · have before : n.toNat < u64Maximum := by unfold u64Maximum; omega
    simp [apply, arity, h, naturalResult, boundedSuccessor, before,
      UInt64.toNat_add, Nat.mod_eq_of_lt h]
  · have full : ¬ n.toNat < u64Maximum := by unfold u64Maximum; omega
    simp [apply, arity, h, naturalResult, boundedSuccessor, full]

theorem overflow_is_undefined : primitiveClass .overflow = .undefined := rfl

theorem wrong_arity (maxTrace : Nat) : apply maxTrace .succ [] = .error .arity := rfl

theorem wrong_sort (maxTrace : Nat) : apply maxTrace .succ [.unit] = .error .sort := rfl

theorem preserves_machine_bound (maxTrace : Nat) (n output : UInt64)
    (success : apply maxTrace .succ [.nat n] = .ok (.nat output)) :
    output.toNat = n.toNat + 1 ∧ output.toNat ≤ u64Maximum := by
  have same := successor_correspondence maxTrace n
  rw [success] at same
  simp only [naturalResult, boundedSuccessor] at same
  split at same
  · have value := Option.some.inj same
    constructor
    · exact value
    · unfold u64Maximum at *; omega
  · cases same
end CheckedNumerals
