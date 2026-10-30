import VerumExternalReplay.CanonicalProduct

/-! Exact finite row ratios. Factors and the model supplying them are fixed.
This mathematical adapter proves no Rust implementation or native certificate.
Local row accumulation never publishes global state; only close may do so.
Absolute mass is a separate, unchanged input to every absolute-mass consumer. -/
namespace RowRatioFold
open ExactMass
set_option warningAsError true

structure Positive where
  value : Fraction
  positive : 0 < value.numerator
structure Factor where
  c : Positive
  r : Positive

def initial (L : Positive) : Fraction := CanonicalProduct.inverse L.value L.positive

def step (b : Fraction) (p : Factor) : Fraction :=
  mul (mul b p.c.value) (CanonicalProduct.inverse p.r.value p.r.positive)

def fold (b : Fraction) : List Factor → Fraction
  | [] => b
  | p :: ps => fold (step b p) ps

def product (f : Factor → Positive) : List Factor → Positive
  | [] => ⟨one, by decide⟩
  | p :: ps =>
    ⟨mul (f p).value (product f ps).value,
     Nat.mul_pos (f p).positive (product f ps).positive⟩

def closed (b : Fraction) (ps : List Factor) : Fraction :=
  mul (mul b (product Factor.c ps).value)
    (CanonicalProduct.inverse (product Factor.r ps).value (product Factor.r ps).positive)

def quotient (L : Positive) (ps : List Factor) : Fraction :=
  mul (product Factor.c ps).value
    (CanonicalProduct.inverse
      (mul L.value (product Factor.r ps).value)
      (Nat.mul_pos L.positive (product Factor.r ps).positive))

theorem closed_nil (b : Fraction) : Equal (closed b []) b := by
  simp [closed, product, CanonicalProduct.inverse, Equal, mul, one]

theorem closed_step (b : Fraction) (p : Factor) (ps : List Factor) :
    Equal (closed (step b p) ps) (closed b (p :: ps)) := by
  unfold Equal closed step
  simp only [product, CanonicalProduct.inverse, mul]
  ac_rfl

/-- General finite-fold law, with no length bound or numerical cutoff. -/
theorem fold_closed (b : Fraction) (ps : List Factor) :
    Equal (fold b ps) (closed b ps) := by
  induction ps generalizing b with
  | nil => exact eq_symm (closed_nil b)
  | cons p ps ih => exact eq_trans (ih (step b p)) (closed_step b p ps)

/-- B₀=1/L and B'=B*c/r give B=C/(L*R) for arbitrary positive rational factors. -/
theorem ratio_invariant (L : Positive) (ps : List Factor) :
    Equal (fold (initial L) ps) (quotient L ps) := by
  apply eq_trans (fold_closed (initial L) ps)
  unfold Equal closed initial quotient
  simp only [CanonicalProduct.inverse, mul]
  ac_rfl

/-- All factors are evaluated from one fixed model; no per-factor model update is substituted. -/
theorem frozen_model_invariant (factor : M → X → Factor) (model : M)
    (L : Positive) (observations : List X) :
    Equal (fold (initial L) (observations.map (factor model)))
      (quotient L (observations.map (factor model))) :=
  ratio_invariant L (observations.map (factor model))

theorem every_prefix (L : Positive) (ps : List Factor) (n : Nat) :
    Equal (fold (initial L) (ps.take n)) (quotient L (ps.take n)) :=
  ratio_invariant L (ps.take n)

/-- Resuming the same row keeps its accumulator; it does not restart 1/L. -/
theorem fold_append (b : Fraction) (xs ys : List Factor) :
    fold b (xs ++ ys) = fold (fold b xs) ys := by
  induction xs generalizing b with
  | nil => rfl
  | cons p ps ih => exact ih (step b p)

theorem concatenation_invariant (L : Positive) (xs ys : List Factor) :
    Equal (fold (fold (initial L) xs) ys) (quotient L (xs ++ ys)) := by
  rw [← fold_append]
  exact ratio_invariant L (xs ++ ys)

theorem step_congr (p : Factor) {a b : Fraction} (same : Equal a b) :
    Equal (step a p) (step b p) :=
  mul_congr_right _ (mul_congr_right _ same)

theorem fold_congr (ps : List Factor) {a b : Fraction} (same : Equal a b) :
    Equal (fold a ps) (fold b ps) := by
  induction ps generalizing a b with
  | nil => exact same
  | cons p ps ih => exact ih (step_congr p same)

/-- Rational equality, not numerator/denominator spelling, makes a factor neutral. -/
theorem neutral_step (b : Fraction) (p : Factor) (neutral : Equal p.c.value p.r.value) :
    Equal (step b p) b := by
  have scaled := congrArg (fun n => b.numerator * b.denominator * n) neutral
  simpa only [Equal, step, mul, CanonicalProduct.inverse, Nat.mul_assoc,
    Nat.mul_comm, Nat.mul_left_comm] using scaled

theorem neutral_insertion (b : Fraction) (xs ys : List Factor) (p : Factor)
    (neutral : Equal p.c.value p.r.value) :
    Equal (fold b (xs ++ p :: ys)) (fold b (xs ++ ys)) := by
  rw [fold_append, fold_append]
  exact fold_congr ys (neutral_step (fold b xs) p neutral)

theorem fold_positive (b : Fraction) (positive : 0 < b.numerator) (ps : List Factor) :
    0 < (fold b ps).numerator := by
  induction ps generalizing b with
  | nil => exact positive
  | cons p ps ih =>
    exact ih (step b p) (Nat.mul_pos (Nat.mul_pos positive p.c.positive) p.r.value.positive)

theorem row_positive (L : Positive) (ps : List Factor) :
    0 < (fold (initial L) ps).numerator :=
  fold_positive (initial L) L.value.positive ps

/-- A whole exactly-neutral group cancels, including reciprocal non-unit factors. -/
theorem neutral_group (b : Fraction) (ps : List Factor)
    (neutral : Equal (product Factor.c ps).value (product Factor.r ps).value) :
    Equal (fold b ps) b :=
  eq_trans (fold_closed b ps)
    (neutral_step b ⟨product Factor.c ps, product Factor.r ps⟩ neutral)

/-- Global q and absolute masses are distinct from an optional open-row accumulator. -/
structure State (Q A : Type) where
  q : Q
  row : Option Fraction
  absolute : A

def beginRow (L : Positive) (q : Q) (absolute : A) : State Q A :=
  ⟨q, some (initial L), absolute⟩

def advance (s : State Q A) (p : Factor) : State Q A :=
  { s with row := s.row.map (fun b => step b p) }

def advanceMany (s : State Q A) : List Factor → State Q A
  | [] => s
  | p :: ps => advanceMany (advance s p) ps

def close (publish : Q → Fraction → Q) (s : State Q A) : State Q A :=
  match s.row with
  | none => s
  | some b => { s with q := publish s.q b, row := none }

theorem advance_many_state (s : State Q A) (ps : List Factor) :
    advanceMany s ps = { s with row := s.row.map (fun b => fold b ps) } := by
  induction ps generalizing s with
  | nil => cases s with | mk q row absolute => cases row <;> rfl
  | cons p ps ih =>
    rw [advanceMany, ih]
    cases s with | mk q row absolute => cases row <;> rfl

theorem local_steps_preserve_global (s : State Q A) (ps : List Factor) :
    (advanceMany s ps).q = s.q := by rw [advance_many_state]

theorem open_row_invariant (L : Positive) (q : Q) (a : A) (ps : List Factor) :
    (advanceMany (beginRow L q a) ps).row = some (fold (initial L) ps) := by
  rw [advance_many_state]; rfl

/-- Publication receives the completed row, while the row's factors saw frozen q. -/
theorem close_publishes_completed_row (publish : Q → Fraction → Q)
    (L : Positive) (q : Q) (a : A) (ps : List Factor) :
    close publish (advanceMany (beginRow L q a) ps) =
      ⟨publish q (fold (initial L) ps), none, a⟩ := by
  rw [advance_many_state]; rfl

theorem closed_steps_do_not_publish (q : Q) (a : A) (ps : List Factor) :
    advanceMany (⟨q, none, a⟩ : State Q A) ps = ⟨q, none, a⟩ := by
  rw [advance_many_state]; rfl

theorem close_once (publish : Q → Fraction → Q) (s : State Q A) :
    close publish (close publish s) = close publish s := by
  cases s with | mk q row absolute => cases row <;> rfl

/-- Arbitrary absolute-mass consumers keep their exact input, even at publication. -/
theorem absolute_consumer_unchanged (observe : A → O) (publish : Q → Fraction → Q)
    (s : State Q A) (ps : List Factor) :
    observe (close publish (advanceMany s ps)).absolute = observe s.absolute := by
  rw [advance_many_state]
  cases s with | mk q row absolute => cases row <;> rfl

-- Kernel-checked dependency rosters: no assumed theorem axioms or admitted proofs.
/-- info: 'RowRatioFold.closed_nil' depends on axioms: [propext] -/
#guard_msgs in
#print axioms closed_nil
/-- info: 'RowRatioFold.closed_step' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms closed_step
/-- info: 'RowRatioFold.fold_closed' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms fold_closed
/-- info: 'RowRatioFold.ratio_invariant' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ratio_invariant
/-- info: 'RowRatioFold.every_prefix' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms every_prefix
/-- info: 'RowRatioFold.fold_append' does not depend on any axioms -/
#guard_msgs in
#print axioms fold_append
/-- info: 'RowRatioFold.concatenation_invariant' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms concatenation_invariant
/-- info: 'RowRatioFold.step_congr' depends on axioms: [propext] -/
#guard_msgs in
#print axioms step_congr
/-- info: 'RowRatioFold.fold_congr' depends on axioms: [propext] -/
#guard_msgs in
#print axioms fold_congr
/-- info: 'RowRatioFold.neutral_step' depends on axioms: [propext] -/
#guard_msgs in
#print axioms neutral_step
/-- info: 'RowRatioFold.neutral_insertion' depends on axioms: [propext] -/
#guard_msgs in
#print axioms neutral_insertion
/-- info: 'RowRatioFold.fold_positive' does not depend on any axioms -/
#guard_msgs in
#print axioms fold_positive
/-- info: 'RowRatioFold.row_positive' does not depend on any axioms -/
#guard_msgs in
#print axioms row_positive
/-- info: 'RowRatioFold.neutral_group' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms neutral_group
/-- info: 'RowRatioFold.advance_many_state' does not depend on any axioms -/
#guard_msgs in
#print axioms advance_many_state
/-- info: 'RowRatioFold.local_steps_preserve_global' does not depend on any axioms -/
#guard_msgs in
#print axioms local_steps_preserve_global
/-- info: 'RowRatioFold.open_row_invariant' does not depend on any axioms -/
#guard_msgs in
#print axioms open_row_invariant
/-- info: 'RowRatioFold.close_publishes_completed_row' does not depend on any axioms -/
#guard_msgs in
#print axioms close_publishes_completed_row
/-- info: 'RowRatioFold.closed_steps_do_not_publish' does not depend on any axioms -/
#guard_msgs in
#print axioms closed_steps_do_not_publish
/-- info: 'RowRatioFold.close_once' does not depend on any axioms -/
#guard_msgs in
#print axioms close_once
/-- info: 'RowRatioFold.absolute_consumer_unchanged' does not depend on any axioms -/
#guard_msgs in
#print axioms absolute_consumer_unchanged

/-- info: 'RowRatioFold.frozen_model_invariant' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms frozen_model_invariant

end RowRatioFold
