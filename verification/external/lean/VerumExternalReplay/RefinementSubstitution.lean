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

/- Locally indexed variables make lexical scope explicit. A caller argument
   has no locally bound variables; inserting it under a binder cannot capture
   its free names. Native replay checks the actual named AST after projecting
   its lexical bindings to these indices. -/
namespace Scoped

inductive Term : Nat → Type where
  | var {n} : String → Term n
  | bound {n} : Fin n → Term n
  | int {n} : Int → Term n
  | add {n} : Term n → Term n → Term n
  | sub {n} : Term n → Term n → Term n
  | letE {n} : Term n → Term (n + 1) → Term n

def extend (value : Value) (locals : Fin n → Value) : Fin (n + 1) → Value :=
  Fin.cases value locals

def eval (algebra : Algebra Value) (env : String → Value)
    (locals : Fin n → Value) : Term n → Value
  | .var name => env name
  | .bound index => locals index
  | .int value => algebra.literal value
  | .add lhs rhs => algebra.add (eval algebra env locals lhs) (eval algebra env locals rhs)
  | .sub lhs rhs => algebra.sub (eval algebra env locals lhs) (eval algebra env locals rhs)
  | .letE initial body =>
    eval algebra env (extend (eval algebra env locals initial) locals) body

def lift (mapping : Fin n → Fin m) : Fin (n + 1) → Fin (m + 1) :=
  Fin.cases 0 (fun index => (mapping index).succ)

def rename (mapping : Fin n → Fin m) : Term n → Term m
  | .var name => .var name
  | .bound index => .bound (mapping index)
  | .int value => .int value
  | .add lhs rhs => .add (rename mapping lhs) (rename mapping rhs)
  | .sub lhs rhs => .sub (rename mapping lhs) (rename mapping rhs)
  | .letE initial body => .letE (rename mapping initial) (rename (lift mapping) body)

theorem eval_rename (term : Term n) (algebra : Algebra Value) (env : String → Value) :
    ∀ {m} (mapping : Fin n → Fin m) (locals : Fin m → Value),
      eval algebra env locals (rename mapping term) =
      eval algebra env (fun index => locals (mapping index)) term := by
  induction term with
  | var name => intros; rfl
  | bound index => intros; rfl
  | int value => intros; rfl
  | add lhs rhs left right =>
    intros; simp only [rename, eval, left, right]
  | sub lhs rhs left right =>
    intros; simp only [rename, eval, left, right]
  | letE initial body first rest =>
    intro m mapping locals
    simp only [rename, eval, first, rest]
    congr 1
    funext index
    refine Fin.cases ?_ (fun index => ?_) index <;> rfl

def insert (term : Term 0) : Term n := rename Fin.elim0 term

theorem eval_insert (term : Term 0) (algebra : Algebra Value) (env : String → Value)
    (locals : Fin n → Value) :
    eval algebra env locals (insert term) = eval algebra env Fin.elim0 term := by
  rw [insert, eval_rename]
  congr 1
  funext index
  exact Fin.elim0 index

def substitute (replacement : String → Term 0) : Term n → Term n
  | .var name => insert (replacement name)
  | .bound index => .bound index
  | .int value => .int value
  | .add lhs rhs => .add (substitute replacement lhs) (substitute replacement rhs)
  | .sub lhs rhs => .sub (substitute replacement lhs) (substitute replacement rhs)
  | .letE initial body => .letE (substitute replacement initial) (substitute replacement body)

theorem sound (term : Term n) (replacement : String → Term 0)
    (algebra : Algebra Value) (env : String → Value) :
    ∀ (locals : Fin n → Value),
      eval algebra env locals (substitute replacement term) =
      eval algebra (fun name => eval algebra env Fin.elim0 (replacement name)) locals term := by
  induction term with
  | var name => intro locals; exact eval_insert _ _ _ _
  | bound index => intros; rfl
  | int value => intros; rfl
  | add lhs rhs left right => intros; simp only [substitute, eval, left, right]
  | sub lhs rhs left right => intros; simp only [substitute, eval, left, right]
  | letE initial body first rest => intros; simp only [substitute, eval, first, rest]

#print axioms eval_rename
#print axioms eval_insert
#print axioms sound
end Scoped
end Verum.RefinementSubstitution
