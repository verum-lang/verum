import VerumExternalReplay.FiniteExecution

/-! Shared owned-code machine, with each retained control transition explicit.
The source and primitive interpretation are fixed parameters. The counter `work`
is a proof ledger of admitted structural units, not an invented native field.
A bounded non-consuming-on-refusal meter is modeled; callback/Clone purity,
allocation success and native ownership need separate implementation evidence.
Malformed internal states return `none`, never a language-level success/refusal.
-/
namespace RetainedExecution
open FiniteCode FiniteExecution
set_option warningAsError true

abbrev Scope := List Nat
inductive Policy where
  | atomic | unmetered | phased
  deriving DecidableEq
inductive Part where
  | start | arguments | primitive | reserveReturn | moveArguments | enterCall
  deriving DecidableEq
inductive InputKind where
  | root | call
  deriving DecidableEq
inductive Decoded where
  | primitive : Scope → Nat → Decoded
  | call : Option Scope → Decoded
  deriving DecidableEq

structure Frame (V : Type) where
  scope : Scope
  kind : InputKind
  inputs : List V
  registers : List V
  deriving DecidableEq

structure Pending (V : Type) where
  scope : Scope
  pc : Nat
  decoded : Decoded
  arity : Nat
  arguments : List V
  part : Part
  deriving DecidableEq

structure Control (V E : Type) where
  rootInputs : Option (List V)
  /-- Current frame first; a native root-first export must explicitly reverse it. -/
  frames : List (Frame V)
  pending : Option (Pending V)
  terminal : Option (Stop × Option (Failure E))
  poisoned : Bool
  deriving DecidableEq

structure State (V E : Type) where
  originalLimit : Option Nat
  control : Control V E
  usage : Usage
  deriving DecidableEq

inductive Reply (E V : Type) where
  | ok : V → Reply E V
  | error : E → Reply E V
  deriving DecidableEq

def reply : Except E V → Reply E V
  | .ok v => .ok v
  | .error e => .error e

inductive Event (V E : Type) where
  | root
  | decode : Scope → Nat → Option (Decoded × Nat) → Event V E
  | admit : Bool → Event V E
  | argument : Scope → Nat → Nat → Reply (Failure E) V → Event V E
  | phase : Part → Event V E
  | primitive : Scope → Nat → Reply E V → Event V E
  | reserveReturn | reserveFrame
  | enterCall : Option Scope → Event V E
  | output : Scope → Option V → Event V E
  deriving DecidableEq

inductive Action where
  | root | inspect | start | argument | choose | primitive
  | reserveReturn | reserveFrame | enterCall
  deriving DecidableEq

structure Demand where
  action : Action
  /-- None means no meter callback, distinct from a zero-unit charge. -/
  work : Option Nat
  /-- Some true starts a call, Some false starts a primitive. -/
  dispatch : Option Bool := none
  checkedCost : Bool := false
  deriving DecidableEq

def Demand.units (d : Demand) : Nat := d.work.getD 0

def codeAt : Code O V → Scope → Option (Code O V)
  | code, [] => some code
  | .mk _ _ library, index :: rest => library[index]?.bind (fun code => codeAt code rest)

/-- Immutable source access is independent of the syntax that owns a program.
The same view supplies dispatch, operands and selected output, so those facts
cannot silently come from different source projections. -/
inductive Dispatch (O : Type) where
  | primitive : O → Dispatch O
  | call : Option Scope → Dispatch O

structure InstructionView (O V : Type) where
  operation : Dispatch O
  arguments : List (Operand V)
  arity : Nat

structure ScopeView (O V : Type) where
  instruction : Nat → Option (InstructionView O V)
  output : Option Nat

structure Source (O V : Type) where
  scope : Scope → Option (ScopeView O V)

/-- Resolving one instruction does not map or copy the rest of its scope.
Native callback purity and fidelity to this fixed view are separate premises. -/
def Source.ofCode (source : Code O V) : Source O V := ⟨fun scope => do
  let .mk instructions output library ← codeAt source scope
  pure ⟨fun pc => instructions[pc]?.map (fun step =>
    ⟨match step.operation with
      | .primitive op => .primitive op
      | .call index => .call (library[index]?.map (fun _ => scope ++ [index])),
     step.arguments,step.arity⟩),output⟩⟩

instance : CoeOut (Code O V) (Source O V) := ⟨Source.ofCode⟩

def instruction (source : Source O V) (scope : Scope) (pc : Nat) : Option (InstructionView O V) := do
  let view ← source.scope scope
  view.instruction pc

def decode (source : Source O V) (scope : Scope) (pc : Nat) : Option (Decoded × Nat) := do
  let step ← instruction source scope pc
  let op := match step.operation with
    | .primitive _ => .primitive scope pc
    | .call target => .call target
  pure (op, step.arity)

def selected (source : Source O V) (frame : Frame V) : Option V := do
  let view ← source.scope frame.scope
  FiniteExecution.output view.output frame.registers

theorem ofCode_missing_scope (source : Code O V) (scope : Scope)
    (missing : codeAt source scope = none) :
    (Source.ofCode source).scope scope = none := by
  simp [Source.ofCode,missing]

theorem ofCode_decode (source : Code O V) (scope : Scope) (pc : Nat)
    (instructions : List (Step O V)) (output : Option Nat) (library : List (Code O V))
    (step : Step O V) (owned : codeAt source scope = some (.mk instructions output library))
    (found : instructions[pc]? = some step) :
    decode (Source.ofCode source) scope pc = some
      ((match step.operation with
        | .primitive _ => .primitive scope pc
        | .call index => .call (library[index]?.map (fun _ => scope ++ [index]))),step.arity) := by
  cases step with | mk operation arguments declaredArity =>
    cases operation <;> simp [decode,instruction,Source.ofCode,owned,found]

theorem ofCode_operands (source : Code O V) (scope : Scope) (pc : Nat)
    (instructions : List (Step O V)) (output : Option Nat) (library : List (Code O V))
    (step : Step O V) (owned : codeAt source scope = some (.mk instructions output library))
    (found : instructions[pc]? = some step) :
    (instruction (Source.ofCode source) scope pc).map InstructionView.arguments = some step.arguments := by
  simp [instruction,Source.ofCode,owned,found]

theorem ofCode_selected (source : Code O V) (frame : Frame V)
    (instructions : List (Step O V)) (output : Option Nat) (library : List (Code O V))
    (owned : codeAt source frame.scope = some (.mk instructions output library)) :
    selected (Source.ofCode source) frame = FiniteExecution.output output frame.registers := by
  simp [selected,Source.ofCode,owned]

def initial (inputs : List V) (limit : Option Nat) : State V E :=
  ⟨limit, ⟨some inputs, [], none, none, false⟩, {}⟩

def demand (policy : Policy) (c : Control V E) : Option Demand :=
  if c.poisoned || c.terminal.isSome then none else
  match c.frames, c.pending with
  | [], none => some ⟨.root, some 1, none, false⟩
  | [], some _ => none
  | _ :: _, none => some ⟨.inspect, none, none, false⟩
  | frame :: _, some p =>
      if p.scope ≠ frame.scope || p.pc ≠ frame.registers.length || p.arguments.length > p.arity then none else
      let paid := if policy = .phased then some 1 else none
      match p.part with
      | .start =>
          let call := match p.decoded with | .call _ => true | _ => false
          some ⟨.start, if policy = .atomic then some (p.arity + if call then 4 else 2)
            else paid, some call, policy = .atomic⟩
      | .arguments =>
          if p.arguments.length = p.arity then some ⟨.choose, none, none, false⟩
          else some ⟨.argument, paid, none, false⟩
      | .primitive => some ⟨.primitive, paid, none, false⟩
      | .reserveReturn => some ⟨.reserveReturn, paid, none, false⟩
      | .moveArguments => some ⟨.reserveFrame, paid, none, false⟩
      | .enterCall => some ⟨.enterCall, paid, none, false⟩

structure Effect (V E : Type) where
  control : Control V E
  events : List (Event V E)
  deriving DecidableEq

def halt (c : Control V E) (reason : Failure E) : Control V E :=
  { c with pending := none, terminal := some (.undefined, some reason) }

def perform (source : Source O V) (apply : O → List V → Except E V)
    (action : Action) (c : Control V E) : Option (Effect V E) := do
  match action with
  | .root =>
      let ins ← c.rootInputs
      pure ⟨{ c with rootInputs := none, frames := [⟨[], .root, ins, []⟩] }, [.root]⟩
  | .inspect =>
      let frame :: parents := c.frames | none
      let _ ← source.scope frame.scope
      let decoded := decode source frame.scope frame.registers.length
      let event := Event.decode frame.scope frame.registers.length decoded
      match decoded with
      | some (op, arity) =>
          pure ⟨{ c with pending := some ⟨frame.scope, frame.registers.length, op, arity, [], .start⟩ }, [event]⟩
      | none =>
          match parents with
          | [] => pure ⟨{ c with terminal := some (.complete, none) }, [event]⟩
          | parent :: ancestors =>
              let result := selected source frame
              let events := [event, .output frame.scope result]
              match result with
              | none => pure ⟨halt c .missingCalleeOutput, events⟩
              | some value =>
                  pure ⟨{ c with frames := { parent with registers := parent.registers ++ [value] } :: ancestors }, events⟩
  | action =>
      let frame :: parents := c.frames | none
      let p ← c.pending
      match action with
      | .start =>
          let call := match p.decoded with | .call _ => true | _ => false
          pure ⟨{ c with pending := some { p with part := .arguments } }, [.admit call]⟩
      | .argument =>
          let step ← instruction source p.scope p.pc
          let position := p.arguments.length
          let result := operand frame.inputs frame.registers position step.arguments[position]?
          let events := [Event.argument p.scope p.pc position (reply result)]
          match result with
          | .error reason => pure ⟨halt c reason, events⟩
          | .ok value =>
              pure ⟨{ c with pending := some { p with arguments := p.arguments ++ [value] } }, events⟩
      | .choose =>
          let part := match p.decoded with | .primitive .. => .primitive | .call _ => .reserveReturn
          pure ⟨{ c with pending := some { p with part } }, [.phase part]⟩
      | .primitive =>
          let .primitive scope pc := p.decoded | none
          let step ← instruction source scope pc
          let .primitive operation := step.operation | none
          let result := apply operation p.arguments
          let events := [Event.primitive scope pc (reply result)]
          match result with
          | .error error => pure ⟨halt c (.primitive error), events⟩
          | .ok value =>
              pure ⟨{ c with pending := none, frames := { frame with registers := frame.registers ++ [value] } :: parents }, events⟩
      | .reserveReturn =>
          pure ⟨{ c with pending := some { p with part := .moveArguments } }, [.reserveReturn]⟩
      | .reserveFrame =>
          pure ⟨{ c with pending := some { p with part := .enterCall } }, [.reserveFrame]⟩
      | .enterCall =>
          let .call scope := p.decoded | none
          match scope with
          | none => pure ⟨halt c .missingCallee, [.enterCall none]⟩
          | some scope =>
              pure ⟨{ c with pending := none, frames := ⟨scope, .call, p.arguments, []⟩ :: c.frames }, [.enterCall (some scope)]⟩
      | _ => none

def charge (usage : Usage) (d : Demand) : Usage :=
  { work := usage.work + d.units
    dispatch := usage.dispatch + if d.dispatch.isSome then 1 else 0
    expanded := usage.expanded + if d.dispatch = some false then 1 else 0
    calls := usage.calls + if d.dispatch = some true then 1 else 0 }

/-- The deterministic admitted transition. Permission is checked by `tick`;
this semantic spine also retains the exact successful structural-work ledger. -/
def next (source : Source O V) (apply : O → List V → Except E V)
    (policy : Policy) (s : State V E) : Option (State V E × List (Event V E)) := do
  let d ← demand policy s.control
  let effect ← perform source apply d.action s.control
  pure ({ s with control := effect.control, usage := charge s.usage d }, effect.events)

inductive Pause where
  | originalDispatch | grantDispatch | counterCapacity | work
  deriving DecidableEq
structure Grant where
  work : Nat
  dispatch : Nat
  deriving DecidableEq

def gate (s : State V E) (g : Grant) (d : Demand) : Option Pause :=
  if d.dispatch.isSome && s.originalLimit.any (fun n => s.usage.dispatch ≥ n) then some .originalDispatch
  else if d.dispatch.isSome && g.dispatch = 0 then some .grantDispatch
  else if d.dispatch.isSome && s.usage.dispatch + 1 ≥ capacity then some .counterCapacity
  else if d.checkedCost && d.units ≥ capacity then some .counterCapacity
  else if d.units > g.work then some .work else none

def spend (g : Grant) (d : Demand) : Grant :=
  ⟨g.work - d.units, g.dispatch - if d.dispatch.isSome then 1 else 0⟩

inductive Status where
  | advanced | complete | paused : Pause → Status
  | poisoned | invalid
  deriving DecidableEq
structure Tick (V E : Type) where
  state : State V E
  grant : Grant
  status : Status
  /-- Successful semantic callbacks/control transitions; denied attempts are
      reported by status and the unchanged snapshot, never silently billed. -/
  events : List (Event V E)
  deriving DecidableEq

def tick (source : Source O V) (apply : O → List V → Except E V)
    (policy : Policy) (s : State V E) (g : Grant) : Tick V E :=
  if s.control.poisoned then ⟨s, g, .poisoned, []⟩
  else if s.control.terminal.isSome then ⟨s, g, .complete, []⟩
  else match demand policy s.control with
    | none => ⟨s, g, .invalid, []⟩
    | some d => match gate s g d with
      | some why => ⟨s, g, .paused why, []⟩
      | none => match perform source apply d.action s.control with
        | none => ⟨s, g, .invalid, []⟩
        | some effect =>
            ⟨{ s with control := effect.control, usage := charge s.usage d }, spend g d,
              if effect.control.terminal.isSome then .complete else .advanced, effect.events⟩

/-- A finite service retains every attempted tick, including the final refusal.
The recursion bound is a checker bound, not a native work or dispatch allowance.
Exhausting it does not manufacture a terminal report. -/
structure Run (V E : Type) where
  state : State V E
  grant : Grant
  events : List (Event V E)
  attempts : List (Tick V E)
  deriving DecidableEq

def run (source : Source O V) (apply : O → List V → Except E V)
    (policy : Policy) : Nat → State V E → Grant → Run V E
  | 0, s, g => ⟨s, g, [], []⟩
  | n + 1, s, g =>
      let step := tick source apply policy s g
      if step.status = .advanced then
        let rest := run source apply policy n step.state step.grant
        ⟨rest.state, rest.grant, step.events ++ rest.events, step :: rest.attempts⟩
      else ⟨step.state, step.grant, step.events, [step]⟩

/-- The exact normal terminal report. Return outputs are already reflected in
parent registers; root selection is deliberately left to the invocation wrapper. -/
def observation (s : State V E) : Option (Observation V E) := do
  let (stop, reason) ← s.control.terminal
  let root ← s.control.frames.getLast?
  pure ⟨stop, root.registers, s.usage.dispatch, s.usage.expanded, s.usage.calls,
    reason.map (fun why => ⟨s.control.frames.reverse.map (fun f => f.registers.length), why⟩)⟩

end RetainedExecution
