import Init

namespace Verum.WordGrade
set_option warningAsError true

structure Env where
  stateCount : Int64
  obsCount : Int64
  budget : Int64

def valid (e : Env) : Bool :=
  e.obsCount ≥ 1 && e.budget ≥ 0 && e.stateCount == e.obsCount + e.budget

def silentChain (e : Env) (k : Int64) : Bool :=
  k ≥ 0 && e.budget ≥ 0 && k ≤ e.budget

def originalGrade (n : Int64) (e : Env) : Bool :=
  n ≥ 0 && e.budget ≥ 0 && !silentChain e (n + 1)

def iso (e f : Env) : Bool :=
  valid e && valid f && e.obsCount == f.obsCount && e.budget == f.budget &&
    e.stateCount == f.stateCount

/-- Verbatim arithmetic of the MSFS R-12 executable predicates at the boundary. -/
theorem counterexample :
    let e : Env := ⟨1, 1, 0⟩
    let n := (9223372036854775807 : Int64)
    (n ≥ 0 && valid e && valid e && originalGrade n e && iso e e) = true ∧
      (originalGrade (n + 1) e && originalGrade n e) = false := by decide

/-- The threshold form of the original grade over mathematical natural numbers. -/
theorem nat_grade (n b : Nat) : (¬ n + 1 ≤ b) ↔ b ≤ n :=
  Nat.not_lt

/-- An order comparison uses only values already representable in the source. -/
def grade (n : Int64) (e : Env) : Prop := 0 ≤ e.budget.toInt ∧ e.budget.toInt ≤ n.toInt

theorem monotone_and_invariant {n m : Int64} {e f : Env} (h : grade n e)
    (order : n.toInt ≤ m.toInt) (same : e.budget = f.budget) : grade m e ∧ grade n f := by
  exact ⟨⟨h.1, Int.le_trans h.2 order⟩,
    Eq.subst (motive := fun v : Int64 => 0 ≤ v.toInt ∧ v.toInt ≤ n.toInt) same h⟩

#print axioms counterexample
#print axioms nat_grade
#print axioms monotone_and_invariant
end Verum.WordGrade
