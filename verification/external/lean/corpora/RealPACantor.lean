import MathFoundations.Metatheory.RealArithmeticPA
import VerumExternalReplay.GodelCantor

/-! Numeralwise representation of Verum's pairing function inside the corpus
Hilbert calculus. The proof constructs derivations from first-order equality
and Robinson arithmetic. It needs neither a completeness theorem nor an
assumption that every true closed sentence is derivable. -/
namespace Verum.RealPACantor
open MathFoundations.Metatheory.RealPA
open Verum.GodelCantor (pair twice_triangle)
set_option warningAsError true

private abbrev EqD (Ax : ArithForm → Prop) (a b : ArithTerm) :=
  PADerives Ax (.eqTerm a b)
variable {Ax : ArithForm → Prop} {a b c d : ArithTerm}

@[simp] private theorem subst_shift (k : Nat) (s t : ArithTerm) :
    substTermAt k s (shiftTerm k t) = t := by
  induction t with
  | var i =>
    by_cases h : i < k
    · simp [shiftTerm, shiftVar, substTermAt, h]
    · have lt : ¬ i + 1 < k := by omega
      have ne : i + 1 ≠ k := by omega
      simp [shiftTerm, shiftVar, substTermAt, h, lt, ne]
  | zero => rfl
  | succ _ ih => simp [shiftTerm, substTermAt, ih]
  | add _ _ ih₁ ih₂ | mul _ _ ih₁ ih₂ => simp [shiftTerm, substTermAt, ih₁, ih₂]

@[simp] private theorem subst_num (k : Nat) (s : ArithTerm) (n : Nat) :
    substTermAt k s (numeral n) = numeral n := by
  induction n <;> simp_all [numeral, substTermAt]

@[simp] private theorem shift_num (k n : Nat) : shiftTerm k (numeral n) = numeral n := by
  induction n <;> simp_all [numeral, shiftTerm]

private theorem ident (a : ArithTerm) : EqD Ax a a := .logAx 0 (.eqRefl a)

private theorem transport (h : EqD Ax a b) (φ : ArithForm)
    (p : PADerives Ax (substForm0 a φ)) : PADerives Ax (substForm0 b φ) :=
  .mp (.mp (.logAx 0 (.eqLeibniz a b φ)) h) p

private theorem flip (h : EqD Ax a b) : EqD Ax b a := by
  let φ := ArithForm.eqTerm (.var 0) (shiftTerm 0 a)
  simpa [φ, substForm0, substFormAt, substTermAt] using
    transport h φ (by simpa [φ, substForm0, substFormAt, substTermAt] using ident (Ax := Ax) a)

private theorem chain (h : EqD Ax a b) (j : EqD Ax b c) : EqD Ax a c := by
  let φ := ArithForm.eqTerm (shiftTerm 0 a) (.var 0)
  simpa [φ, substForm0, substFormAt, substTermAt] using
    transport j φ (by simpa [φ, substForm0, substFormAt, substTermAt] using h)

private theorem congrTerm (h : EqD Ax a b) (t : ArithTerm) :
    EqD Ax (substTermAt 0 a t) (substTermAt 0 b t) := by
  let φ := ArithForm.eqTerm (shiftTerm 0 (substTermAt 0 a t)) t
  simpa [φ, substForm0, substFormAt] using
    transport h φ (by simpa [φ, substForm0, substFormAt] using ident (Ax := Ax) (substTermAt 0 a t))

private theorem succCong (h : EqD Ax a b) : EqD Ax (.succ a) (.succ b) := by
  simpa [substTermAt] using congrTerm h (.succ (.var 0))

private theorem addCong (h : EqD Ax a b) (j : EqD Ax c d) : EqD Ax (.add a c) (.add b d) := by
  simpa [substTermAt] using chain
    (congrTerm h (.add (.var 0) (shiftTerm 0 c)))
    (by simpa [substTermAt] using congrTerm j (.add (shiftTerm 0 b) (.var 0)))

private theorem mulCong (h : EqD Ax a b) (j : EqD Ax c d) : EqD Ax (.mul a c) (.mul b d) := by
  simpa [substTermAt] using chain
    (congrTerm h (.mul (.var 0) (shiftTerm 0 c)))
    (by simpa [substTermAt] using congrTerm j (.mul (shiftTerm 0 b) (.var 0)))

private theorem inst {φ : ArithForm} (h : PADerives Ax (.allForm φ)) (t : ArithTerm) :
    PADerives Ax (substForm0 t φ) := .mp (.logAx 0 (.allElim φ t)) h

private theorem inst₂ {φ : ArithForm} (h : PADerives Ax (.allForm (.allForm φ))) (m n : Nat) :
    PADerives Ax (substForm0 (numeral n) (substFormAt 1 (shiftTerm 0 (numeral m)) φ)) :=
  inst (inst h (numeral m)) (numeral n)

private theorem addNum (m n : Nat) :
    EqD Ax (.add (numeral m) (numeral n)) (numeral (m + n)) := by
  induction n with
  | zero => exact inst (.paAx 0 .q4) (numeral m)
  | succ n ih =>
    apply chain _ (succCong ih)
    simpa [numeral, substForm0, substFormAt, substTermAt] using
      inst₂ (Ax := Ax) (.paAx 0 .q5) m n

private theorem mulNum (m n : Nat) :
    EqD Ax (.mul (numeral m) (numeral n)) (numeral (m * n)) := by
  induction n with
  | zero => exact inst (.paAx 0 .q6) (numeral m)
  | succ n ih =>
    rw [Nat.mul_succ]
    apply chain _ (chain (addCong ih (ident (numeral m))) (addNum (m*n) m))
    simpa [numeral, substForm0, substFormAt, substTermAt] using
      inst₂ (Ax := Ax) (.paAx 0 .q7) m n

/-- Simultaneously instantiate the free variables of a polynomial by numerals. -/
def ground (env : Nat → Nat) : ArithTerm → ArithTerm
  | .var i => numeral (env i)
  | .zero => .zero
  | .succ t => .succ (ground env t)
  | .add t u => .add (ground env t) (ground env u)
  | .mul t u => .mul (ground env t) (ground env u)

/-- A Hilbert derivation normalizing any ground arithmetic term. -/
theorem normalize (env : Nat → Nat) (t : ArithTerm) :
    PADerives Ax (.eqTerm (ground env t) (numeral (evalTerm env t))) := by
  induction t with
  | var i => exact ident _
  | zero => exact ident _
  | succ _ ih => exact succCong ih
  | add _ _ ht hu => exact chain (addCong ht hu) (addNum _ _)
  | mul _ _ ht hu => exact chain (mulCong ht hu) (mulNum _ _)

private theorem succNeZero (n : Nat) :
    PADerives Ax (negForm (.eqTerm (numeral (n+1)) .zero)) :=
  inst (.paAx 0 .q1) (numeral n)

private theorem numeralNe (m n : Nat) (hne : m ≠ n) :
    PADerives Ax (negForm (.eqTerm (numeral m) (numeral n))) := by
  induction m generalizing n Ax with
  | zero =>
    cases n with
    | zero => exact False.elim (hne rfl)
    | succ n =>
      apply pad_deduction
      exact .mp (succNeZero n) (flip (.hyp (Or.inr rfl)))
  | succ m ih =>
    cases n with
    | zero => exact succNeZero m
    | succ n =>
      apply pad_deduction
      apply PADerives.mp (ih n (fun eq => hne (congrArg Nat.succ eq)))
      apply PADerives.mp _ (.hyp (Or.inr rfl))
      simpa [numeral, substForm0, substFormAt, substTermAt] using
        inst₂ (Ax := ExtAx Ax (.eqTerm (numeral (m+1)) (numeral (n+1)))) (.paAx 0 .q2) m n

private theorem eqGround (env : Nat → Nat) (t u : ArithTerm) (h : evalTerm env t = evalTerm env u) :
    EqD Ax (ground env t) (ground env u) := by
  exact chain (normalize env t) (h ▸ flip (normalize env u))

private theorem neGround (env : Nat → Nat) (t u : ArithTerm) (h : evalTerm env t ≠ evalTerm env u) :
    PADerives Ax (negForm (.eqTerm (ground env t) (ground env u))) := by
  apply pad_deduction
  exact .mp (numeralNe _ _ h)
    (chain (flip (normalize env t)) (chain (.hyp (Or.inr rfl)) (normalize env u)))

/-- The quantifier-free Cantor graph in the actual language of PA. -/
def graphLeft (z : ArithTerm) : ArithTerm := .mul (numeral 2) z
def graphRight (a b : ArithTerm) : ArithTerm :=
  .add (.mul (.add a b) (.succ (.add a b))) (.mul (numeral 2) b)
def pairFormula (a b z : ArithTerm) : ArithForm := .eqTerm (graphLeft z) (graphRight a b)

theorem graph_semantics (env : Nat → Nat) (a b z : ArithTerm) :
    SatisfiesN env (pairFormula a b z) ↔ evalTerm env z = pair (evalTerm env a) (evalTerm env b) := by
  simp only [pairFormula, SatisfiesN, graphLeft, graphRight, evalTerm, evalTerm_numeral, pair]
  rw [← twice_triangle, ← Nat.mul_add]
  exact ⟨Nat.eq_of_mul_eq_mul_left (by decide : 0 < 2), congrArg (2 * ·)⟩

private def inputs (m n z : Nat) : Nat → Nat
  | 0 => m
  | 1 => n
  | _ => z

theorem prove_pair (m n z : Nat) (h : z = pair m n) :
    PADerives EmptyAx (pairFormula (numeral m) (numeral n) (numeral z)) :=
  eqGround (inputs m n z) (graphLeft (.var 2)) (graphRight (.var 0) (.var 1))
    ((graph_semantics (inputs m n z) (.var 0) (.var 1) (.var 2)).mpr h)

theorem refute_pair (m n z : Nat) (h : z ≠ pair m n) :
    PADerives EmptyAx (negForm (pairFormula (numeral m) (numeral n) (numeral z))) :=
  neGround (inputs m n z) (graphLeft (.var 2)) (graphRight (.var 0) (.var 1))
    (fun eq => h ((graph_semantics (inputs m n z) (.var 0) (.var 1) (.var 2)).mp eq))

theorem numeral_graph_exact (m n z : Nat) :
    PADerives EmptyAx (pairFormula (numeral m) (numeral n) (numeral z)) ↔ z = pair m n := by
  constructor
  · intro derivation
    by_cases h : z = pair m n
    · exact h
    · exact False.elim (realPA_consistent (.mp (refute_pair m n z h) derivation))
  · exact prove_pair m n z

/-- Every successful finite-word encoding supplies a positive PA instance. -/
theorem prove_word_pair {m n z : Nat} (h : GodelCantor.pairWord m n = some z) :
    PADerives EmptyAx (pairFormula (numeral m) (numeral n) (numeral z)) := by
  rw [GodelCantor.pairWord_exact] at h
  split at h
  · exact prove_pair m n z (Option.some.inj h).symm
  · contradiction

/-- info: 'Verum.RealPACantor.normalize' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms normalize

/-- info: 'Verum.RealPACantor.graph_semantics' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms graph_semantics

/-- info: 'Verum.RealPACantor.prove_pair' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms prove_pair

/-- info: 'Verum.RealPACantor.refute_pair' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms refute_pair

/-- info: 'Verum.RealPACantor.numeral_graph_exact' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms numeral_graph_exact

/-- info: 'Verum.RealPACantor.prove_word_pair' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms prove_word_pair
end Verum.RealPACantor
