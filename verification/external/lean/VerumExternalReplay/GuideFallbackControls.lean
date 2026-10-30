import VerumExternalReplay.GuideFallback

/-! Finite examples and causal false controls for the explicit premises.
Negative controls must produce Lean's false-proposition diagnostic. They do not
assert native scheduler correspondence or show that any order learns better. -/
namespace GuideFallbackControls
open ExactMass GuideFallback
set_option warningAsError true

def family : List Nat := [0,1,2]
def C (_ : Nat) : Bool := true
def R (_ : Nat) : Bool := true
def S (key : Nat) : Bool := key == 2

def oldTrace : List Nat := [2,0,1,2]
def newTrace : List Nat := [0,0,1,1,2,2]
def mass (key : Nat) : Fraction :=
  match key with
  | 0 => ⟨1,2,by decide⟩
  | 1 => ⟨1,3,by decide⟩
  | _ => ⟨1,6,by decide⟩

example : oldGuide family C R S = [2] := by decide_cbv
example : newGuide family C R = [0,1,2] := by decide_cbv

theorem oldDrains : Drain (oldGuide family C R S) family oldTrace :=
  Drain.guide 2 (fallback_only [] family)

theorem newDrains : Drain (newGuide family C R) family newTrace :=
  Drain.guide 0 (Drain.fallback 0 (Drain.guide 1 (Drain.fallback 1
    (Drain.guide 2 (Drain.fallback 2 (Drain.done []))))))

example : emit [] oldTrace = [2,0,1] := by decide_cbv
example : emit [] newTrace = [0,1,2] := by decide_cbv
example : (emit [] oldTrace).Perm (emit [] newTrace) :=
  relaxed_permutation family C R S (by decide) oldTrace newTrace oldDrains newDrains
example : Equal (sum ((emit [] oldTrace).map mass)) one := by decide_cbv
example : Equal (sum ((emit [] newTrace).map mass)) one := by decide_cbv
example : Equal (sum ((emit [] oldTrace).map mass)) (sum ((emit [] newTrace).map mass)) :=
  relaxed_mass family C R S (by decide) oldTrace newTrace mass oldDrains newDrains
example : weighted mass oldTrace = [(2,mass 2),(0,mass 0),(1,mass 1)] := by decide_cbv
example : emit ([] : List Nat) [] = [] := by decide_cbv
example : emit ([] : List Nat) [4,4,4] = [4] := by decide_cbv

-- Exact names retain distinctions that a coarse key would erase.
def names : List (Nat × Nat) := [(4,0),(4,1)]
def lossy := names.eraseDupsBy (fun a b => a.1 == b.1)
example : emit [] names = names := by decide_cbv
example : lossy = [(4,0)] := by decide_cbv

-- Every finite prefix of this repeating trace contains only key 2. It cannot
-- represent a drained fallback containing key 0; no scheduler is modeled here.
def starved (steps : Nat) : List Nat := List.replicate steps 2

theorem starved_missing (steps : Nat) : 0 ∉ emit [] (starved steps) := by
  simp [emit_mem, starved]

theorem starved_not_drained (steps : Nat) :
    ¬ Drain (oldGuide family C R S) family (starved steps) := by
  intro h
  apply starved_missing steps
  exact (complete_set (old_guide_subset family C R S) h 0).mpr (by decide)

-- Removing dedup duplicates a name and charges its mass twice.
/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : newTrace.Nodup := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : Equal (sum (newTrace.map mass)) one := by decide_cbv

-- Lossy dedup incorrectly merges two distinct complete names.
/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : (4,1) ∈ lossy := by decide_cbv

-- Visiting only the guide, even repeatedly, does not cover the fallback.
/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : 0 ∈ emit [] (starved 100) := by decide_cbv

-- Set/mass preservation does not preserve or prefer emission order.
/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : emit [] oldTrace = emit [] newTrace := by decide_cbv

-- Distinct names cannot receive a new rank-dependent mass after reordering.
/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : (2,mass 0) ∈ weighted mass oldTrace := by decide_cbv

-- Without the subset premise a guide can inject a foreign name.
/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : 99 ∉ emit [] (99 :: family) := by decide_cbv

-- A family containing duplicate names is not a multiset preserved by dedup.
/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : Equal (sum ((emit [] [0,0]).map mass)) (sum ([0,0].map mass)) := by decide_cbv

end GuideFallbackControls
