import VerumExternalReplay.FiniteExecutionProjection
import VerumExternalReplay.ObserverAdmission

/- Independent semantic boundary for finite acquired-observer exports. Trace
   values retain the entire oldest-first event sequence; persistent Rust links
   and allocator behavior are not modeled. The decoder bounds all foreign
   integer fields and preserves full source/action identities. -/
namespace AcquiredObserver
set_option warningAsError true

structure Lineage where
  authority : Nat
  birth : Nat
  deriving DecidableEq

inductive Shape where
  | event | query
  deriving DecidableEq

structure Source where
  epoch : Nat
  rawSpace : Nat
  owner : Option Lineage
  shape : Shape
  deriving DecidableEq

structure Event where
  epoch : Nat
  rawSpace : Nat
  owner : Option Lineage
  raw : UInt64
  input : UInt64
  action : UInt8
  episode : UInt64
  lifeAge : UInt64
  pose : Option UInt8
  deriving DecidableEq

structure Query where
  epoch : Nat
  rawSpace : Nat
  owner : Option Lineage
  raw : UInt64
  input : UInt64
  episode : UInt64
  lifeAge : UInt64
  pose : Option UInt8
  action : Option UInt8
  deriving DecidableEq

inductive Terminal where
  | death | time | completed | ended
  deriving DecidableEq

inductive Outcome where
  | next : UInt64 → Outcome
  | refused : UInt64 → Outcome
  | terminal : Terminal → Outcome
  deriving DecidableEq

inductive Value where
  | unit
  | nat : UInt64 → Value
  | bool : Bool → Value
  | action : UInt8 → Value
  | event : Event → Value
  | query : Query → Value
  | pair : Value → Value → Value
  | trace : List Event → Value
  | outcome : Outcome → Value
  deriving DecidableEq

inductive Op where
  | unit | zero | emptyTrace | identity
  | append | at | last | length | eventInput | eventRaw | eventAction | eventAge
  | eventPose | queryInput | queryRaw | queryAge | queryAction | queryPose
  | pair | first | second | equal | select | add | rem | next | refused
  deriving DecidableEq

inductive Error where
  | arity | sort | missing | overflow | traceCapacity | allocation
  deriving DecidableEq

def arity : Op → Nat
  | .unit | .zero | .emptyTrace => 0
  | .append | .at | .pair | .equal | .add | .rem => 2
  | .select => 3
  | _ => 1

def apply (maxTrace : Nat) (op : Op) (args : List Value) : Except Error Value :=
  if args.length ≠ arity op then .error .arity else
  match op, args with
  | .unit, [] => .ok .unit
  | .zero, [] => .ok (.nat 0)
  | .emptyTrace, [] => .ok (.trace [])
  | .identity, [value] => .ok value
  | .append, [.trace events, .event event] =>
      if events.length + 1 ≥ 2^64 then .error .overflow
      else if events.length + 1 > maxTrace then .error .traceCapacity
      else .ok (.trace (events ++ [event]))
  | .at, [.trace events, .nat index] =>
      match events[index.toNat]? with
      | some event => .ok (.event event)
      | none => .error .missing
  | .last, [.trace events] => match events.getLast? with
      | some event => .ok (.event event)
      | none => .error .missing
  | .length, [.trace events] =>
      if events.length < 2^64 then .ok (.nat (UInt64.ofNat events.length)) else .error .overflow
  | .eventInput, [.event e] => .ok (.nat e.input)
  | .eventRaw, [.event e] => .ok (.nat e.raw)
  | .eventAction, [.event e] => .ok (.action e.action)
  | .eventAge, [.event e] => .ok (.nat e.lifeAge)
  | .eventPose, [.event e] => match e.pose with
      | some p => .ok (.nat (UInt64.ofNat p.toNat))
      | none => .error .missing
  | .queryInput, [.query q] => .ok (.nat q.input)
  | .queryRaw, [.query q] => .ok (.nat q.raw)
  | .queryAge, [.query q] => .ok (.nat q.lifeAge)
  | .queryPose, [.query q] => match q.pose with
      | some p => .ok (.nat (UInt64.ofNat p.toNat))
      | none => .error .missing
  | .queryAction, [.query q] => match q.action with
      | some a => .ok (.action a)
      | none => .error .missing
  | .pair, [a, b] => .ok (.pair a b)
  | .first, [.pair a _] => .ok a
  | .second, [.pair _ b] => .ok b
  | .equal, [a, b] => .ok (.bool (decide (a = b)))
  | .select, [.bool test, a, b] => .ok (if test then a else b)
  | .add, [.nat a, .nat b] =>
      if a.toNat + b.toNat < 2^64 then .ok (.nat (a + b)) else .error .overflow
  | .rem, [.nat a, .nat b] => if b = 0 then .error .missing else .ok (.nat (a % b))
  | .next, [.nat key] => .ok (.outcome (.next key))
  | .refused, [.nat key] => .ok (.outcome (.refused key))
  | _, _ => .error .sort

inductive RefusalClass where
  | undefined | resource | poisoned
  deriving DecidableEq

def primitiveClass : Error → RefusalClass
  | .traceCapacity | .allocation => .resource
  | _ => .undefined

abbrev Observation := FiniteExecution.Observation Value Error

def primitiveRefusal (report : Observation) : Option RefusalClass :=
  report.halt.bind fun halt => match halt.reason with
    | .primitive error => some (primitiveClass error)
    | _ => none

-- Unit-valued foreign work errors reveal that admission failed, not its private
-- counter contents. Preserve the exact observable shape and derive the class.
inductive Refusal where
  | poisoned : ObserverAdmission.Phase → Refusal
  | binding : ObserverAdmission.Phase → ObserverAdmission.BindingRefusal → Refusal
  | work : ObserverAdmission.Phase → ObserverAdmission.WorkPoint → Option Observation → Refusal
  | capacity : ObserverAdmission.Phase → ObserverAdmission.WorkPoint → Refusal
  | execution : ObserverAdmission.Phase → Observation → Bool → Bool → Option RefusalClass → Refusal
  | missingRootOutput : ObserverAdmission.Phase → Observation → Refusal
  deriving DecidableEq

def refusalView : ObserverAdmission.Refusal Observation → Refusal
  | .poisoned phase => .poisoned phase
  | .binding phase reason => .binding phase reason
  | .work phase point _ report => .work phase point report
  | .capacity phase point => .capacity phase point
  | .execution phase report meter counter =>
      .execution phase report meter.isSome counter (primitiveRefusal report)
  | .missingRootOutput phase report => .missingRootOutput phase report

def executionClass (stop : FiniteExecution.Stop) (meter : Bool)
    (primitive : Option RefusalClass) : RefusalClass :=
  if stop = .budget ∨ meter ∨ primitive = some .resource then .resource else .undefined

def classification : Refusal → RefusalClass
  | .poisoned _ => .poisoned
  | .work .. | .capacity .. => .resource
  | .execution _ report meter _ primitive =>
      executionClass report.stop meter primitive
  | _ => .undefined

def observe (result : ObserverAdmission.Result Value Error) : Except Refusal (Value × Observation) :=
  result.observation.mapError refusalView

def run (maxTrace : Nat) (code : FiniteCode.Code Op Value) (binding : ObserverAdmission.Binding Source)
    (state : Value) (source : Source) (values : List Value) : Option Value :=
  if source = binding.interface then
    (FiniteCode.bind binding.width binding.slots state values).bind
      (FiniteCode.run (fun op args => (apply maxTrace op args).toOption) code)
  else none

end AcquiredObserver
