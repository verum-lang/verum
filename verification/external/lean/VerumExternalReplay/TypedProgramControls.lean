import VerumExternalReplay.TypedPrograms

/-! Executable controls for the generic structural theorem. These examples
exercise heterogeneous registers, eager failure, sharing and capture avoidance;
the general theorem is in TypedPrograms, not inferred from these examples. -/
namespace TypedProgramControls
open TypedPrograms
set_option warningAsError true

inductive Kind where
  | number | flag

abbrev Value : Kind → Type
  | .number => Nat
  | .flag => Bool

inductive Literal : Kind → Type where
  | number : Nat → Literal .number
  | flag : Bool → Literal .flag

def literalValue : Literal t → Value t
  | .number n => n
  | .flag b => b

inductive Op : List Kind → Kind → Type where
  | add : Op [.number, .number] .number
  | nonzero : Op [.number] .flag
  | require : Op [.flag, .number] .number
  | fail : Op [] .number

def target : Semantics Op Literal Value where
  literal := literalValue
  primitive := fun op args => match op, args with
    | .add, .cons x (.cons y .nil) => some (x + y)
    | .nonzero, .cons x .nil => some (x != 0)
    | .require, .cons b (.cons n .nil) => if b then some n else none
    | .fail, .nil => none

inductive Source : List Kind → Kind → Type where
  | guardedDouble : Source [.number] .number

def source : Semantics Source Literal Value where
  literal := literalValue
  primitive := fun op args => match op, args with
    | .guardedDouble, .cons x .nil => if x != 0 then some (x + x) else none

def expansion : Source params t → Program Op Literal params t
  | .guardedDouble =>
      .call .add (.cons (.reg .here) (.cons (.reg .here) .nil))
      (.call .nonzero (.cons (.reg (.there .here)) .nil)
      (.call .require (.cons (.reg .here) (.cons (.reg (.there .here)) .nil))
      (.ret (.reg .here))))

theorem expansion_exact (op : Source params t) (xs : HList Value params) :
    (expansion op).run target (xs.map (fun x => x)).get =
      (source.primitive op xs).map (fun x => x) := by
  cases op
  cases xs with | cons x xs =>
    cases xs
    simp [expansion, Program.run, target, source, HList.map, HList.get,
      Arg.eval, Env.cons]

def input (x : Nat) : Env Value [.number] := (HList.cons x .nil).get

def shared : Program Source Literal [.number] .number :=
  .call .guardedDouble (.cons (.reg .here) .nil) (.ret (.reg .here))

def compiled := shared.translate (fun x => x) expansion

theorem shared_exact (x : Nat) :
    compiled.run target (input x) = shared.run source (input x) := by
  have exact := shared.run_translate (fun x => x) expansion source target (fun x => x)
    (fun _ => rfl) expansion_exact (input x)
  change compiled.run target (input x) = (shared.run source (input x)).map id at exact
  simpa using exact

def increment : Program Op Literal [.number] .number :=
  .call .add (.cons (.reg .here) (.cons (.literal (.number 1)) .nil)) (.ret (.reg .here))

def continuation : Program Op Literal [.number, .number] .number :=
  .call .add (.cons (.reg .here) (.cons (.reg (.there .here)) .nil)) (.ret (.reg .here))

/-- Incorrect inlining captures the original input as the new register. -/
def captured : Program Op Literal [.number] .number :=
  .call .add (.cons (.reg .here) (.cons (.literal (.number 1)) .nil))
    (.call .add (.cons (.reg .here) (.cons (.reg .here) .nil)) (.ret (.reg .here)))

def unusedFailure : Program Op Literal [] .number :=
  .call .fail .nil (.ret (.literal (.number 7)))

def erasedFailure : Program Op Literal [] .number := .ret (.literal (.number 7))

def empty : Env Value [] := fun r => nomatch r

def inaccurateExpansion : Source params t → Program Op Literal params t
  | .guardedDouble => .ret (.reg .here)

def changedLiteral : Literal t → Literal t
  | .number n => .number (n + 1)
  | .flag b => .flag b

def returnLiteral : Program Source Literal [] .number := .ret (.literal (.number 3))

theorem shared_result : compiled.run target (input 4) = some 8 := rfl
theorem shared_failure : compiled.run target (input 0) = none := rfl
theorem splice_result : (increment.splice continuation).run target (input 3) = some 7 := rfl
theorem captured_result : captured.run target (input 3) = some 8 := rfl
theorem unused_failure : unusedFailure.run target empty = none := rfl
theorem erased_failure : erasedFailure.run target empty = some 7 := rfl

/-- info: 'TypedProgramControls.expansion_exact' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms expansion_exact
/-- info: 'TypedProgramControls.shared_exact' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms shared_exact
end TypedProgramControls
