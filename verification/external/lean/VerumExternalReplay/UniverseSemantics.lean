import VerumExternalReplay.ReferenceChecker

/-! Mathematical meaning of the reference checker's exact successor and compact offset operations.
This does not assert correctness of the entire level normalizer or Rust checker. -/
namespace VerumKernel
set_option warningAsError true

def Level.denote (environment : String → Nat) : Level → Nat
  | .concrete n => n
  | .var name => environment name
  | .succ base => base.denote environment + 1
  | .max a b => Nat.max (a.denote environment) (b.denote environment)
  | .offset base amount => base.denote environment + amount

/-- The certificate wire admits bounded literals, including offset amounts.
The denoted universe value itself has no fixed-width bound. -/
def Level.WireValid : Level → Prop
  | .concrete n => n ≤ Level.wireMax
  | .var _ => True
  | .succ base => base.WireValid
  | .max a b => a.WireValid ∧ b.WireValid
  | .offset base amount => base.WireValid ∧ amount ≤ Level.wireMax

theorem Level.offsetNode_exact (environment : String → Nat) (level : Level)
    (amount : Nat) :
    (Level.offsetNode level amount).denote environment = level.denote environment + amount := by
  cases amount with
  | zero => rfl
  | succ n => cases n <;> rfl

theorem Level.offsetNode_wireValid (level : Level) (amount : Nat)
    (valid : level.WireValid) (bound : amount ≤ Level.wireMax) :
    (Level.offsetNode level amount).WireValid := by
  cases amount with
  | zero => exact valid
  | succ n =>
    cases n with
    | zero => exact valid
    | succ n => exact ⟨valid, bound⟩

/-- Every branch of the actual compact offset implementation preserves its
natural-number meaning. The bounded-constant premise is essential because
Lean's raw datatype can express literals that the wire decoder refuses. -/
theorem Level.addOffset_exact (environment : String → Nat) (level : Level)
    (amount : Nat) (valid : level.WireValid) :
    (level.addOffset amount).denote environment = level.denote environment + amount := by
  induction level generalizing amount with
  | concrete n =>
    simp only [Level.addOffset]
    split
    · subst amount; rfl
    · split
      · rfl
      · rw [Level.offsetNode_exact]
        simp only [Level.denote]
        have bound : n ≤ Level.wireMax := valid
        omega
  | var name =>
    simp only [Level.addOffset]
    split
    · subst amount; rfl
    · exact Level.offsetNode_exact environment (.var name) amount
  | succ base ih =>
    simp only [Level.addOffset]
    split
    · subst amount; rfl
    · split
      · rw [ih _ valid]
        simp only [Level.denote]
        omega
      · exact Level.offsetNode_exact environment (.succ base) amount
  | max a b _ _ =>
    simp only [Level.addOffset]
    split
    · subst amount; rfl
    · exact Level.offsetNode_exact environment (.max a b) amount
  | offset base prior ih =>
    simp only [Level.addOffset]
    split
    · subst amount; rfl
    · split
      · rw [ih _ valid.1]
        simp only [Level.denote]
        omega
      · exact Level.offsetNode_exact environment (.offset base prior) amount

/-- Exact arithmetic also preserves the admissible representation, rather
than manufacturing out-of-range literals for subsequent checker operations. -/
theorem Level.addOffset_wireValid (level : Level) (amount : Nat)
    (valid : level.WireValid) (bound : amount ≤ Level.wireMax) :
    (level.addOffset amount).WireValid := by
  induction level generalizing amount with
  | concrete n =>
    simp only [Level.addOffset]
    split
    · exact valid
    · split
      · assumption
      · apply Level.offsetNode_wireValid
        · exact Nat.le_refl _
        · omega
  | var name =>
    simp only [Level.addOffset]
    split
    · exact valid
    · exact Level.offsetNode_wireValid _ _ valid bound
  | succ base ih =>
    simp only [Level.addOffset]
    split
    · exact valid
    · split
      · exact ih _ valid (by assumption)
      · exact Level.offsetNode_wireValid _ _ valid bound
  | max a b _ _ =>
    simp only [Level.addOffset]
    split
    · exact valid
    · exact Level.offsetNode_wireValid _ _ valid bound
  | offset base prior ih =>
    simp only [Level.addOffset]
    split
    · exact valid
    · split
      · exact ih _ valid.1 (by assumption)
      · exact Level.offsetNode_wireValid _ _ valid bound

/-- Prefix extraction budgets are bounded representations, even when the
represented universe value exceeds the wire width. -/
theorem Level.succDepth_bounded (level : Level) : level.succDepth ≤ Level.wireMax := by
  cases level with
  | succ base | offset base amount => exact Nat.min_le_left _ _
  | concrete n | var name | max a b => exact Nat.zero_le _

/-- Every advertised removable prefix is part of the mathematical value. -/
theorem Level.succDepth_le_denote (environment : String → Nat) (level : Level) :
    level.succDepth ≤ level.denote environment := by
  induction level with
  | concrete n | var name | max a b _ _ => exact Nat.zero_le _
  | succ base ih =>
    have capped := Nat.min_le_right Level.wireMax (1 + base.succDepth)
    simp only [Level.succDepth, Level.denote]
    omega
  | offset base amount ih =>
    have capped := Nat.min_le_right Level.wireMax (amount + base.succDepth)
    simp only [Level.succDepth, Level.denote]
    omega

/-- Stripping preserves admissible syntax. Availability is unnecessary for
this representation law; exact subtraction below additionally requires it. -/
theorem Level.stripSucc_wireValid (level : Level) (count : Nat)
    (valid : level.WireValid) : (level.stripSucc count).WireValid := by
  induction level generalizing count with
  | concrete n | var name | max a b _ _ =>
    simp only [Level.stripSucc]
    split <;> exact valid
  | succ base ih =>
    simp only [Level.stripSucc]
    split
    · exact valid
    · exact ih _ valid
  | offset base amount ih =>
    simp only [Level.stripSucc]
    split
    · exact valid
    · split
      · exact ih _ valid.1
      · apply Level.addOffset_wireValid _ _ valid.1
        have bound := valid.2
        omega

/-- A requested prefix must be available in the actual syntactic prefix
budget. The conclusion retains the count, so truncated subtraction cannot
hide removing too much. -/
theorem Level.stripSucc_exact (environment : String → Nat) (level : Level) (count : Nat)
    (valid : level.WireValid) (available : count ≤ level.succDepth) :
    (level.stripSucc count).denote environment + count = level.denote environment := by
  induction level generalizing count with
  | concrete n | var name | max a b _ _ =>
    have zero : count = 0 := Nat.eq_zero_of_le_zero available
    subst count
    rfl
  | succ base ih =>
    simp only [Level.stripSucc]
    split
    · subst count; rfl
    · have depthBound : count ≤ 1 + base.succDepth :=
        Nat.le_trans available (Nat.min_le_right _ _)
      have law := ih (count - 1) valid (by omega)
      simp only [Level.denote]
      omega
  | offset base amount ih =>
    simp only [Level.stripSucc]
    split
    · subst count; rfl
    · split
      · have depthBound : count ≤ amount + base.succDepth :=
          Nat.le_trans available (Nat.min_le_right _ _)
        have law := ih (count - amount) valid.1 (by omega)
        simp only [Level.denote]
        omega
      · rw [Level.addOffset_exact _ _ _ valid.1]
        simp only [Level.denote]
        omega

/-- Factoring an available prefix out and restoring it with the actual
compact-offset operation preserves the mathematical universe value. -/
theorem Level.stripSucc_restore (environment : String → Nat) (level : Level) (count : Nat)
    (valid : level.WireValid) (available : count ≤ level.succDepth) :
    ((level.stripSucc count).addOffset count).denote environment = level.denote environment := by
  rw [Level.addOffset_exact _ _ _ (Level.stripSucc_wireValid _ _ valid)]
  exact Level.stripSucc_exact _ _ _ valid available

/-- Common offsets cannot conceal a difference between mathematical levels. -/
theorem Level.addOffset_cancel (environment : String → Nat) (a b : Level) (amount : Nat)
    (validA : a.WireValid) (validB : b.WireValid)
    (equal : (a.addOffset amount).denote environment = (b.addOffset amount).denote environment) :
    a.denote environment = b.denote environment := by
  rw [Level.addOffset_exact _ _ _ validA, Level.addOffset_exact _ _ _ validB] at equal
  exact Nat.add_right_cancel equal

theorem Level.successor_exact (environment : String → Nat) (level : Level) :
    level.successor.denote environment = level.denote environment + 1 := by
  cases level with
  | concrete n =>
    simp only [Level.successor]
    split <;> rfl
  | var name | succ base | max a b | offset base amount => rfl

theorem Level.successor_strict (environment : String → Nat) (level : Level) :
    level.denote environment < level.successor.denote environment := by
  rw [Level.successor_exact]
  exact Nat.lt_succ_self _

/-- info: 'VerumKernel.Level.successor_exact' depends on axioms: [propext] -/
#guard_msgs in
#print axioms Level.successor_exact
/-- info: 'VerumKernel.Level.successor_strict' depends on axioms: [propext] -/
#guard_msgs in
#print axioms Level.successor_strict
/-- info: 'VerumKernel.Level.addOffset_exact' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms Level.addOffset_exact
/-- info: 'VerumKernel.Level.addOffset_wireValid' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms Level.addOffset_wireValid
/-- info: 'VerumKernel.Level.addOffset_cancel' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms Level.addOffset_cancel
/-- info: 'VerumKernel.Level.succDepth_bounded' depends on axioms: [propext] -/
#guard_msgs in
#print axioms Level.succDepth_bounded
/-- info: 'VerumKernel.Level.succDepth_le_denote' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms Level.succDepth_le_denote
/-- info: 'VerumKernel.Level.stripSucc_wireValid' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms Level.stripSucc_wireValid
/-- info: 'VerumKernel.Level.stripSucc_exact' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms Level.stripSucc_exact
/-- info: 'VerumKernel.Level.stripSucc_restore' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms Level.stripSucc_restore
end VerumKernel
