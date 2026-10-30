import VerumExternalReplay.ActionRegistry

/-! Concrete controls for exact names, finite transactional admission, and
owner-relative coordinates. Rejection controls require semantic falsity, not a
parser or tool failure. No native map/allocator/transport execution is modeled. -/
namespace ActionRegistryControls
open ActionRegistry
set_option warningAsError true
set_option maxRecDepth 2048

-- Local decidable equality for control results; production refusal semantics
-- remain the existing Except constructors.
local instance : DecidableEq (Except Refusal (List Nat)) := fun first second =>
  match first, second with
  | .ok a, .ok b =>
      if h : a = b then isTrue (by cases h; rfl)
      else isFalse (fun h' => h (Except.ok.inj h'))
  | .error a, .error b =>
      if h : a = b then isTrue (by cases h; rfl)
      else isFalse (fun h' => h (Except.error.inj h'))
  | .ok _, .error _ => isFalse (by intro h; cases h)
  | .error _, .ok _ => isFalse (by intro h; cases h)

def state : StateId := ⟨123, by decide⟩
def farState : StateId := ⟨2^63, by decide⟩
def donor : List Nat := [20,10]
def receiver : List Nat := [10,30]
def donorContext : Context := ⟨state,0⟩
def receivedContext : Context := ⟨state,2⟩
def merged : List Nat := (extend 4 true receiver donor).1

def byteRank (n : Nat) : Nat := n % 256
example : byteRank 0 = byteRank 256 := by decide_cbv
example : Context.mk state 0 ≠ Context.mk state 256 := by decide_cbv
example : Context.mk state 256 ≠ Context.mk farState 256 := by decide_cbv
example : name (List.range 257) 0 = some 0 := by decide_cbv
example : name (List.range 257) 256 = some 256 := by decide_cbv

example : fresh receiver [20,10,20,40] = [20,40] := by decide_cbv
example : freshNames receiver [20,10,20,40] = [2,3] := by decide_cbv
example : extend 4 true receiver [20,10,20,40] = ([10,30,20,40],.ok [2,3]) := by decide_cbv
example : name merged 10 = name receiver 10 := by decide_cbv
example : name merged 30 = name receiver 30 := by decide_cbv
example : name merged 20 = some 2 := by decide_cbv
example : bind donor state 20 = some donorContext := by decide_cbv
example : bind merged state 20 = some receivedContext := by decide_cbv
example : rebind donor merged donorContext 20 = some receivedContext := by decide_cbv
example : ownsAt merged receivedContext.ordinal 20 = true := by decide_cbv
example : ownsAt receiver donorContext.ordinal 20 = false := by decide_cbv
example : rebind receiver merged donorContext 20 = none := by decide_cbv
example : rebind donor receiver donorContext 20 = none := by decide_cbv
example : bind [20,20] state 20 = none := by decide_cbv
example : (ownsAt [10,20] 0 10 && ownsAt [10,20] 0 20) = false := by decide_cbv
example : rebind donor merged ⟨state,99⟩ 20 = none := by decide_cbv

example : extend 3 true receiver [20,40] = (receiver,.error .addressability) := by decide_cbv
example : extend 4 false receiver [20,40] = (receiver,.error .admission) := by decide_cbv
example : extend 4 true [10,10] [20] = ([10,10],.error .malformedRegistry) := by decide_cbv
example : extend 0 true ([] : List Nat) [] = ([],.ok []) := by decide_cbv
example : extend 2 true receiver [10,30,10] = (receiver,.ok []) := by decide_cbv

-- The issued interval is tied to actual names and Context binding.
example : name (receiver ++ fresh receiver [20,40]) 40 = some 3 :=
  fresh_name receiver [20,40] (by decide) 1 40 (by decide_cbv)
example : bind (receiver ++ fresh receiver [20,40]) state 40 = some ⟨state,3⟩ :=
  fresh_bind receiver [20,40] (by decide) state 1 40 (by decide_cbv)

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : name (receiver ++ fresh receiver [20,40]) 40 = some 2 := by decide_cbv

-- Byte truncation loses a distinction even when the exact ordinals are valid.
/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : byteRank 0 ≠ byteRank 256 := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : name (List.range 257) 256 = some (byteRank 256) := by decide_cbv

-- A bare rank under the wrong registry does not identify the supplied action.
/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : ownsAt receiver donorContext.ordinal 20 = true := by decide_cbv

-- A merge must rebind through the native action, not copy a donor coordinate.
/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : rebind donor merged donorContext 20 = some donorContext := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : ownsAt merged donorContext.ordinal 20 = true := by decide_cbv

-- Aliased ownership claims and duplicate-action registries are not accepted.
/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : (ownsAt [10,20] 0 10 && ownsAt [10,20] 0 20) = true := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : bind [20,20] state 20 = some donorContext := by decide_cbv

-- There is space for the first new action but not the whole batch: no partial
-- publication is permitted, even though incremental insertion could fit one.
/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : (extend 3 true receiver [20,40]).1 = [10,30,20] := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : (extend 4 false receiver [20,40]).1 = [10,30,20,40] := by decide_cbv

-- The upper addressability bound is exclusive: rank 3 does not fit bound 3.
/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : (extend 3 true receiver [20,40]).2 = .ok [2,3] := by decide_cbv

-- Previously assigned receiver names stay fixed after the merge.
/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : name merged 10 = name donor 10 := by decide_cbv

end ActionRegistryControls
