import VerumExternalReplay.RetainedExecutionTrace

/-! Kernel-computed controls, not observations of a foreign runtime. -/
namespace RetainedExecutionControls
open RetainedExecution FiniteCode
set_option warningAsError true

def apply (ok : Bool) (args : List Nat) : Except Nat Nat :=
  if ok then .ok args.sum else .error 7

def child : Code Bool Nat := .mk
  [⟨.primitive true, [.input 0, .literal 2], none⟩, ⟨.primitive true, [.literal 99], none⟩] (some 0) []
def code : Code Bool Nat := .mk
  [⟨.call 0, [.input 0], none⟩, ⟨.primitive true, [.register 0, .literal 1], none⟩] (some 0) [child]
def start : State Nat Nat := initial [5] (some 4)
def whole (p : Policy) := run code apply p 40 start ⟨100, 100⟩

def sliced (p : Policy) : Nat → State Nat Nat → Run Nat Nat
  | 0, s => ⟨s, ⟨0, 0⟩, [], []⟩
  | n + 1, s =>
      let first := run code apply p 40 s ⟨1, 100⟩
      let rest := sliced p n first.state
      ⟨rest.state, rest.grant, first.events ++ rest.events, first.attempts ++ rest.attempts⟩

def first := run code apply .phased 40 start ⟨1, 100⟩
def second := run code apply .phased 40 first.state ⟨1, 100⟩
def third := run code apply .phased 40 second.state ⟨1, 0⟩
def fourth := run code apply .phased 40 third.state ⟨1, 0⟩
def fifth := run code apply .phased 40 fourth.state ⟨1, 0⟩
def sixth := run code apply .phased 40 fifth.state ⟨1, 0⟩

def single (step : Step Bool Nat) (library : List (Code Bool Nat) := []) :=
  Code.mk [step] none library

def finish (c : Code Bool Nat) (p : Policy := .phased) :=
  (run c apply p 40 (initial [5] none) ⟨100, 100⟩).state

def fullCounter : State Nat Nat :=
  { first.state with originalLimit := none, usage := ⟨1, FiniteExecution.capacity - 1, FiniteExecution.capacity - 1, 0⟩ }
def fullDemand : Demand := ⟨.start, some 1, some false, false⟩

/-- A scope handle need not be an index path into the finite-code syntax. Both
views still run through the same machine, with selected outputs kept explicit. -/
def sparseSource : Source Bool Nat := ⟨fun scope => match scope with
  | [] => some ⟨fun pc => if pc = 0 then some ⟨.call (some [7,9]),[.input 0],1⟩ else none,some 0⟩
  | [7,9] => some ⟨fun pc => match pc with
      | 0 => some ⟨.primitive true,[.input 0,.literal 2],2⟩
      | 1 => some ⟨.primitive true,[.literal 99],1⟩
      | _ => none,some 0⟩
  | _ => none⟩

def sparseRun := run sparseSource apply .phased 40 (initial [5] none) ⟨100,100⟩

example : observation sparseRun.state = some ⟨.complete,[7],3,2,1,none⟩ := by decide
example : decode sparseSource [] 0 = some (.call (some [7,9]),1) := by decide
example : selected sparseSource ⟨[7,9],.call,[5],[7,99]⟩ = some 7 := by decide
example : (instruction sparseSource [7,10] 0).map InstructionView.arity = none := by decide

example : (whole .phased).state.usage = ⟨17, 4, 3, 1⟩ := by decide
example : (whole .atomic).state = (whole .phased).state := by decide
example : (whole .unmetered).state.usage = ⟨1, 4, 3, 1⟩ := by decide
example : observation (whole .phased).state = some ⟨.complete, [7, 8], 4, 3, 1, none⟩ := by decide
example : (whole .phased).state.control.frames.head?.bind (selected (Source.ofCode code)) = some 7 := by decide
example : (sliced .phased 17 start).state = (whole .phased).state := by decide
example : (sliced .phased 17 start).events = (whole .phased).events := by decide
-- Undersized atomic grants cannot buy an instruction by being discarded and summed.
example : (sliced .atomic 17 start).state = first.state := by decide
example : (sliced .phased 16 start).state.control.terminal = none := by decide
example : first.state.control.pending = some ⟨[], 0, .call (some [0]), 1, [], .start⟩ := by decide
example : second.state.control.pending = some ⟨[], 0, .call (some [0]), 1, [], .arguments⟩ := by decide
-- An admitted instruction continues with zero NEW dispatch allowance.
example : third.state.control.pending = some ⟨[], 0, .call (some [0]), 1, [5], .reserveReturn⟩ := by decide
example : fourth.state.control.pending = some ⟨[], 0, .call (some [0]), 1, [5], .moveArguments⟩ := by decide
example : fifth.state.control.pending = some ⟨[], 0, .call (some [0]), 1, [5], .enterCall⟩ := by decide
example : sixth.state.control.frames = [⟨[0], .call, [5], []⟩, ⟨[], .root, [5], []⟩] := by decide
example : sixth.state.usage = ⟨6, 1, 0, 1⟩ := by decide
example : (tick code apply .phased fullCounter ⟨0, 1⟩).status = .paused .counterCapacity := by decide
example : gate fullCounter ⟨0, 0⟩ fullDemand = some .grantDispatch := by decide
example : gate { fullCounter with originalLimit := some 0 } ⟨0, 0⟩ fullDemand = some .originalDispatch := by decide
example : (tick code apply .phased
    { fullCounter with usage := ⟨1, FiniteExecution.capacity - 2, FiniteExecution.capacity - 2, 0⟩ }
    ⟨1, 1⟩).state.usage = ⟨2, FiniteExecution.capacity - 1, FiniteExecution.capacity - 2, 1⟩ := by decide
example : gate start ⟨FiniteExecution.capacity, 1⟩
    ⟨.start, some FiniteExecution.capacity, some false, true⟩ = some .counterCapacity := by decide
example : (tick code apply .phased fullCounter ⟨1, 1⟩).state = fullCounter := by decide
example : (run code apply .phased 40 (initial [5] (some 1)) ⟨100, 100⟩).state.usage = ⟨6, 1, 0, 1⟩ := by decide
example : observation (finish (single ⟨.primitive false, [.input 0], none⟩)) =
    some ⟨.undefined, [], 1, 1, 0, some ⟨[0], .primitive 7⟩⟩ := by decide
example : observation (finish (single ⟨.primitive true, [.register 0], none⟩)) =
    some ⟨.undefined, [], 1, 1, 0, some ⟨[0], .missingRegister 0 0⟩⟩ := by decide
example : observation (finish (single ⟨.primitive true, [.input 1], none⟩)) =
    some ⟨.undefined, [], 1, 1, 0, some ⟨[0], .missingInput 0 1⟩⟩ := by decide
example : (finish (single ⟨.primitive true, [.input 1], none⟩)).usage.work = 3 := by decide
example : (finish (single ⟨.primitive true, [.input 1], none⟩) .atomic).usage.work = 4 := by decide
example : observation (finish (single ⟨.primitive true, [], some 1⟩)) =
    some ⟨.undefined, [], 1, 1, 0, some ⟨[0], .missingOperand 0⟩⟩ := by decide
example : observation (finish (single ⟨.call 0, [], none⟩)) =
    some ⟨.undefined, [], 1, 0, 1, some ⟨[0], .missingCallee⟩⟩ := by decide
example : observation (finish (single ⟨.call 0, [], none⟩ [.mk [] none []])) =
    some ⟨.undefined, [], 1, 0, 1, some ⟨[0, 0], .missingCalleeOutput⟩⟩ := by decide
-- On a nested failure the root keeps its completed prefix and the full halt path.
example : observation (finish (.mk [⟨.primitive true, [.literal 42], none⟩, ⟨.call 0, [], none⟩] none
    [single ⟨.primitive false, [], none⟩])) =
    some ⟨.undefined, [42], 3, 2, 1, some ⟨[1, 0], .primitive 7⟩⟩ := by decide
example : (run code apply .phased 0 start ⟨100, 100⟩).state = start := by decide
example : (run code apply .phased 40 { start with control.poisoned := true } ⟨100, 100⟩).attempts =
    [⟨{ start with control.poisoned := true }, ⟨100, 100⟩, .poisoned, []⟩] := by decide

end RetainedExecutionControls
