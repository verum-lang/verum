import Std

/- Simultaneous substitution for a binder-free fragment of source refinements.
   Replacements are evaluated in the caller's environment. Operators are
   arbitrary: the result does not assume unbounded runtime arithmetic and also
   applies when values carry checked-arithmetic failures.
   This theorem does not cover AST binders or the compiler's expression walker;
   native replay must separately check its projection into this fragment. -/
namespace Verum.RefinementSubstitution

inductive Term where
  | var : String → Term
  | int : Int → Term
  | add : Term → Term → Term
  | sub : Term → Term → Term

structure Algebra (Value : Type) where
  literal : Int → Value
  add : Value → Value → Value
  sub : Value → Value → Value

def eval (algebra : Algebra Value) (env : String → Value) : Term → Value
  | .var name => env name
  | .int value => algebra.literal value
  | .add lhs rhs => algebra.add (eval algebra env lhs) (eval algebra env rhs)
  | .sub lhs rhs => algebra.sub (eval algebra env lhs) (eval algebra env rhs)

def substitute (replacement : String → Term) : Term → Term
  | .var name => replacement name
  | .int value => .int value
  | .add lhs rhs => .add (substitute replacement lhs) (substitute replacement rhs)
  | .sub lhs rhs => .sub (substitute replacement lhs) (substitute replacement rhs)

theorem sound (term : Term) (replacement : String → Term)
    (algebra : Algebra Value) (env : String → Value) :
    eval algebra env (substitute replacement term) =
    eval algebra (fun name => eval algebra env (replacement name)) term := by
  induction term with
  | var name => rfl
  | int value => rfl
  | add lhs rhs left right => simp only [substitute, eval, left, right]
  | sub lhs rhs left right => simp only [substitute, eval, left, right]

#print axioms sound
end Verum.RefinementSubstitution
