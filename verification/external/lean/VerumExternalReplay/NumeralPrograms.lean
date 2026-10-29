import VerumExternalReplay.TypedPrograms

/-! Closed numerals need no literals or environmental input. The portable core
proves partial iteration; the typed extension uses the existing register
programs and splice theorem. Machine evaluation is partial at its bound, not
modular arithmetic or unbounded runtime storage. -/
namespace NumeralPrograms
set_option warningAsError true

def iterate (step : A → Option A) (initial : Option A) : Nat → Option A
  | 0 => initial
  | n + 1 => (iterate step initial n).bind step

theorem iterate_zero (step : A → Option A) (initial : Option A) :
    iterate step initial 0 = initial := rfl

theorem iterate_successor (step : A → Option A) (initial : Option A) (n : Nat) :
    iterate step initial (n + 1) = (iterate step initial n).bind step := rfl

theorem undefined_stays_undefined (step : A → Option A) (n : Nat) :
    iterate step none n = none := by
  induction n with
  | zero => rfl
  | succ n ih => simp [iterate, ih]

theorem successor_expresses_every_natural (n : Nat) :
    iterate (fun x => some (x + 1)) (some 0) n = some n := by
  induction n with
  | zero => rfl
  | succ n ih => simp [iterate, ih]

namespace Typed
open TypedPrograms
inductive Kind where | number
abbrev Value (_ : Kind) := Nat
abbrev NoLiteral (_ : Kind) := Empty

inductive Op : List Kind → Kind → Type where
  | zero : Op [] .number
  | successor : Op [.number] .number

def semantics (step : Nat → Option Nat) : Semantics Op NoLiteral Value where
  literal := fun value => nomatch value
  primitive := by
    intro params t op
    cases op with
    | zero => exact fun _ => some 0
    | successor => exact fun args => step (args.get .here)

def zero : Program Op NoLiteral [] .number :=
  .call .zero .nil (.ret (.reg .here))

def successor : Program Op NoLiteral [.number] .number :=
  .call .successor (.cons (.reg .here) .nil) (.ret (.reg .here))

def numeral : Nat → Program Op NoLiteral [] .number
  | 0 => zero
  | n + 1 => (numeral n).splice successor

def empty : Env Value [] := fun r => nomatch r

theorem run_successor (step : Nat → Option Nat) (env : Env Value [.number]) :
    successor.run (semantics step) env = step (env .here) := by
  change (step (env .here)).bind some = step (env .here)
  simp

/-- This bridge uses the general composition theorem, without a separate
untyped evaluator or compiler-specific axiom. -/
theorem run_numeral (step : Nat → Option Nat) (n : Nat) :
    (numeral n).run (semantics step) empty = iterate step (some 0) n := by
  induction n with
  | zero => rfl
  | succ n ih =>
    simp only [numeral, Program.run_splice, run_successor, Env.cons, ih, iterate]

theorem closed_natural (n : Nat) :
    (numeral n).run (semantics (fun x => some (x + 1))) empty = some n := by
  rw [run_numeral, successor_expresses_every_natural]

/-- The bound is inclusive: `maximum` is valid, its successor is undefined. -/
def boundedSuccessor (maximum value : Nat) : Option Nat :=
  if value < maximum then some (value + 1) else none

theorem bounded_iteration (maximum n : Nat) :
    iterate (boundedSuccessor maximum) (some 0) n =
      if n ≤ maximum then some n else none := by
  induction n with
  | zero => simp [iterate]
  | succ n ih =>
    rw [iterate, ih]
    by_cases h : n ≤ maximum
    · simp only [ite_eq_left h, Option.bind_some, boundedSuccessor]
      by_cases next : n < maximum
      · simp [next, Nat.succ_le_of_lt next]
      · simp [next, show ¬ n + 1 ≤ maximum by omega]
    · simp [h, show ¬ n + 1 ≤ maximum by omega]

theorem bounded_numeral (maximum n : Nat) :
    (numeral n).run (semantics (boundedSuccessor maximum)) empty =
      if n ≤ maximum then some n else none := by
  rw [run_numeral, bounded_iteration]

def u64Maximum : Nat := 18446744073709551615

theorem u64_numeral (n : Nat) :
    (numeral n).run (semantics (boundedSuccessor u64Maximum)) empty =
      if n ≤ u64Maximum then some n else none := bounded_numeral _ _

theorem u64_overflow : boundedSuccessor u64Maximum u64Maximum = none := by
  simp [boundedSuccessor]

theorem u64_last_successor :
    boundedSuccessor u64Maximum (u64Maximum - 1) = some u64Maximum := by decide
end Typed
end NumeralPrograms
