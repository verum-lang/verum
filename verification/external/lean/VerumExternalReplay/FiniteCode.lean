import VerumExternalReplay.TypedPrograms

/-! Extraction boundary for finite owned code trees. Source operands use append
order; the typed target uses prepend order. Calls are expanded with the existing
proved substitution/splicing operations, preserving every eager instruction.
Missing references, callees and selected outputs denote partial failure. Work
budgets and diagnostic payloads are deliberately not represented by Option.
This is a checked lowering, not a theorem about a foreign machine implementation.
-/
namespace FiniteCode
open TypedPrograms
set_option warningAsError true
universe u

inductive Operand (V : Type u) where
  | input : Nat → Operand V
  | register : Nat → Operand V
  | literal : V → Operand V

inductive Operation (O : Type u) where
  | primitive : O → Operation O
  | call : Nat → Operation O

structure Step (O : Type u) (V : Type u) where
  operation : Operation O
  arguments : List (Operand V)

inductive Code (O : Type u) (V : Type u) where
  | mk : List (Step O V) → Option Nat → List (Code O V) → Code O V

abbrev Row (n : Nat) := List.replicate n ()
abbrev Carrier (V : Type u) (_ : Unit) := V
abbrev Prim (O : Type u) (_ : List Unit) (_ : Unit) := Option O
abbrev Target (O : Type u) (V : Type u) (Γ : List Unit) :=
  Program (Prim O) (Carrier V) Γ ()

def row (xs : List V) : HList (Carrier V) (Row xs.length) :=
  match xs with
  | [] => .nil
  | x :: xs => .cons x (row xs)

def values : HList (Carrier V) Γ → List V
  | .nil => []
  | .cons x xs => x :: values xs

def semantics (apply : O → List V → Option V) :
    Semantics (Prim O) (Carrier V) (Carrier V) where
  literal := fun x => x
  primitive := fun op args => op.bind (fun o => apply o (values args))

def fail : Target O V Γ := .call none .nil (.ret (.reg .here))

@[simp] theorem run_fail (apply : O → List V → Option V) (env : Env (Carrier V) Γ) :
    (fail : Target O V Γ).run (semantics apply) env = none := rfl

def inputs : (n : Nat) → List (Arg (Carrier V) (Row n) ())
  | 0 => []
  | n + 1 => .reg .here :: (inputs n).map Arg.weaken

def resolve (ins regs : List (Arg (Carrier V) Γ ())) :
    Operand V → Option (Arg (Carrier V) Γ ())
  | .input n => ins[n]?
  | .register n => regs[n]?
  | .literal v => some (.literal v)

def arguments (ins regs : List (Arg (Carrier V) Γ ())) :
    List (Operand V) → Option (List (Arg (Carrier V) Γ ()))
  | [] => some []
  | a :: rest => do
      let a ← resolve ins regs a
      let rest ← arguments ins regs rest
      pure (a :: rest)

def steps (library : List ((n : Nat) → Target O V (Row n)))
    (output : Option Nat) (code : List (Step O V))
    (ins regs : List (Arg (Carrier V) Γ ())) : Target O V Γ :=
  match code with
  | [] =>
      match output.bind (fun n => regs[n]?) <|>
          (if output.isNone then regs.getLast? else none) with
      | some a => .ret a
      | none => fail
  | step :: rest =>
      match arguments ins regs step.arguments with
      | none => fail
      | some args =>
          let continuation := steps library output rest
            (ins.map Arg.weaken) (regs.map Arg.weaken ++ [.reg .here])
          match step.operation with
          | .primitive op => .call (some op) (row args) continuation
          | .call n =>
              match library[n]? with
              | none => fail
              | some callee =>
                  ((callee args.length).subst (row args).get).splice continuation

def lower (code : Code O V) (arity : Nat) : Target O V (Row arity) :=
  match code with
  | .mk instructions output library =>
      steps (library.attach.map (fun child => fun n => lower child.val n))
        output instructions (inputs arity) []
termination_by sizeOf code
decreasing_by
  have h := List.sizeOf_lt_of_mem child.property
  simp only [Code.mk.sizeOf_spec]
  omega

def run (apply : O → List V → Option V) (code : Code O V) (xs : List V) : Option V :=
  (lower code xs.length).run (semantics apply) (row xs).get

/-- Only unfolds the checked lowering equations. The final proof is checked by
the kernel; this tactic does not execute generated native code. -/
macro "reduce_finite_code" : tactic => `(tactic|
  (simp only [FiniteCode.run, FiniteCode.lower,
    List.attach_cons, List.attach_nil, List.map_cons, List.map_nil]; rfl))

inductive InputSource where
  | state
  | external : Nat → InputSource

def bind (width : Nat) (slots : List InputSource) (state : V) (external : List V) :
    Option (List V) :=
  if external.length = width then
    slots.mapM (fun slot => match slot with
      | .state => some state
      | .external n => external[n]?)
  else none

theorem bind_wrong_width (width : Nat) (slots : List InputSource) (state : V)
    (external : List V) (h : external.length ≠ width) :
    bind width slots state external = none := by simp [bind, h]

end FiniteCode
