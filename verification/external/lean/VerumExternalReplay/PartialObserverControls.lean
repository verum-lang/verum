import VerumExternalReplay.PartialObservers

namespace PartialObserverControls
open PartialObservers
set_option warningAsError true

def ordered : Observer Nat Nat Nat where
  initial := 1
  update z e := some (2 * z + e)
  read := some

def pending : Observer Nat Nat Nat where
  initial := 0
  update z e := some (z + e)
  read _ := none

def guarded : Observer Nat Nat Nat where
  initial := 0
  update z e := if e = 0 then none else some (z + e)
  read := some

example : ordered.observe [1, 2] = some 8 := rfl
example : ordered.observe [2, 1] = some 9 := rfl
example : ordered.replay [] = some 1 := rfl
example : (ordered.product pending).observe [1, 2] = none := rfl
example : guarded.replay [1, 0, 2] = none := rfl
example : (ordered.product guarded).replay [1, 0, 2] = none := rfl
example : online ordered.update [1, 2] (some ordered.initial) = ordered.replay [1, 2] := rfl
example : (ordered.replay [1]).getD 0 + (ordered.replay [2]).getD 0 ≠
    (ordered.replay [1, 2]).getD 0 := by decide

end PartialObserverControls
