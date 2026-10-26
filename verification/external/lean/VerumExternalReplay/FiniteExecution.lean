import VerumExternalReplay.FiniteCode

/-! Operational observation model for finite owned Code views. This supplements
the pure typed-program projection: completion, semantic refusal, resource
refusal, diagnostic position and prefix registers remain distinct. This models
64-bit counters/usize and the exported bounded meter, which does not charge on
refusal. It does not model an arbitrary WorkMeter. The meter counts structural
work, not elapsed time or arbitrary primitive/clone costs.
It is a specification checked by Lean, not a replacement foreign runtime.
-/
namespace FiniteExecution
open FiniteCode
set_option warningAsError true

inductive Stop where
  | complete | undefined | budget
  deriving DecidableEq

inductive Failure (E : Type) where
  | missingOperand : Nat → Failure E
  | missingInput : Nat → Nat → Failure E
  | missingRegister : Nat → Nat → Failure E
  | missingCallee | missingCalleeOutput
  | primitive : E → Failure E
  | allowance | counterCapacity
  deriving DecidableEq

structure Halt (E : Type) where
  path : List Nat
  reason : Failure E
  deriving DecidableEq

structure WorkRefusal where
  used : Nat
  limit : Nat
  requested : Nat
  deriving DecidableEq

structure Limits where
  dispatch : Option Nat
  work : Nat

structure Usage where
  work : Nat := 0
  dispatch : Nat := 0
  expanded : Nat := 0
  calls : Nat := 0
  deriving DecidableEq

structure Report (V E : Type) where
  stop : Stop
  registers : List V
  usage : Usage
  halt : Option (Halt E)
  meterError : Option WorkRefusal := none
  deriving DecidableEq

/-- Fields actually exposed by the machine's execution report. Work belongs to
the external meter; do not invent a per-invocation work observation. -/
structure Observation (V E : Type) where
  stop : Stop
  registers : List V
  dispatch : Nat
  expanded : Nat
  calls : Nat
  halt : Option (Halt E)
  deriving DecidableEq

def Report.observation (report : Report V E) : Observation V E :=
  ⟨report.stop, report.registers, report.usage.dispatch,
    report.usage.expanded, report.usage.calls, report.halt⟩

def capacity : Nat := 2^64

def reserve (limit units used : Nat) : Except WorkRefusal Nat :=
  if units ≤ limit - used then .ok (used + units)
  else .error ⟨used, limit, units⟩

def stopped (stop : Stop) (regs : List V) (usage : Usage) (reason : Failure E)
    (meterError : Option WorkRefusal := none) : Report V E :=
  ⟨stop, regs, usage, some ⟨[regs.length], reason⟩, meterError⟩

def gate (limits : Limits) (step : Step O V) (usage : Usage) :
    Except (Failure E × Option WorkRefusal × Usage) Usage := do
  if limits.dispatch.any (fun n => usage.dispatch ≥ n) then
    .error (.allowance, none, usage)
  else
    let isCall := match step.operation with | .call _ => true | _ => false
    let units := step.arity + if isCall then 4 else 2
    if units ≥ capacity then .error (.counterCapacity, none, usage)
    else match reserve limits.work units usage.work with
      | .error error => .error (.allowance, some error, usage)
      | .ok work =>
          let paid := { usage with work }
          if usage.dispatch + 1 ≥ capacity then .error (.counterCapacity, none, paid)
          else .ok { paid with
            dispatch := usage.dispatch + 1
            calls := usage.calls + if isCall then 1 else 0
            expanded := usage.expanded + if isCall then 0 else 1 }

def operand (ins regs : List V) (position : Nat) : Option (Operand V) → Except (Failure E) V
  | none => .error (.missingOperand position)
  | some (.literal x) => .ok x
  | some (.input n) => match ins[n]? with
      | some v => .ok v
      | none => .error (.missingInput position n)
  | some (.register n) => match regs[n]? with
      | some v => .ok v
      | none => .error (.missingRegister position n)

def arguments (ins regs : List V) (args : List (Operand V)) (start : Nat) :
    Nat → Except (Failure E) (List V)
  | 0 => .ok []
  | n + 1 => do
      let x ← operand ins regs start args[start]?
      let rest ← arguments ins regs args (start + 1) n
      pure (x :: rest)

def output (selected : Option Nat) (regs : List V) : Option V :=
  match selected with | some n => regs[n]? | none => regs.getLast?

abbrev Callee (V E : Type) := Limits → List V → Usage → Report V E × Option Nat

def steps (apply : O → List V → Except E V) (library : List (Callee V E))
    (limits : Limits) (ins : List V) (code : List (Step O V))
    (regs : List V) (usage : Usage) : Report V E :=
  match code with
  | [] => ⟨.complete, regs, usage, none, none⟩
  | step :: rest =>
      match gate limits step usage with
      | .error (reason, meter, paid) => stopped .budget regs paid reason meter
      | .ok paid =>
          match arguments ins regs step.arguments 0 step.arity with
          | .error reason => stopped .undefined regs paid reason
          | .ok args =>
              match step.operation with
              | .primitive op =>
                  match apply op args with
                  | .error error => stopped .undefined regs paid (.primitive error)
                  | .ok x => steps apply library limits ins rest (regs ++ [x]) paid
              | .call n =>
                  match library[n]? with
                  | none => stopped .undefined regs paid .missingCallee
                  | some callee =>
                      let (child, selected) := callee limits args paid
                      if child.stop = .complete then
                        match output selected child.registers with
                        | some x => steps apply library limits ins rest (regs ++ [x]) child.usage
                        | none => ⟨.undefined, regs, child.usage,
                            some ⟨[regs.length, child.registers.length], .missingCalleeOutput⟩, none⟩
                      else { child with
                        registers := regs
                        halt := child.halt.map (fun h => { h with path := regs.length :: h.path }) }

def frame (apply : O → List V → Except E V) (code : Code O V) : Callee V E :=
  match code with
  | .mk instructions selected library => fun limits ins usage =>
      (steps apply (library.attach.map (fun child => frame apply child.val))
        limits ins instructions [] usage, selected)
termination_by sizeOf code
decreasing_by
  have h := List.sizeOf_lt_of_mem child.property
  simp only [Code.mk.sizeOf_spec]
  omega

def execute (apply : O → List V → Except E V) (code : Code O V)
    (limits : Limits) (ins : List V) (used : Nat := 0) : Report V E :=
  match reserve limits.work 1 used with
  | .error error => stopped .budget [] {work := used} .allowance (some error)
  | .ok work => (frame apply code limits ins {work}).1

inductive Root (V : Type) where
  | unattempted | missing
  | selected : V → Root V
  deriving DecidableEq

def selectRoot (code : Code O V) (report : Report V E) : Root V :=
  if report.stop = .complete then
    let .mk _ selected _ := code
    match output selected report.registers with
    | some x => .selected x
    | none => .missing
  else .unattempted

-- These are observation boundaries, not assumptions that budgets imply failure
-- of the unlimited pure computation.
theorem refused_root_unattempted (code : Code O V) (report : Report V E)
    (h : report.stop ≠ .complete) : selectRoot code report = .unattempted := by
  simp [selectRoot, h]

theorem dispatch_refusal_precedes_work (limits : Limits) (step : Step O V) (usage : Usage)
    (h : limits.dispatch.any (fun n => usage.dispatch ≥ n) = true) :
    gate (E := E) limits step usage = .error (.allowance, none, usage) := by
  simp [gate, h]

macro "reduce_execution" : tactic => `(tactic|
  (simp only [FiniteExecution.execute, FiniteExecution.frame,
    List.attach_cons, List.attach_nil, List.map_cons, List.map_nil]; rfl))

end FiniteExecution
