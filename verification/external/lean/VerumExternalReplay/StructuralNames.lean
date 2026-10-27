import VerumExternalReplay.FiniteCode

/- Exact finite structural naming relative to a frozen catalogue. Ownership is
   a nominal identity supplied by the exporter, not an allocator theorem. The
   codec models gamma instruction length and fixed-width opcode/operand/root
   fields. It does not prove enumeration order, family cardinality, Kraft sums,
   runtime allocation or work-meter fidelity. Native observations must separately
   satisfy decoding, encoding and materialization equations against this model. -/
namespace StructuralNames
set_option warningAsError true
open FiniteCode
variable {O V : Type}

structure Grammar (O V : Type) where
  owner : Nat
  inputs : Nat
  limit : Option Nat
  operations : List (O × Nat)
  literals : List V
  calls : List (Nat × Code O V)

structure Instruction where
  alternative : Nat
  operands : List Nat
  deriving DecidableEq

structure Address where
  owner : Nat
  instructions : List Instruction
  output : Nat
  deriving DecidableEq

def width (choices : Nat) : Nat := if choices ≤ 1 then 0 else (choices - 1).log2 + 1

def bits : Nat → Nat → List Bool
  | 0, _ => []
  | n + 1, value => decide (value / 2^n % 2 = 1) :: bits n value

def readBits : Nat → List Bool → Nat → Option (Nat × List Bool)
  | 0, rest, value => some (value, rest)
  | _ + 1, [], _ => none
  | n + 1, bit :: rest, value => readBits n rest (value * 2 + if bit then 1 else 0)

def gamma (n : Nat) : List Bool := List.replicate n.log2 false ++ bits (n.log2 + 1) n

def readGamma : List Bool → Nat → Option (Nat × List Bool)
  | [], _ => none
  | false :: rest, zeros => readGamma rest (zeros + 1)
  | true :: rest, zeros => readBits zeros rest 1

def alternative (g : Grammar O V) (n : Nat) : Option (Operation O × Nat) :=
  if n < g.operations.length then g.operations[n]?.map (fun (op, arity) => (.primitive op, arity))
  else g.calls[n - g.operations.length]?.map (fun (arity, _) => (.call (n - g.operations.length), arity))

def pool (g : Grammar O V) (position : Nat) := g.inputs + position + g.literals.length

def readOperands (available : Nat) : Nat → List Bool → Option (List Nat × List Bool)
  | 0, word => some ([], word)
  | n + 1, word => do
    let (value, rest) ← readBits (width available) word 0
    if value ≥ available then none else do
      let (values, tail) ← readOperands available n rest
      pure (value :: values, tail)

def readInstructions (g : Grammar O V) : Nat → Nat → List Bool → Option (List Instruction × List Bool)
  | 0, _, word => some ([], word)
  | n + 1, position, word => do
    let (op, rest) ← readBits (width (g.operations.length + g.calls.length)) word 0
    let (_, arity) ← alternative g op
    if pool g position ≥ 2^64 then none else do
     let (args, rest) ← readOperands (pool g position) arity rest
     let (instructions, tail) ← readInstructions g n (position + 1) rest
     pure (⟨op, args⟩ :: instructions, tail)

def inExtent (g : Grammar O V) (length : Nat) : Bool :=
  decide (0 < length ∧ length < 2^64 ∧ g.inputs < 2^64 ∧
    g.operations.length + g.calls.length < 2^64) &&
    g.limit.all (fun bound => decide (length ≤ bound))

def decode (g : Grammar O V) (word : List Bool) : Option Address := do
  let (length, rest) ← readGamma word 0
  if !inExtent g length then none else do
    let (instructions, rest) ← readInstructions g length 0 rest
    let (output, rest) ← readBits (width length) rest 0
    if output < length && rest.isEmpty then some ⟨g.owner, instructions, output⟩ else none

def operand (g : Grammar O V) (position n : Nat) : Option (Operand V) :=
  if n < g.inputs then some (.input n)
  else if n < g.inputs + position then some (.register (n - g.inputs))
  else g.literals[n - g.inputs - position]?.map Operand.literal

def materializeInstructions (g : Grammar O V) : Nat → List Instruction → Option (List (Step O V))
  | _, [] => some []
  | position, instruction :: rest => do
    let (operation, arity) ← alternative g instruction.alternative
    if instruction.operands.length != arity || pool g position ≥ 2^64 then none else do
      let arguments ← instruction.operands.mapM (operand g position)
      let steps ← materializeInstructions g (position + 1) rest
      pure (⟨operation, arguments, none⟩ :: steps)

def materialize (g : Grammar O V) (address : Address) : Option (Code O V) := do
  if address.owner != g.owner || !inExtent g address.instructions.length ||
      address.output ≥ address.instructions.length then none else do
    let steps ← materializeInstructions g 0 address.instructions
    pure (.mk steps (some address.output) (g.calls.map Prod.snd))

def encodeInstructions (g : Grammar O V) : Nat → List Instruction → List Bool
  | _, [] => []
  | position, instruction :: rest =>
    bits (width (g.operations.length + g.calls.length)) instruction.alternative ++
    instruction.operands.flatMap (bits (width (pool g position))) ++ encodeInstructions g (position + 1) rest

def encode (g : Grammar O V) (address : Address) : Option (List Bool) := do
  let _ ← materialize g address
  pure (gamma address.instructions.length ++ encodeInstructions g 0 address.instructions ++
    bits (width address.instructions.length) address.output)

def VerifiedName (g : Grammar O V) (word : List Bool) (address : Address) : Prop :=
  decode g word = some address ∧ encode g address = some word

-- Applies to names that actually pass replay in the SAME immutable catalogue.
-- No universal encode/decode theorem or equivalence across foreign owners is
-- assumed. Different structural programs may have the same semantic behavior.
theorem verified_name_injective (g : Grammar O V) (word : List Bool) (a b : Address)
    (first : VerifiedName g word a) (second : VerifiedName g word b) : a = b :=
  Option.some.inj (first.1.symm.trans second.1)

theorem decoded_materialization_unique (g : Grammar O V) (word : List Bool)
    (a b : Address) (first second : Code O V)
    (da : decode g word = some a) (db : decode g word = some b)
    (ma : materialize g a = some first) (mb : materialize g b = some second) : first = second := by
  have same : a = b := Option.some.inj (da.symm.trans db)
  subst b
  exact Option.some.inj (ma.symm.trans mb)

theorem foreign_owner_refused (g : Grammar O V) (address : Address)
    (foreign : address.owner ≠ g.owner) : materialize g address = none := by
  simp [materialize, foreign]

theorem foreign_owner_not_encoded (g : Grammar O V) (address : Address)
    (foreign : address.owner ≠ g.owner) : encode g address = none := by
  simp [encode, foreign_owner_refused g address foreign]

end StructuralNames
