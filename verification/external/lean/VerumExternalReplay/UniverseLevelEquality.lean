import VerumExternalReplay.UniverseSemantics

/-! Denotation of the executable universe comparison in ReferenceChecker.
The wire bounds literals; a valuation may assign arbitrary naturals to names. -/
namespace VerumKernel
set_option warningAsError true

def Level.Atom.eval (ρ : String → Nat) (a : Level.Atom) : Nat :=
  (a.parameter.map ρ).getD 0 + a.offset

private def evalAtoms (ρ : String → Nat) : List Level.Atom → Nat
  | [] => 0
  | a :: rest => max (a.eval ρ) (evalAtoms ρ rest)

private theorem eval_zero (a : Level.Atom) : a.eval (fun _ => 0) = a.offset := by
  obtain ⟨p, k⟩ := a
  cases p <;> simp [Level.Atom.eval]

private theorem eval_append (ρ : String → Nat) (left right : List Level.Atom) :
    evalAtoms ρ (left ++ right) = max (evalAtoms ρ left) (evalAtoms ρ right) := by
  induction left with
  | nil => simp [evalAtoms]
  | cons a tail ih => simp [evalAtoms, ih, Nat.max_assoc]

theorem Level.atomsFrom_exact (ρ : String → Nat) (l : Level) (shift : Nat) :
    evalAtoms ρ (l.atomsFrom shift) = l.denote ρ + shift := by
  induction l generalizing shift with
  | concrete n => simp [Level.atomsFrom, evalAtoms, Level.Atom.eval, Level.denote, Nat.add_comm]
  | var name => simp [Level.atomsFrom, evalAtoms, Level.Atom.eval, Level.denote]
  | succ base ih => simp only [Level.atomsFrom, Level.denote, ih]; omega
  | offset base amount ih => simp only [Level.atomsFrom, Level.denote, ih]; omega
  | max left right ihLeft ihRight =>
    exact (eval_append ρ _ _).trans (by rw [ihLeft, ihRight]; exact Nat.add_max_add_right _ _ _)

private theorem atoms_nonempty (l : Level) (shift : Nat) : l.atomsFrom shift ≠ [] := by
  induction l generalizing shift with
  | concrete _ | var _ => simp [Level.atomsFrom]
  | succ _ ih | offset _ _ ih => exact ih _
  | max a b left _ => simp [Level.atomsFrom, left]

private theorem member_le (ρ : String → Nat) {a : Level.Atom} {xs : List Level.Atom}
    (found : a ∈ xs) : a.eval ρ ≤ evalAtoms ρ xs := by
  induction xs with
  | nil => cases found
  | cons x rest ih =>
    rcases List.mem_cons.mp found with same | later
    · subst x; exact Nat.le_max_left _ _
    · exact Nat.le_trans (ih later) (Nat.le_max_right _ _)

private theorem atoms_le (ρ : String → Nat) (xs : List Level.Atom) (n : Nat)
    (bound : ∀ a ∈ xs, a.eval ρ ≤ n) : evalAtoms ρ xs ≤ n := by
  induction xs with
  | nil => exact Nat.zero_le _
  | cons a rest ih =>
    exact Nat.max_le.mpr ⟨bound a (by simp), ih (fun x hx => bound x (by simp [hx]))⟩

private theorem maximum_attained (ρ : String → Nat) (xs : List Level.Atom) (nonempty : xs ≠ []) :
    ∃ b ∈ xs, evalAtoms ρ xs = b.eval ρ := by
  induction xs with
  | nil => contradiction
  | cons a rest ih =>
    by_cases top : evalAtoms ρ rest ≤ a.eval ρ
    · exact ⟨a, by simp, Nat.max_eq_left top⟩
    · have more : rest ≠ [] := by intro empty; simp [empty, evalAtoms] at top
      obtain ⟨b, member, value⟩ := ih more
      exact ⟨b, List.mem_cons_of_mem _ member, (Nat.max_eq_right (by omega)).trans value⟩

private theorem cover_sound {a b : Level.Atom} (covered : a.coveredBy b = true)
    (ρ : String → Nat) : a.eval ρ ≤ b.eval ρ := by
  obtain ⟨offset, h | h⟩ := of_decide_eq_true covered
  · simp only [Level.Atom.eval, h, Option.map_none, Option.getD_none, Nat.zero_add]
    omega
  · simp only [Level.Atom.eval, h]
    omega

private theorem covered_iff (a : Level.Atom) (bs : List Level.Atom) (nonempty : bs ≠ []) :
    bs.any (fun b => a.coveredBy b) = true ↔ ∀ ρ, a.eval ρ ≤ evalAtoms ρ bs := by
  constructor
  · intro accepted ρ
    obtain ⟨b, found, covered⟩ := List.any_eq_true.mp accepted
    exact Nat.le_trans (cover_sound covered ρ) (member_le ρ found)
  · intro bound
    obtain ⟨parameter, k⟩ := a
    cases parameter with
    | none =>
      obtain ⟨b, found, value⟩ := maximum_attained (fun _ => 0) bs nonempty
      have le := bound (fun _ => 0)
      rw [value, eval_zero, eval_zero] at le
      exact List.any_eq_true.mpr ⟨b, found, decide_eq_true ⟨le, Or.inl rfl⟩⟩
    | some x =>
      let peak := evalAtoms (fun _ => 0) bs + 1
      let ρ : String → Nat := fun y => if y = x then peak else 0
      obtain ⟨b, found, value⟩ := maximum_attained ρ bs nonempty
      have le := bound ρ
      rw [value] at le
      have atX : (Level.Atom.mk (some x) k).eval ρ = peak + k := by simp [Level.Atom.eval, ρ]
      rw [atX] at le
      have offsetBound := member_le (fun _ => 0) found
      rw [eval_zero] at offsetBound
      have same : b.parameter = some x := by
        apply Decidable.byContradiction
        intro different
        have low : (b.parameter.map ρ).getD 0 = 0 := by
          cases hp : b.parameter with
          | none => rfl
          | some y =>
            have ne : y ≠ x := by intro eq; subst y; exact different hp
            simp [ρ, ne]
        have lowValue : b.eval ρ = b.offset := by simp [Level.Atom.eval, low]
        rw [lowValue] at le
        dsimp only [peak] at le
        omega
      have highValue : b.eval ρ = peak + b.offset := by simp [Level.Atom.eval, same, ρ]
      rw [highValue] at le
      exact List.any_eq_true.mpr ⟨b, found, decide_eq_true ⟨Nat.le_of_add_le_add_left le, Or.inr same.symm⟩⟩

/-- The checker accepts universal dominance exactly. The reverse implication uses
    zero for constants and a separating valuation for each named parameter. -/
theorem Level.leEverywhere_iff (a b : Level) :
    a.leEverywhere b = true ↔ ∀ ρ, a.denote ρ ≤ b.denote ρ := by
  simp only [Level.leEverywhere, List.all_eq_true]
  constructor
  · intro covered ρ
    have bound := atoms_le ρ (a.atomsFrom 0) (evalAtoms ρ (b.atomsFrom 0))
      (fun term mem => (covered_iff term _ (atoms_nonempty b 0)).mp (covered term mem) ρ)
    simpa only [Level.atomsFrom_exact, Nat.add_zero] using bound
  · intro bound term mem
    apply (covered_iff term _ (atoms_nonempty b 0)).mpr
    intro ρ
    have lhs := member_le ρ mem
    simp only [Level.atomsFrom_exact, Nat.add_zero] at lhs ⊢
    exact Nat.le_trans lhs (bound ρ)

theorem Level.eq_iff_denote (a b : Level) :
    Level.eq a b = true ↔ ∀ ρ, a.denote ρ = b.denote ρ := by
  simp only [Level.eq, Bool.and_eq_true, Level.leEverywhere_iff]
  exact ⟨fun ⟨ab, ba⟩ ρ => Nat.le_antisymm (ab ρ) (ba ρ),
    fun eq => ⟨fun ρ => Nat.le_of_eq (eq ρ), fun ρ => Nat.le_of_eq (eq ρ).symm⟩⟩

theorem Level.maxCanonical_exact (ρ : String → Nat) (a b : Level) :
    (Level.maxCanonical a b).denote ρ = Nat.max (a.denote ρ) (b.denote ρ) := by
  unfold Level.maxCanonical
  split
  · rename_i le; exact (Nat.max_eq_right ((Level.leEverywhere_iff a b).mp le ρ)).symm
  · split
    · rename_i le; exact (Nat.max_eq_left ((Level.leEverywhere_iff b a).mp le ρ)).symm
    · rfl

theorem Level.normalize_wireValid (l : Level) (valid : l.WireValid) :
    l.normalize.WireValid := by
  induction l with
  | concrete _ | var _ => exact valid
  | succ base ih => exact Level.addOffset_wireValid _ 1 (ih valid) (by decide)
  | offset base k ih => exact Level.addOffset_wireValid _ k (ih valid.1) valid.2
  | max a b ihA ihB =>
    have left := ihA valid.1
    have right := ihB valid.2
    simp only [Level.normalize, Level.maxCanonical]
    split
    · exact right
    · split
      · exact left
      · exact ⟨left, right⟩

theorem Level.normalize_exact (ρ : String → Nat) (l : Level) (valid : l.WireValid) :
    l.normalize.denote ρ = l.denote ρ := by
  induction l with
  | concrete _ | var _ => rfl
  | succ base ih =>
    rw [Level.normalize, Level.addOffset_exact ρ _ _ (base.normalize_wireValid valid), ih valid]
    rfl
  | offset base k ih =>
    rw [Level.normalize, Level.addOffset_exact ρ _ _ (base.normalize_wireValid valid.1), ih valid.1]
    rfl
  | max a b ihA ihB =>
    rw [Level.normalize, Level.maxCanonical_exact, ihA valid.1, ihB valid.2]
    rfl

theorem Level.shiftedBy_exact (ρ : String → Nat) (l : Level) (k : Nat) (valid : l.WireValid) :
    (l.shiftedBy k).denote ρ = l.denote ρ + k := by
  unfold Level.shiftedBy
  split
  · subst k; rfl
  · rw [Level.addOffset_exact _ _ _ (l.normalize_wireValid valid), l.normalize_exact ρ valid]

-- A missing parameter can agree at zero and at a large value of another
-- parameter. Raising that parameter alone separates the two expressions.
private def wideV : Level := .succ (.offset (.var "v") Level.wireMax)
private def wideUV : Level := .max (.offset (.var "u") Level.wireMax) wideV
private def raiseOnly (name x : String) : Nat := if x = name then 2 * Level.wireMax else 0

example : wideUV.denote (fun _ => 0) = wideV.denote (fun _ => 0) := by decide
example : wideUV.denote (raiseOnly "v") = wideV.denote (raiseOnly "v") := by decide
example : wideUV.denote (raiseOnly "u") ≠ wideV.denote (raiseOnly "u") := by decide
example : Level.eq wideUV wideV = false := by decide

-- A missing constant floor is visible at zero even if a large parameter hides it.
private def wideFloor : Level := .max (.offset (.concrete Level.wireMax) 2) wideV
example : wideFloor.denote (fun _ => 0) ≠ wideV.denote (fun _ => 0) := by decide
example : wideFloor.denote (raiseOnly "v") = wideV.denote (raiseOnly "v") := by decide
example : Level.eq wideFloor wideV = false := by decide

/-- info: 'VerumKernel.Level.leEverywhere_iff' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms Level.leEverywhere_iff

/-- info: 'VerumKernel.Level.eq_iff_denote' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms Level.eq_iff_denote

/-- info: 'VerumKernel.Level.maxCanonical_exact' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms Level.maxCanonical_exact

/-- info: 'VerumKernel.Level.normalize_exact' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms Level.normalize_exact

/-- info: 'VerumKernel.Level.shiftedBy_exact' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms Level.shiftedBy_exact

end VerumKernel
