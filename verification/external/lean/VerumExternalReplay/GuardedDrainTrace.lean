import VerumExternalReplay.GuardedDrain

/-! Projection replay for the 64-bit native driver's exported control boundaries.
Ranks are observations, not complete callback states. This checker establishes
guard decisions and continuity within each grant, not VM/allocator refinement.
Ready/refused callbacks close the service according to the source return branch;
the export does not separately observe those outer return values.
-/
namespace GuardedDrainTrace
open WorkDrain GuardedDrain
set_option warningAsError true

abbrev NativeRank := Rank (2^64) 7 (2^64)

inductive Event (E : Type) where
  | enter (work : Nat)
  | callback (beforeWork afterWork : Nat) (beforeRank afterRank : NativeRank) (result : Status E)
  | yielded (remaining : Nat)
  | exhausted (remaining : Nat)

inductive Cursor where
  | closed
  | active (work : Nat) (lastRank : Option NativeRank)
  | yielding (remaining : Nat)
  | exhausting
  deriving DecidableEq

def advance (cursor : Cursor) (event : Event E) : Option Cursor :=
  match cursor,event with
  | .closed,.enter work => some (if work = 0 then .exhausting else .active work none)
  | .active work lastRank,.callback beforeWork afterWork beforeRank afterRank result =>
      if beforeWork ≠ work ∨ (lastRank.isSome ∧ lastRank ≠ some beforeRank) then none else
      match classify beforeWork beforeRank.value afterWork afterRank.value result with
      | .pending => some (if afterWork = 0 then .exhausting else .active afterWork (some afterRank))
      | .yielded => some (.yielding afterWork)
      | .ready | .refused _ => some .closed
  | .yielding remaining,.yielded actual => if remaining = actual then some .closed else none
  | .exhausting,.exhausted actual => if actual = 0 then some .closed else none
  | _,_ => none

def valid (events : List (Event E)) : Bool :=
  events.foldlM advance .closed == some .closed

theorem zero_inert : valid (E := E) [.enter 0,.exhausted 0] = true := rfl

theorem zero_cannot_call (beforeWork afterWork : Nat) (beforeRank afterRank : NativeRank)
    (result : Status E) (rest : List (Event E)) :
    valid (.enter 0 :: .callback beforeWork afterWork beforeRank afterRank result :: rest) = false := by
  simp [valid,List.foldlM,advance]

theorem pending_end_rejected (work : Nat) (rank : Option NativeRank) :
    (List.foldlM advance (.active work rank) ([] : List (Event E)) == some .closed) = false := rfl

theorem wrong_grant_rejected (work beforeWork afterWork : Nat) (lastRank : Option NativeRank)
    (beforeRank afterRank : NativeRank) (result : Status E) (different : beforeWork ≠ work) :
    advance (.active work lastRank) (.callback beforeWork afterWork beforeRank afterRank result) = none := by
  simp [advance,different]

end GuardedDrainTrace
