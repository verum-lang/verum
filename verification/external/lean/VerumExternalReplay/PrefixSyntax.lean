import Std.Tactic

/-! Structural codec and materialization only. Primitive/literal atoms are
canonical captured data, not VM semantics. Recursive edges remain lexical
references; they never unfold the target or reprice it. Native allocation,
physical owner identity, work/panic behavior and execution are separate. -/
namespace PrefixSyntax
set_option warningAsError true

inductive Operand where
  | input (index : Nat) | register (index : Nat) | literal (value : String)
  deriving DecidableEq
inductive Operation where
  | primitive (name : String) | call (index : Nat)
  | branch (yes no : Nat) | fold (body : Nat) | recur (ancestor : Nat) (member : Option Nat)
  deriving DecidableEq
structure Instruction where
  operation : Operation
  operands : List Operand
  deriving DecidableEq
inductive Program where
  | mk (steps : List Instruction) (output : Option Nat) (library : List Program)
inductive Control where
  | branch (yes no : Nat) | fold (body : Nat)
  | recur (ancestor : Nat) (member : Option Nat) (arity : Nat)
  deriving DecidableEq
structure Grammar where
  owner : Nat
  inputs : Nat
  limit : Option Nat
  version : Nat
  operations : List (String × Nat)
  literals : List String
  calls : List (Nat × Program)
  controls : List Control
structure CodeInstruction where
  alternative : Nat
  operands : List Nat
  deriving DecidableEq
inductive Address where
  | mk (owner : Nat) (instructions : List CodeInstruction) (output : Nat)
      (children : List (Nat × Nat × Address))

def lookup (catalog : List Grammar) (owner : Nat) : Option Grammar :=
  catalog.find? (fun g => g.owner == owner)
def width (choices : Nat) : Nat := if choices ≤ 1 then 0 else (choices - 1).log2 + 1
def bits : Nat → Nat → List Nat
  | 0, _ => []
  | n+1, value => (value / 2^n % 2) :: bits n value
def gamma (n : Nat) := List.replicate n.log2 0 ++ bits (n.log2+1) n

def readBits : Nat → List Nat → Nat → Option (Nat × List Nat)
  | 0, rest, value => some (value, rest)
  | _+1, [], _ => none
  | n+1, bit::rest, value =>
    if bit < 2 ∧ value*2+bit < 2^64 then readBits n rest (value*2+bit) else none

def readGamma : List Nat → Nat → Option (Nat × List Nat)
  | [], _ => none
  | 0::rest, zeros => readGamma rest (zeros+1)
  | 1::rest, zeros => readBits zeros rest 1
  | _::_, _ => none

def alternatives (g : Grammar) := g.operations.length + g.calls.length + g.controls.length
def pool (g : Grammar) (position : Nat) := g.inputs + position + g.literals.length

def arity (catalog : List Grammar) (g : Grammar) (alternative : Nat) : Option Nat := do
  if alternative < g.operations.length then return (← g.operations[alternative]?).2
  let localIndex := alternative - g.operations.length
  if localIndex < g.calls.length then return (← g.calls[localIndex]?).1
  match ← g.controls[localIndex-g.calls.length]? with
  | .branch yes no =>
    let left ← lookup catalog yes
    let right ← lookup catalog no
    if left.inputs = right.inputs ∧ left.inputs+1 < 2^64 then return left.inputs+1 else none
  | .fold body =>
    let child ← lookup catalog body
    if child.inputs ≥ 2 then return child.inputs else none
  | .recur _ _ arity => return arity

def childSlots (g : Grammar) (instructions : List CodeInstruction) : List (Nat × Nat × Nat) :=
  instructions.zipIdx.flatMap fun (instruction, position) =>
    if instruction.alternative < g.operations.length+g.calls.length then [] else
    match g.controls[instruction.alternative-g.operations.length-g.calls.length]? with
    | some (.branch yes no) => [(position, 0, yes), (position, 1, no)]
    | some (.fold body) => [(position, 0, body)]
    | _ => []

def readOperands (available : Nat) : Nat → List Nat → Option (List Nat × List Nat)
  | 0, input => some ([], input)
  | n+1, input => do
    let (value, rest) ← readBits (width available) input 0
    if value ≥ available then none else do
      let (values, tail) ← readOperands available n rest
      return (value::values, tail)

def readInstructions (catalog : List Grammar) (g : Grammar) : Nat → Nat → List Nat →
    Option (List CodeInstruction × List Nat)
  | 0, _, input => some ([], input)
  | n+1, position, input => do
    let (op, rest) ← readBits (width (alternatives g)) input 0
    let count ← arity catalog g op
    if pool g position ≥ 2^64 then none else do
      let (args, rest) ← readOperands (pool g position) count rest
      let (instructions, tail) ← readInstructions catalog g n (position+1) rest
      return (⟨op,args⟩::instructions, tail)

def inExtent (g : Grammar) (n : Nat) : Bool :=
  decide (0 < n ∧ n < 2^64 ∧ g.inputs < 2^64 ∧ alternatives g < 2^64) &&
  g.limit.all (fun limit => n ≤ limit)

/-- Fuel bounds lexical catalog descent, not recursive program execution. -/
def decode : Nat → List Grammar → Nat → List Nat → Option (Address × List Nat)
  | 0, _, _, _ => none
  | fuel+1, catalog, owner, input => do
    let g ← lookup catalog owner
    let (n, rest) ← readGamma input 0
    if !inExtent g n then none else do
      let (instructions, rest) ← readInstructions catalog g n 0 rest
      let (output, rest) ← readBits (width n) rest 0
      if output ≥ n then none else do
        let (children, tail) ← (childSlots g instructions).foldlM (fun (children, input) (position,side,child) => do
          let (address, rest) ← decode fuel catalog child input
          return (children ++ [(position,side,address)],rest)) ([],rest)
        return (.mk owner instructions output children,tail)

def resolve (g : Grammar) (position n : Nat) : Option Operand :=
  if n < g.inputs then some (.input n)
  else if n < g.inputs+position then some (.register (n-g.inputs))
  else g.literals[n-g.inputs-position]?.map Operand.literal

def statements (catalog : List Grammar) (g : Grammar) : Nat → Nat → List CodeInstruction → Option (List Instruction)
  | _, _, [] => some []
  | position, child, i::rest => do
    let count ← arity catalog g i.alternative
    if i.operands.length != count || pool g position ≥ 2^64 then none else do
      let args ← i.operands.mapM (resolve g position)
      let (op, used) ← if i.alternative < g.operations.length then do
          let operation ← g.operations[i.alternative]?
          pure (Operation.primitive operation.1, 0)
        else if i.alternative < g.operations.length+g.calls.length then
          pure (.call (i.alternative-g.operations.length),0)
        else do
          match ← g.controls[i.alternative-g.operations.length-g.calls.length]? with
          | .branch _ _ => pure (.branch (g.calls.length+child) (g.calls.length+child+1),2)
          | .fold _ => pure (.fold (g.calls.length+child),1)
          | .recur ancestor member _ => pure (.recur ancestor member,0)
      return ⟨op,args⟩ :: (← statements catalog g (position+1) (child+used) rest)

def addressOwner : Address → Nat | .mk owner _ _ _ => owner

def materialize : Nat → List Grammar → Nat → Address → Option Program
  | 0, _, _, _ => none
  | fuel+1, catalog, owner, .mk actual instructions output children => do
    let g ← lookup catalog owner
    if owner != actual || !inExtent g instructions.length || output ≥ instructions.length then none else do
      let slots := childSlots g instructions
      if children.map (fun (position,side,address) => (position,side,addressOwner address)) != slots then none else do
        let steps ← statements catalog g 0 0 instructions
        let bodies ← children.mapM fun (_,_,address) => materialize fuel catalog (addressOwner address) address
        return .mk steps (some output) (g.calls.map Prod.snd ++ bodies)

def encodeFields (g : Grammar) (instructions : List CodeInstruction) : List Nat :=
  instructions.zipIdx.flatMap fun (i,position) =>
    bits (width (alternatives g)) i.alternative ++ i.operands.flatMap (bits (width (pool g position)))

def encode : Nat → List Grammar → Nat → Address → Option (List Nat)
  | 0, _, _, _ => none
  | fuel+1, catalog, owner, address@(.mk _ instructions output children) => do
    let _ ← materialize (fuel+1) catalog owner address
    let g ← lookup catalog owner
    let suffixes ← children.mapM fun (_,_,child) => encode fuel catalog (addressOwner child) child
    return gamma instructions.length ++ encodeFields g instructions ++
      bits (width instructions.length) output ++ suffixes.flatten

end PrefixSyntax
