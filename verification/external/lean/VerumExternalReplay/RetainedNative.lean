import VerumExternalReplay.RetainedExecutionTrace

/-! Explicit finite u64 adapter and native service observation boundary.
`onPanic` is universally parameterized, not assigned a fabricated return value.
A native panic is checked as an interrupted primitive, never as `next` completing.
The active guard is exposed positively; exported poison bits are never erased.
-/
namespace RetainedNative
open RetainedExecution FiniteExecution
set_option warningAsError true

inductive Primitive where
  | add | identity | reject | resource | panic
  deriving DecidableEq
inductive Error where
  | arity | overflow | rejected | resource
  deriving DecidableEq

def interpret (onPanic : List Nat → Except Error Nat) : Primitive → List Nat → Except Error Nat
  | .add, [a, b] => if a + b < capacity then .ok (a + b) else .error .overflow
  | .identity, [v] => .ok v
  | .reject, _ => .error .rejected
  | .resource, _ => .error .resource
  | .panic, args => onPanic args
  | _, _ => .error .arity

/-- Actual intermediate snapshots carry the active unwind guard, while the
logical admitted-state model is quiescent. The relation checks both bits. -/
def active (logical : State V E) : State V E :=
  { logical with control.poisoned := true }

def ActiveSnapshot (logical native : State V E) : Prop :=
  logical.control.poisoned = false ∧ native = active logical

theorem active_injective (s t : State V E)
    (hs : s.control.poisoned = false) (ht : t.control.poisoned = false)
    (same : active s = active t) : s = t := by
  cases s with | mk sl sc su =>
    cases t with | mk tl tc tu =>
      cases sc; cases tc
      simp_all [active]

/-- A primitive callback can unwind after its admitted work charge and before
producing a result. Its moved local Pending is destroyed. This transition has
NO callback result and does not pretend to complete the source instruction.
Only the actual frozen panic adapter is bound to this observation. -/
def abortPrimitive (policy : Policy) (s : State V E) (g : Grant) : Option (State V E × Grant) := do
  let d ← demand policy s.control
  if d.action != .primitive then none else
  if (gate s g d).isSome then none else
  let control := { s.control with pending := none, poisoned := true }
  let usage := { s.usage with work := s.usage.work + d.units }
  pure ({ s with control, usage }, spend g d)

theorem abort_retains_poison (policy : Policy) (s t : State V E) (g h : Grant)
    (aborted : abortPrimitive policy s g = some (t, h)) : t.control.poisoned = true := by
  unfold abortPrimitive at aborted
  cases hd : demand policy s.control <;> simp [hd] at aborted
  rename_i d
  obtain ⟨_, _, rfl, _⟩ := aborted
  rfl

theorem abort_cannot_resume (source : FiniteCode.Code O V) (apply : O → List V → Except E V)
    (policy : Policy) (s t : State V E) (g h fresh : Grant)
    (aborted : abortPrimitive policy s g = some (t, h)) :
    tick source apply policy t fresh = ⟨t, fresh, .poisoned, []⟩ :=
  poisoned_cannot_resume source apply policy t fresh (abort_retains_poison policy s t g h aborted)

/-- Consuming Machine.finish includes pauses and the unentered-root halt path.
Normal terminal observations remain exactly those of the retained model. -/
def finish (s : State V E) (pause : Option Pause) : Observation V E :=
  match observation s with
  | some result => result
  | none =>
      ⟨.budget, (s.control.frames.getLast?.map (·.registers)).getD [],
        s.usage.dispatch, s.usage.expanded, s.usage.calls,
        some ⟨if s.control.frames.isEmpty then [0]
          else s.control.frames.reverse.map (fun f => f.registers.length),
          if pause = some .counterCapacity then .counterCapacity else .allowance⟩⟩

end RetainedNative
