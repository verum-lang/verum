import VerumExternalReplay.WorkDrain

/-! A checked adapter establishes the progress premise of the existing driver.
Raw callbacks keep their state and effects even when the guard yields or rejects
increased work. Terminal callback results take precedence over progress checks.
Bounded native rank coordinates have an exact natural-valued order embedding;
reversing an unbounded natural ordinal would not be well-founded.
-/
namespace GuardedDrain
open WorkDrain
set_option warningAsError true

inductive Error (E : Type) where
  | callback (reason : E)
  | workIncreased
  deriving DecidableEq, Repr

def classify (work rank remaining nextRank : Nat) : Status E → Status (Error E)
  | .pending =>
      if work < remaining then .refused .workIncreased
      else if remaining = work ∧ rank ≤ nextRank then .yielded
      else .pending
  | .yielded => .yielded
  | .ready => .ready
  | .refused e => .refused (.callback e)

theorem pending_iff (work rank remaining nextRank : Nat) (status : Status E) :
    classify work rank remaining nextRank status = .pending ↔
      status = .pending ∧ (remaining < work ∨ remaining = work ∧ nextRank < rank) := by
  cases status with
  | pending =>
    simp only [classify]
    split
    · rename_i increased
      constructor
      · intro bad; cases bad
      · rintro ⟨_,paid | ⟨equal,_⟩⟩ <;> exfalso <;> omega
    · rename_i nonincreasing
      split
      · rename_i stuck
        constructor
        · intro bad; cases bad
        · rintro ⟨_,paid | ⟨equal,lower⟩⟩ <;> exfalso <;> omega
      · rename_i progressing
        constructor
        · intro _
          refine ⟨True.intro,?_⟩
          by_cases paid : remaining < work
          · exact Or.inl paid
          · have equal : remaining = work := by clear progressing; omega
            have notHigher : ¬ rank ≤ nextRank := fun higher => progressing ⟨equal,higher⟩
            clear progressing
            exact Or.inr ⟨equal,by omega⟩
        · intro _; rfl
  | yielded | ready | refused _ => simp [classify]

theorem ready_iff (work rank remaining nextRank : Nat) (status : Status E) :
    classify work rank remaining nextRank status = .ready ↔ status = .ready := by
  cases status <;> simp only [classify]
  · split
    · simp
    · split <;> simp
  all_goals simp

theorem callback_refusal_iff (work rank remaining nextRank : Nat) (status : Status E) (e : E) :
    classify work rank remaining nextRank status = .refused (.callback e) ↔ status = .refused e := by
  cases status <;> simp only [classify]
  · split
    · simp
    · split <;> simp
  all_goals simp

theorem increased_pending_refused (work rank remaining nextRank : Nat) (increased : work < remaining) :
    classify (E := E) work rank remaining nextRank .pending = .refused .workIncreased := by
  simp [classify,increased]

theorem unchanged_rank_yields (work rank nextRank : Nat) (stuck : rank ≤ nextRank) :
    classify (E := E) work rank work nextRank .pending = .yielded := by
  simp [classify,stuck]

def checked (rank : S → Nat) (step : S → Nat → Observation S E A)
    (s : S) (work : Nat) : Observation S (Error E) A :=
  let result := step s work
  ⟨result.state,result.remaining,
    classify work (rank s) result.remaining (rank result.state) result.status,result.events⟩

theorem checked_retains (rank : S → Nat) (step : S → Nat → Observation S E A) (s : S) (work : Nat) :
    (checked rank step s work).state = (step s work).state ∧
    (checked rank step s work).remaining = (step s work).remaining ∧
    (checked rank step s work).events = (step s work).events := ⟨rfl,rfl,rfl⟩

/-- Progress follows from the guard. Meter monotonicity remains a separate
callback premise: the guard reports a bad meter without repairing its value. -/
def machine (rank : S → Nat) (step : S → Nat → Observation S E A)
    (metered : ∀ s work, (step s work).remaining ≤ work) : Machine S (Error E) A where
  step := checked rank step
  rank := rank
  bounded := metered
  progress s work _ pending sameWork := by
    have progress := (pending_iff work (rank s) (step s work).remaining
      (rank (step s work).state) (step s work).status).mp pending
    change (step s work).remaining = work at sameWork
    rcases progress.2 with paid | ⟨_,lower⟩
    · omega
    · exact lower

private theorem pair_lt (base a b x y : Nat) (xBound : x < base) (yBound : y < base) :
    a*base+x < b*base+y ↔ a < b ∨ a = b ∧ x < y := by
  constructor
  · intro h
    by_cases lower : a < b
    · exact Or.inl lower
    · by_cases same : a = b
      · refine Or.inr ⟨same,?_⟩
        rw [same] at h
        omega
      · have step : (b+1)*base ≤ a*base := Nat.mul_le_mul_right base (by omega)
        simp only [Nat.add_mul,Nat.one_mul] at step
        omega
  · rintro (lower | ⟨rfl,lower⟩)
    · have step : (a+1)*base ≤ b*base := Nat.mul_le_mul_right base (by omega)
      simp only [Nat.add_mul,Nat.one_mul] at step
      omega
    · omega

private theorem pair_eq (base a b x y : Nat) (xBound : x < base) (yBound : y < base) :
    a*base+x = b*base+y ↔ a = b ∧ x = y := by
  constructor
  · intro equal
    have notLower : ¬ a < b := by
      intro lower
      have less := (pair_lt base a b x y xBound yBound).mpr (Or.inl lower)
      omega
    have notHigher : ¬ b < a := by
      intro lower
      have less := (pair_lt base b a y x yBound xBound).mpr (Or.inl lower)
      omega
    have same : a = b := by omega
    refine ⟨same,?_⟩
    rw [same] at equal
    omega
  · rintro ⟨rfl,rfl⟩; rfl

/-- Coordinates use their actual finite native bounds, not a sample maximum.
Encoding uses mathematical naturals; no multiplication is added to native code. -/
structure Rank (ordinals levels phases : Nat) where
  ordinal : Fin ordinals
  level : Fin levels
  phase : Fin phases
  deriving DecidableEq

def Rank.value (r : Rank ordinals levels phases) : Nat :=
  ((ordinals-r.ordinal.val)*levels+r.level.val)*phases+r.phase.val

theorem Rank.value_lt_iff (a b : Rank ordinals levels phases) :
    a.value < b.value ↔ b.ordinal.val < a.ordinal.val ∨
      a.ordinal.val = b.ordinal.val ∧ (a.level.val < b.level.val ∨
        a.level.val = b.level.val ∧ a.phase.val < b.phase.val) := by
  simp only [Rank.value]
  rw [pair_lt phases _ _ _ _ a.phase.isLt b.phase.isLt,
    pair_lt levels _ _ _ _ a.level.isLt b.level.isLt,
    pair_eq levels _ _ _ _ a.level.isLt b.level.isLt]
  have aBound := a.ordinal.isLt
  have bBound := b.ordinal.isLt
  constructor
  · rintro ((ord | ⟨ord,level⟩) | ⟨⟨ord,level⟩,phase⟩)
    · exact Or.inl (by omega)
    · exact Or.inr ⟨by omega,Or.inl level⟩
    · exact Or.inr ⟨by omega,Or.inr ⟨level,phase⟩⟩
  · rintro (ord | ⟨ord,level | ⟨level,phase⟩⟩)
    · exact Or.inl (Or.inl (by omega))
    · exact Or.inl (Or.inr ⟨by omega,level⟩)
    · exact Or.inr ⟨⟨by omega,level⟩,phase⟩

theorem Rank.value_injective (a b : Rank ordinals levels phases) : a.value = b.value ↔ a = b := by
  constructor
  · intro same
    obtain ⟨high,phase⟩ := (pair_eq phases _ _ _ _ a.phase.isLt b.phase.isLt).mp same
    obtain ⟨reversed,level⟩ := (pair_eq levels _ _ _ _ a.level.isLt b.level.isLt).mp high
    have aBound := a.ordinal.isLt
    have bBound := b.ordinal.isLt
    have ordinal : a.ordinal.val = b.ordinal.val := by omega
    cases a; cases b
    simp_all only [Fin.val_inj]
  · rintro rfl; rfl

end GuardedDrain
