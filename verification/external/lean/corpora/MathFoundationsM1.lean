import MathFoundations.Metatheory.DAGResolutionPHP
import MathFoundations.Metatheory.BoundedArithmeticS21
import MathFoundations.Metatheory.ProofSystemsEFIPS

namespace Verum.CorpusResolution
open MathFoundations.Metatheory.DAGResolutionPHP
set_option warningAsError true

private theorem lookup_eq {α : Type} (steps : List α) (idx : Nat) :
    stepAt? steps idx = steps[idx]? := by
  induction steps generalizing idx with
  | nil => rfl
  | cons x xs ih =>
    cases idx with
    | zero => rfl
    | succ i => exact ih i

/-- Validate one resolution step, including each nonpivot literal from both parents. -/
def checkStep {n : Nat} (steps : Array (ResStep n)) (idx : Nat) : Bool :=
  match steps[idx]? with
  | none => false
  | some (.axStep _) => true
  | some (.resolve p q i j clause) =>
    decide (p < idx) && (decide (q < idx) &&
      match steps[p]?, steps[q]? with
      | some left, some right =>
        clauseSubsetExceptZA left.clause clause ⟨i, j, .pos⟩ &&
        clauseSubsetExceptZA right.clause clause ⟨i, j, .neg⟩
      | _, _ => false)

theorem checkStep_iff {n : Nat} (dag : ResDAG n) (idx : Nat) :
    checkStep dag.steps.toArray idx = true ↔ dag.ValidStep idx := by
  simp only [checkStep, ResDAG.ValidStep, List.getElem?_toArray, ← lookup_eq]
  cases s : stepAt? dag.steps idx with
  | none => simp
  | some st =>
    cases st with
    | axStep _ => simp
    | resolve p q i j clause =>
      cases hp : stepAt? dag.steps p <;> cases hq : stepAt? dag.steps q <;>
        simp [hp, hq]

/-- Index once and reuse that index at every edge of the derivation. -/
def check {n : Nat} (dag : ResDAG n) : Bool :=
  let steps := dag.steps.toArray
  decide (0 < steps.size) && (Nat.allLTTR steps.size (fun idx _ => checkStep steps idx) &&
    match steps[steps.size - 1]? with
    | some last => last.clause.isEmpty
    | none => false)

theorem check_iff {n : Nat} (dag : ResDAG n) : check dag = true ↔ dag.ValidRefutation := by
  simp only [check, ResDAG.ValidRefutation, List.size_toArray, Bool.and_eq_true,
    decide_eq_true_eq, Nat.allLTTR_eq_true, checkStep_iff,
    List.getElem?_toArray, ← lookup_eq]
  cases h : stepAt? dag.steps (dag.steps.length - 1) <;> simp

theorem checked_lower_bound {n : Nat} {dag : ResDAG n} (h : check dag = true) :
    2 ^ (n / 1000) ≤ dag.size :=
  haken_dag_resolution_php_exp_lower_bound_za n dag ((check_iff dag).mp h)

example : check php1_dag = true := by decide
example : check php2_dag = true := by decide
example : check (⟨[]⟩ : ResDAG 1) = false := by decide
example : check (⟨php1_dag.steps.take 4⟩ : ResDAG 1) = false := by decide
-- A self dependency at the final node, a forward dependency, and a lost nonpivot literal.
example : check (⟨php1_dag.steps.set 4 (.resolve 1 4 1 0 [])⟩ : ResDAG 1) = false := by decide
example : check (⟨php1_dag.steps.set 3 (.resolve 0 4 0 0 [])⟩ : ResDAG 1) = false := by decide
example : check (⟨php1_dag.steps.set 3 (.resolve 0 2 0 0 [])⟩ : ResDAG 1) = false := by decide

/-- info: 'Verum.CorpusResolution.check_iff' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms check_iff

/-- info: 'Verum.CorpusResolution.checked_lower_bound' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms checked_lower_bound

/-- info: 'MathFoundations.Metatheory.BoundedArithmeticS21.buss_witnessing' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms MathFoundations.Metatheory.BoundedArithmeticS21.buss_witnessing

/-- info: 'MathFoundations.Metatheory.BoundedArithmeticS21.pvTerm_step_poly_bound' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms MathFoundations.Metatheory.BoundedArithmeticS21.pvTerm_step_poly_bound

/-- info: 'MathFoundations.Metatheory.ProofSystemsEFIPS.ips_sound' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms MathFoundations.Metatheory.ProofSystemsEFIPS.ips_sound

/-- info: 'MathFoundations.Metatheory.ProofSystemsEFIPS.ef_to_ips_sim_size' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms MathFoundations.Metatheory.ProofSystemsEFIPS.ef_to_ips_sim_size
end Verum.CorpusResolution
