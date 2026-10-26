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

def expansion : Source params t → Program Op Literal (params.map id) (id t)
  | .guardedDouble =>
      .call .add (.cons (.reg .here) (.cons (.reg .here) .nil))
      (.call .nonzero (.cons (.reg (.there .here)) .nil)
      (.call .require (.cons (.reg .here) (.cons (.reg (.there .here)) .nil))
      (.ret (.reg .here))))

theorem expansion_exact (op : Source params t) (xs : HList Value params) :
    (expansion op).run target (xs.relabel id (fun x => x)).get =
      (source.primitive op xs).map (fun x => x) := by
  cases op
  cases xs with | cons x xs =>
    cases xs
    simp [expansion, Program.run, target, source, HList.map, HList.get,
      Arg.eval, Env.cons, HList.relabel]

def input (x : Nat) : Env Value [.number] := (HList.cons x .nil).get

def shared : Program Source Literal [.number] .number :=
  .call .guardedDouble (.cons (.reg .here) .nil) (.ret (.reg .here))

def compiled : Program Op Literal [.number] .number := shared.translate id (fun x => x) expansion

theorem shared_exact (x : Nat) :
    compiled.run target (input x) = shared.run source (input x) := by
  have exact := shared.run_translate id (fun x => x) expansion source target (fun x => x)
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

def inaccurateExpansion : Source params t → Program Op Literal (params.map id) (id t)
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

namespace Merged
inductive TargetKind where
  | scalar | condition

abbrev Carrier : TargetKind → Type
  | .scalar => Nat
  | .condition => Bool

abbrev sorts : Kind → TargetKind := fun _ => .scalar

def embed : {t : Kind} → Value t → Carrier (sorts t)
  | .number, n => n
  | .flag, b => if b then 1 else 0

inductive Lit : TargetKind → Type where
  | scalar : Nat → Lit .scalar
  | condition : Bool → Lit .condition

def literals : Literal t → Lit (sorts t)
  | .number n => .scalar n
  | .flag b => .scalar (if b then 1 else 0)

inductive Old : List Kind → Kind → Type where
  | guardedIncrement : Old [.flag, .number] .number

def old : Semantics Old Literal Value where
  literal := literalValue
  primitive := fun op xs => match op, xs with
    | .guardedIncrement, .cons b (.cons n .nil) => if b then some (n + 1) else none

inductive New : List TargetKind → TargetKind → Type where
  | decode : New [.scalar] .condition
  | increment : New [.scalar] .scalar
  | require : New [.condition, .scalar] .scalar

def new : Semantics New Lit Carrier where
  literal := fun l => match l with
    | .scalar n => n
    | .condition b => b
  primitive := fun op xs => match op, xs with
    | .decode, .cons n .nil => some (n == 1)
    | .increment, .cons n .nil => some (n + 1)
    | .require, .cons b (.cons n .nil) => if b then some n else none

/-- The intermediate condition sort is outside the image of `sorts`. -/
def expand : Old params t → Program New Lit (params.map sorts) (sorts t)
  | .guardedIncrement =>
      .call .decode (.cons (.reg .here) .nil)
      (.call .increment (.cons (.reg (.there (.there .here))) .nil)
      (.call .require (.cons (.reg (.there .here)) (.cons (.reg .here) .nil))
      (.ret (.reg .here))))

theorem literals_exact (l : Literal t) : new.literal (literals l) = embed (old.literal l) := by
  cases l <;> rfl

theorem expansion_exact (op : Old params t) (xs : HList Value params) :
    (expand op).run new (xs.relabel sorts embed).get = (old.primitive op xs).map embed := by
  cases op
  cases xs with | cons b xs =>
    cases xs with | cons n xs =>
      cases xs
      cases b <;> rfl

theorem all_programs (p : Program Old Literal Γ t) (env : Env Value Γ) :
    (p.translate sorts literals expand).run new (Env.relabel sorts embed env) =
      (p.run old env).map embed :=
  p.run_translate sorts literals expand old new embed literals_exact expansion_exact env

theorem merged_sorts : sorts .number = sorts .flag := rfl
theorem target_only_sort (t : Kind) : sorts t ≠ .condition := by intro equal; cases equal
theorem distinct_registers :
    (Ref.there (.here : Ref [Kind.number] Kind.number) : Ref [.flag, .number] .number).relabel
      sorts ≠ (Ref.here : Ref [TargetKind.scalar, TargetKind.scalar] TargetKind.scalar) := by
  intro equal
  cases equal

def program : Program Old Literal [.flag, .number] .number :=
  .call .guardedIncrement (.cons (.reg .here) (.cons (.reg (.there .here)) .nil)) (.ret (.reg .here))

def inputs (b : Bool) (n : Nat) : Env Value [.flag, .number] := (.cons b (.cons n .nil) : HList Value _).get

def compiled : Program New Lit [.scalar, .scalar] .scalar := program.translate sorts literals expand

def swapped : Env Carrier [.scalar, .scalar] := (.cons 4 (.cons 1 .nil) : HList Carrier _).get

theorem merged_result : compiled.run new (Env.relabel sorts embed (inputs true 4)) = some 5 := rfl
theorem merged_failure : compiled.run new (Env.relabel sorts embed (inputs false 4)) = none := rfl

/-- info: 'TypedProgramControls.Merged.all_programs' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms all_programs
end Merged
end TypedProgramControls
