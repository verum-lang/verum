import Std

/- Independent model of scoped VBC type substitution. Pair represents structural
   composition and binder represents a locally quantified type. This proves the
   scope laws of lifting, not automatic correspondence of Rust code, its wire
   decoder, or every TypeRef constructor. Those require separate implementation
   controls. Natural-number indices separate the mathematical law from checked
   finite-width representation. -/
namespace Verum.QuantifiedTypeSubstitution

-- Elementary arithmetic by induction keeps these scope proofs independent of
-- all additional axioms, including propositional extensionality.
private theorem cancel_added_suffix (a b suffix : Nat) (same : a + suffix = b + suffix) : a = b := by
  induction suffix with
  | zero => exact same
  | succ suffix ih => exact ih (Nat.succ.inj same)

private theorem subtract_prefix (offset index : Nat) : (offset + index) - offset = index := by
  induction offset with
  | zero => rw [Nat.zero_add, Nat.sub_zero]
  | succ offset ih =>
      rw [Nat.succ_add, Nat.succ_sub_succ]
      exact ih

inductive Shape where
  | atom (identity : Nat)
  | variable (index : Nat)
  | pair (left right : Shape)
  | binder (count : Nat) (body : Shape)
  deriving DecidableEq

def lift (amount cutoff : Nat) : Shape → Shape
  | .atom identity => .atom identity
  | .variable index => .variable (if index < cutoff then index else index + amount)
  | .pair left right => .pair (lift amount cutoff left) (lift amount cutoff right)
  | .binder count body => .binder count (lift amount (cutoff + count) body)

/-- No occurrence can refer to the newly inserted binder interval. Occurrences
    inside an existing binder are checked in that binder's extended scope. -/
def Gap (lower count : Nat) : Shape → Prop
  | .atom _ => True
  | .variable index => index < lower ∨ lower + count ≤ index
  | .pair left right => Gap lower count left ∧ Gap lower count right
  | .binder bound body => Gap (lower + bound) count body

/-- A free occurrence is identified relative to all enclosing local binders. -/
def FreeAt (free depth : Nat) : Shape → Prop
  | .atom _ => False
  | .variable index => index = depth + free
  | .pair left right => FreeAt free depth left ∨ FreeAt free depth right
  | .binder count body => FreeAt free (depth + count) body

theorem lift_preserves_bound_variable (amount cutoff index : Nat) (bound : index < cutoff) :
    lift amount cutoff (.variable index) = .variable index := by
  unfold lift
  rw [ite_eq_left bound]

theorem lift_preserves_free_identity (amount depth free : Nat) :
    lift amount depth (.variable (depth + free)) = .variable (depth + amount + free) := by
  unfold lift
  rw [ite_eq_right (Nat.not_lt_of_ge (Nat.le_add_right depth free))]
  rw [Nat.add_right_comm depth free amount]

theorem lift_cannot_capture (source : Shape) (amount cutoff : Nat) :
    Gap cutoff amount (lift amount cutoff source) := by
  induction source generalizing cutoff with
  | atom identity => trivial
  | «variable» index =>
      unfold lift
      rcases Nat.lt_or_ge index cutoff with bound | free
      · rw [ite_eq_left bound]
        exact Or.inl bound
      · rw [ite_eq_right (Nat.not_lt_of_ge free)]
        exact Or.inr (Nat.add_le_add_right free amount)
  | pair left right ihLeft ihRight =>
      exact ⟨ihLeft cutoff, ihRight cutoff⟩
  | binder count body ih => exact ih (cutoff + count)

theorem lift_preserves_all_free_occurrences (source : Shape) (amount depth free : Nat) :
    FreeAt free (depth + amount) (lift amount depth source) ↔ FreeAt free depth source := by
  induction source generalizing depth with
  | atom identity => rfl
  | «variable» index =>
      unfold lift
      rcases Nat.lt_or_ge index depth with bound | outside
      · rw [ite_eq_left bound]
        constructor
        · intro same
          have ge := Nat.le_trans (Nat.le_add_right depth amount) (Nat.le_add_right (depth + amount) free)
          exact False.elim ((Nat.not_lt_of_ge ge) (same ▸ bound))
        · intro same
          exact False.elim ((Nat.not_lt_of_ge (Nat.le_add_right depth free)) (same ▸ bound))
      · rw [ite_eq_right (Nat.not_lt_of_ge outside)]
        change index + amount = (depth + amount) + free ↔ index = depth + free
        rw [Nat.add_right_comm depth amount free]
        constructor
        · exact cancel_added_suffix index (depth + free) amount
        · intro same
          exact congrArg (· + amount) same
  | pair left right ihLeft ihRight =>
      constructor
      · intro occurrence
        cases occurrence with
        | inl left => exact Or.inl ((ihLeft depth).mp left)
        | inr right => exact Or.inr ((ihRight depth).mp right)
      · intro occurrence
        cases occurrence with
        | inl left => exact Or.inl ((ihLeft depth).mpr left)
        | inr right => exact Or.inr ((ihRight depth).mpr right)
  | binder count body ih =>
      change FreeAt free ((depth + amount) + count) (lift amount (depth + count) body) ↔ _
      rw [Nat.add_right_comm depth amount count]
      exact ih (depth + count)

abbrev Substitution := Nat → Option Shape

def substitute (bindings : Substitution) (depth : Nat) : Shape → Shape
  | .atom identity => .atom identity
  | .variable index =>
      if index < depth then .variable index
      else match bindings (index - depth) with
        | none => .variable index
        | some replacement => lift depth 0 replacement
  | .pair left right => .pair (substitute bindings depth left) (substitute bindings depth right)
  | .binder count body => .binder count (substitute bindings (depth + count) body)

theorem substitution_preserves_bound_variable (bindings : Substitution) (depth index : Nat)
    (bound : index < depth) : substitute bindings depth (.variable index) = .variable index := by
  unfold substitute
  rw [ite_eq_left bound]

theorem inserted_replacement_is_capture_free (bindings : Substitution) (depth free : Nat)
    (replacement : Shape) (selected : bindings free = some replacement) :
    Gap 0 depth (substitute bindings depth (.variable (depth + free))) := by
  unfold substitute
  rw [ite_eq_right (Nat.not_lt_of_ge (Nat.le_add_right depth free)), subtract_prefix, selected]
  exact lift_cannot_capture replacement depth 0

/-- The representation check returns the exact index or refuses; it never
    reports a wrapped or saturated successful index. -/
def checkedLift (limit index shift : Nat) : Option Nat :=
  if index + shift ≤ limit then some (index + shift) else none

theorem checked_lift_success_is_exact (limit index shift output : Nat)
    (accepted : checkedLift limit index shift = some output) :
    output = index + shift ∧ output ≤ limit := by
  unfold checkedLift at accepted
  split at accepted
  · cases accepted
    exact ⟨rfl, by assumption⟩
  · cases accepted

theorem checked_lift_refuses_overflow (limit index shift : Nat) (overflow : limit < index + shift) :
    checkedLift limit index shift = none := by
  unfold checkedLift
  exact ite_eq_right (Nat.not_le_of_lt overflow)

end Verum.QuantifiedTypeSubstitution
