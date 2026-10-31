import VerumExternalReplay.ReferenceChecker

/-! Mathematical meaning of the reference checker's exact universe arithmetic and maximum helpers.
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

/-- The supremum of a finite collection of universe levels; the empty maximum is zero. -/
def Level.denoteMax (environment : String → Nat) : List Level → Nat
  | [] => 0
  | head :: tail => Nat.max (head.denote environment) (Level.denoteMax environment tail)

theorem Level.denoteMax_append (environment : String → Nat) (left right : List Level) :
    Level.denoteMax environment (left ++ right) =
      Nat.max (Level.denoteMax environment left) (Level.denoteMax environment right) := by
  induction left with
  | nil => simp [Level.denoteMax]
  | cons head tail ih => simp [Level.denoteMax, ih, Nat.max_assoc]

theorem Level.denoteMax_reverse (environment : String → Nat) (levels : List Level) :
    Level.denoteMax environment levels.reverse = Level.denoteMax environment levels := by
  induction levels with
  | nil => rfl
  | cons head tail ih =>
    simp [List.reverse_cons, Level.denoteMax_append, Level.denoteMax, ih, Nat.max_comm]

/-- This unfolds the function used by the checker, with no abstraction assumption. -/
theorem Level.flattenMax_exact (environment : String → Nat) (level : Level) :
    Level.denoteMax environment level.flattenMax = level.denote environment := by
  induction level with
  | max left right ihLeft ihRight =>
    simp [Level.flattenMax, Level.denoteMax_append, ihLeft, ihRight, Level.denote]
  | concrete _ | var _ | succ _ _ | offset _ _ _ =>
    simp [Level.flattenMax, Level.denoteMax]

theorem Level.flattenMax_wireValid (level : Level) (valid : level.WireValid) :
    ∀ member ∈ level.flattenMax, member.WireValid := by
  induction level with
  | max left right ihLeft ihRight =>
    intro member found
    simp only [Level.flattenMax, List.mem_append] at found
    exact found.elim (ihLeft valid.1 member) (ihRight valid.2 member)
  | concrete _ | var _ | succ _ _ | offset _ _ _ =>
    simpa only [Level.flattenMax, List.mem_singleton, forall_eq] using valid

private theorem denote_foldl_max (environment : String → Nat) (levels : List Level) (initial : Level) :
    (levels.foldl (fun acc level => Level.max level acc) initial).denote environment =
      Nat.max (Level.denoteMax environment levels) (initial.denote environment) := by
  induction levels generalizing initial with
  | nil => simp [Level.denoteMax]
  | cons head tail ih =>
    simp only [List.foldl_cons, ih, Level.denoteMax, Level.denote]
    exact (Nat.max_left_comm _ _ _).trans (Nat.max_assoc _ _ _).symm

private theorem wireValid_foldl_max (levels : List Level) (initial : Level)
    (initialValid : initial.WireValid) (valid : ∀ level ∈ levels, level.WireValid) :
    (levels.foldl (fun acc level => Level.max level acc) initial).WireValid := by
  induction levels generalizing initial with
  | nil => exact initialValid
  | cons head tail ih =>
    apply ih (.max head initial) ⟨valid head (by simp), initialValid⟩
    intro level found
    exact valid level (by simp [found])

private theorem assemble_list_exact (environment : String → Nat) (levels : List Level) :
    (match levels.reverse with
      | [] => Level.concrete 0
      | last :: rest => rest.foldl (fun acc level => Level.max level acc) last).denote environment =
      Level.denoteMax environment levels := by
  rw [← Level.denoteMax_reverse environment levels]
  cases levels.reverse with
  | nil => rfl
  | cons head tail => simp [denote_foldl_max, Level.denoteMax, Nat.max_comm]

private theorem assemble_list_wireValid (levels : List Level)
    (valid : ∀ level ∈ levels, level.WireValid) :
    (match levels.reverse with
      | [] => Level.concrete 0
      | last :: rest => rest.foldl (fun acc level => Level.max level acc) last).WireValid := by
  have reversed : ∀ level ∈ levels.reverse, level.WireValid := by simpa using valid
  cases reversedLevels : levels.reverse with
  | nil => exact Nat.zero_le _
  | cons head tail =>
    rw [reversedLevels] at reversed
    apply wireValid_foldl_max tail head (reversed head (by simp))
    intro level found
    exact reversed level (by simp [found])

/-- Zero removal and right-associated assembly preserve the whole finite maximum. -/
theorem Level.assembleMax_exact (environment : String → Nat) (concrete : Option Nat) (symbolic : List Level) :
    (Level.assembleMax concrete symbolic).denote environment =
      Nat.max (concrete.getD 0) (Level.denoteMax environment symbolic) := by
  let kept := match concrete with
    | some 0 => if symbolic.isEmpty then some 0 else none
    | other => other
  let all := (match kept with | some n => [Level.concrete n] | none => []) ++ symbolic
  have assembled : (Level.assembleMax concrete symbolic).denote environment = Level.denoteMax environment all :=
    assemble_list_exact environment all
  rw [assembled]
  dsimp only [all, kept]
  cases concrete with
  | none => simp
  | some n =>
    cases n with
    | zero => cases symbolic <;> simp [Level.denoteMax, Level.denote]
    | succ n => simp [Level.denoteMax, Level.denote]

theorem Level.assembleMax_wireValid (concrete : Option Nat) (symbolic : List Level)
    (concreteValid : ∀ n ∈ concrete, n ≤ Level.wireMax)
    (symbolicValid : ∀ level ∈ symbolic, level.WireValid) :
    (Level.assembleMax concrete symbolic).WireValid := by
  unfold Level.assembleMax
  apply assemble_list_wireValid
  cases concrete with
  | none => simpa using symbolicValid
  | some n =>
    have bound := concreteValid n (by simp)
    cases n with
    | zero =>
      cases symbolic with
      | nil => simp [Level.WireValid]
      | cons head tail => simpa using symbolicValid
    | succ n => simpa [Level.WireValid] using And.intro bound symbolicValid

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
/-- info: 'VerumKernel.Level.flattenMax_exact' depends on axioms: [propext] -/
#guard_msgs in
#print axioms Level.flattenMax_exact
/-- info: 'VerumKernel.Level.flattenMax_wireValid' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms Level.flattenMax_wireValid
/-- info: 'VerumKernel.Level.assembleMax_exact' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms Level.assembleMax_exact
/-- info: 'VerumKernel.Level.assembleMax_wireValid' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms Level.assembleMax_wireValid
end VerumKernel
