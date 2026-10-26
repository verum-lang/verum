import VerumExternalReplay.FiniteCode

/-! Concrete semantic boundary for the CheckedPairsV1 export. All arithmetic is
64-bit; byte packing discards high bits. This definition is independent of the
Rust implementation. Equality with exported observations is finite conformance,
not a proof of all Rust executions or of the provenance of those observations.
-/
namespace CheckedPairs
set_option warningAsError true

inductive Value where
  | nat : UInt64 → Value
  | bool : Bool → Value
  | pair : Value → Value → Value
  deriving DecidableEq

inductive Op where
  | first | second | checkedAdd | select | rem | packByte | pair | equal
  deriving DecidableEq

inductive Refusal where
  | arity : Op → Nat → Nat → Refusal
  | sort : Op → Nat → Refusal
  | overflow | zeroDivisor
  | byteOverflow : UInt64 → Refusal
  deriving DecidableEq

def arity : Op → Nat
  | .first | .second => 1
  | .select => 3
  | _ => 2

def number (op : Op) (index : Nat) : Value → Except Refusal UInt64
  | .nat n => .ok n
  | _ => .error (.sort op index)

def apply (op : Op) (args : List Value) : Except Refusal Value :=
  match op, args with
  | .first, [a] => match a with
      | .pair x _ => .ok x
      | _ => .error (.sort op 0)
  | .second, [a] => match a with
      | .pair _ y => .ok y
      | _ => .error (.sort op 0)
  | .select, [a, x, y] => match a with
      | .bool b => .ok (if b then x else y)
      | _ => .error (.sort op 0)
  | .pair, [x, y] => .ok (.pair x y)
  | .equal, [x, y] => .ok (.bool (decide (x = y)))
  | .checkedAdd, [a, b] => do
      let x ← number op 0 a
      let y ← number op 1 b
      if x.toNat + y.toNat < 2^64 then
        pure (.nat (x + y))
      else .error .overflow
  | .rem, [a, b] => do
      let x ← number op 0 a
      let y ← number op 1 b
      if y = 0 then .error .zeroDivisor else pure (.nat (x % y))
  | .packByte, [a, b] => do
      let x ← number op 0 a
      let y ← number op 1 b
      if y.toNat > 255 then .error (.byteOverflow y)
      else pure (.nat ((x <<< 8) ||| y))
  | _, _ => .error (.arity op (arity op) args.length)

def partialApply (op : Op) (args : List Value) : Option Value := (apply op args).toOption

abbrev Code := FiniteCode.Code Op Value
def run := FiniteCode.run partialApply

-- These universal equations specify the boundary independently of examples.
theorem checkedAdd_exact (x y : UInt64) :
    apply .checkedAdd [.nat x, .nat y] =
      if x.toNat + y.toNat < 2^64 then .ok (.nat (x + y)) else .error .overflow := rfl

theorem packByte_exact (x y : UInt64) :
    apply .packByte [.nat x, .nat y] =
      if y.toNat > 255 then .error (.byteOverflow y)
      else .ok (.nat ((x <<< 8) ||| y)) := rfl

theorem select_exact (b : Bool) (x y : Value) :
    apply .select [.bool b, x, y] = .ok (if b then x else y) := rfl

/-- info: 'CheckedPairs.checkedAdd_exact' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms checkedAdd_exact

end CheckedPairs
