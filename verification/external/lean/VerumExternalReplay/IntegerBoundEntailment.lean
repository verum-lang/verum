import Std

/- The arithmetic premises used by refinement_entailment::BoundImplication.
   Bounds and values here are mathematical integers; the source adapter admits
   only exactly represented Int literals. No arithmetic is assumed on runtime
   values other than the six primitive comparisons. -/
namespace Verum.IntegerBoundEntailment

inductive Relation where
  | eq | ne | lt | le | gt | ge
  deriving DecidableEq

def Holds (relation : Relation) (bound value : Int) : Prop :=
  match relation with
  | .eq => value = bound
  | .ne => value ≠ bound
  | .lt => value < bound
  | .le => value ≤ bound
  | .gt => value > bound
  | .ge => value ≥ bound

def Accepts (source : Relation) (a : Int) (target : Relation) (b : Int) : Prop :=
  match source, target with
  | .eq, .eq | .ne, .ne => a = b
  | .eq, .ne => a ≠ b
  | .eq, .lt => a < b
  | .eq, .le => a ≤ b
  | .eq, .gt => a > b
  | .eq, .ge => a ≥ b
  | .gt, .gt | .ge, .ge => a ≥ b
  | .gt, .ge => a ≥ b ∨ (a - b).natAbs = 1
  | .ge, .gt => a > b
  | .lt, .lt | .le, .le => a ≤ b
  | .lt, .le => a ≤ b ∨ (a - b).natAbs = 1
  | .le, .lt => a < b
  | .gt, .ne => a ≥ b
  | .ge, .ne => a > b
  | .lt, .ne => a ≤ b
  | .le, .ne => a < b
  | _, _ => False

theorem sound (source target : Relation) (a b value : Int)
    (accepted : Accepts source a target b) (holds : Holds source a value) :
    Holds target b value := by
  cases source <;> cases target <;>
    simp only [Accepts, Holds, Int.natAbs_eq_iff] at * <;> omega

#print axioms sound
end Verum.IntegerBoundEntailment
