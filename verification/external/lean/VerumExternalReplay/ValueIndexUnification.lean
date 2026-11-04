import Std

/- Independent value-index fragment. Parameter identities are scope-owned;
   only inference slots are assignable. This does not certify Rust elaboration,
   dependent sorts, domain obligations or callable instantiation. -/
namespace Verum.ValueIndexUnification

inductive Term where
  | parameter (id : Nat)
  | inference (id : Nat)
  | constant (value : Int)
  deriving DecidableEq

abbrev Binding := Nat × Term

def apply (binding : Option Binding) (term : Term) : Term :=
  match binding, term with
  | some (id, replacement), .inference other => if id = other then replacement else term
  | _, _ => term

def unify (left right : Term) : Option (Option Binding) :=
  if left = right then some none else
  match left, right with
  | .inference id, term => some (some (id, term))
  | term, .inference id => some (some (id, term))
  | _, _ => none

theorem parameter_preserved (binding : Option Binding) (id : Nat) :
    apply binding (.parameter id) = .parameter id := by
  cases binding <;> rfl

theorem constant_preserved (binding : Option Binding) (value : Int) :
    apply binding (.constant value) = .constant value := by
  cases binding <;> rfl

theorem sound (left right : Term) (binding : Option Binding)
    (accepted : unify left right = some binding) :
    apply binding left = apply binding right := by
  by_cases same : left = right
  · subst right
    rfl
  · cases left <;> cases right <;>
      simp_all [unify]
    all_goals subst binding; simp_all [apply]

theorem different_parameters_refused (left right : Nat) (distinct : left ≠ right) :
    unify (.parameter left) (.parameter right) = none := by
  simp [unify, distinct]

theorem parameter_constant_refused (id : Nat) (value : Int) :
    unify (.parameter id) (.constant value) = none := by
  simp [unify]

theorem constant_parameter_refused (id : Nat) (value : Int) :
    unify (.constant value) (.parameter id) = none := by
  simp [unify]

theorem bind_slot (id : Nat) (value : Int) :
    unify (.inference id) (.constant value) = some (some (id, .constant value)) := by
  simp [unify]

#print axioms sound
#print axioms parameter_preserved
#print axioms constant_preserved
end Verum.ValueIndexUnification
