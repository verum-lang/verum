import VerumExternalReplay.FiniteExecution

/-! Observer admission around finite execution. Work is reserved before interface
comparison; reads and updates share their dispatch allowance. A complete body
still needs paid root selection. Purity of user operations, allocator behavior,
panic/poison propagation and foreign ownership are separate implementation laws.
Foreign correspondence uses constructor-validated slots and budgets reachable
from an initially empty allowance; raw structures alone do not establish these
invariants. `validateSlots` exposes the construction check separately.
-/
namespace ObserverAdmission
open FiniteCode FiniteExecution
set_option warningAsError true

inductive Phase where
  | update | read
  deriving DecidableEq

inductive WorkPoint where
  | inputBinding | rootOutput
  deriving DecidableEq

inductive BindingRefusal where
  | invalidSlot : Nat → Nat → Nat → BindingRefusal
  | interface
  | width : Nat → Nat → BindingRefusal
  deriving DecidableEq

structure Binding (D : Type) where
  interface : D
  width : Nat
  slots : List InputSource

def validateSlots (width : Nat) : List InputSource → Nat → Except BindingRefusal Unit
  | [], _ => .ok ()
  | .state :: rest, position => validateSlots width rest (position + 1)
  | .external n :: rest, position =>
      if n < width then validateSlots width rest (position + 1)
      else .error (.invalidSlot position n width)

def bindValues (width : Nat) (state : V) (external : List V) :
    List InputSource → Nat → Except BindingRefusal (List V)
  | [], _ => .ok []
  | .state :: rest, position => (bindValues width state external rest (position + 1)).map (state :: ·)
  | .external n :: rest, position => match external[n]? with
      | none => .error (.invalidSlot position n width)
      | some v => (bindValues width state external rest (position + 1)).map (v :: ·)

/-- Constructor-valid slots and the checked input width suffice for binding in
the value model. This proves definedness rather than assuming two successful
bindings. Native cloning is not represented by value projection: its totality
and preservation of values remain separate implementation obligations. -/
theorem bindValues_total (width position : Nat) (state : V) (external : List V)
    (slots : List InputSource)
    (valid : validateSlots width slots position = .ok ())
    (shape : external.length = width) :
    ∃ bound, bindValues width state external slots position = .ok bound := by
  induction slots generalizing position with
  | nil => exact ⟨[], rfl⟩
  | cons source slots ih =>
      cases source with
      | state =>
          obtain ⟨bound, success⟩ := ih (position + 1) valid
          exact ⟨state :: bound, by simp [bindValues, success, Except.map]⟩
      | external index =>
          have inside : index < width := by
            by_cases inside : index < width
            · exact inside
            · simp [validateSlots, inside] at valid
          have tail : validateSlots width slots (position + 1) = .ok () := by
            simpa [validateSlots, inside] using valid
          obtain ⟨bound, success⟩ := ih (position + 1) tail
          have present : external[index]? = some (external.get ⟨index, by omega⟩) :=
            List.getElem?_eq_getElem (by omega)
          exact ⟨external.get ⟨index, by omega⟩ :: bound,
            by simp [bindValues, present, success, Except.map]⟩

structure Budget where
  dispatchLimit : Option Nat
  workLimit : Nat
  workUsed : Nat := 0
  dispatches : Nat := 0
  poisoned : Bool := false
  deriving DecidableEq

inductive Refusal (R : Type) where
  | poisoned : Phase → Refusal R
  | binding : Phase → BindingRefusal → Refusal R
  | work : Phase → WorkPoint → WorkRefusal → Option R → Refusal R
  | capacity : Phase → WorkPoint → Refusal R
  | execution : Phase → R → Option WorkRefusal → Bool → Refusal R
  | missingRootOutput : Phase → R → Refusal R
  deriving DecidableEq

def Refusal.map (f : R → S) : Refusal R → Refusal S
  | .poisoned p => .poisoned p
  | .binding p e => .binding p e
  | .work p w e r => .work p w e (r.map f)
  | .capacity p w => .capacity p w
  | .execution p r e c => .execution p (f r) e c
  | .missingRootOutput p r => .missingRootOutput p (f r)

structure Result (V E : Type) where
  result : Except (Refusal (Report V E)) (V × Report V E)
  budget : Budget

def Result.observation (result : Result V E) :
    Except (Refusal (Observation V E)) (V × Observation V E) :=
  match result.result with
  | .ok (v, r) => .ok (v, r.observation)
  | .error e => .error (e.map Report.observation)

def runBody (apply : O → List V → Except E V) (phase : Phase) (code : Code O V)
    (ins : List V) (budget : Budget) : Result V E :=
  let maximum := budget.dispatchLimit.getD (capacity - 1)
  let report := execute apply code ⟨some (maximum - budget.dispatches), budget.workLimit⟩
    ins budget.workUsed
  let after := { budget with
    workUsed := report.usage.work
    dispatches := budget.dispatches + report.usage.dispatch }
  if report.stop ≠ .complete ∨ report.meterError.isSome then
    let counterCapacity := budget.dispatchLimit.isNone && after.dispatches == capacity - 1 &&
      report.meterError.isNone && report.halt.any (fun h => match h.reason with
        | .allowance => true | _ => false)
    ⟨.error (.execution phase report report.meterError counterCapacity), after⟩
  else match reserve budget.workLimit 1 after.workUsed with
    | .error error => ⟨.error (.work phase .rootOutput error (some report)), after⟩
    | .ok work =>
        let paid := { after with workUsed := work }
        match selectRoot code report with
        | .selected v => ⟨.ok (v, report), paid⟩
        | _ => ⟨.error (.missingRootOutput phase report), paid⟩

def evaluate [DecidableEq D] (apply : O → List V → Except E V) (phase : Phase)
    (code : Code O V) (binding : Binding D) (state : V)
    (interface : D) (ins : List V) (budget : Budget) : Result V E :=
  if budget.poisoned then ⟨.error (.poisoned phase), budget⟩
  else
    let units := binding.width + binding.slots.length + 1
    if units ≥ capacity then ⟨.error (.capacity phase .inputBinding), budget⟩
    else match reserve budget.workLimit units budget.workUsed with
      | .error error => ⟨.error (.work phase .inputBinding error none), budget⟩
      | .ok work =>
          let paid := { budget with workUsed := work }
          if binding.interface ≠ interface then ⟨.error (.binding phase .interface), paid⟩
          else if ins.length ≠ binding.width then
            ⟨.error (.binding phase (.width binding.width ins.length)), paid⟩
          else match bindValues binding.width state ins binding.slots 0 with
            | .error error => ⟨.error (.binding phase error), paid⟩
            | .ok bound => runBody apply phase code bound paid

theorem poisoned_refused [DecidableEq D] (apply : O → List V → Except E V)
    (phase : Phase) (code : Code O V) (binding : Binding D) (state : V)
    (interface : D) (ins : List V) (budget : Budget) (h : budget.poisoned = true) :
    (evaluate apply phase code binding state interface ins budget).result =
      .error (.poisoned phase) := by simp [evaluate, h]

macro "reduce_admission" : tactic => `(tactic|
  (simp only [ObserverAdmission.evaluate, ObserverAdmission.runBody,
    FiniteExecution.execute, FiniteExecution.frame,
    List.attach_cons, List.attach_nil, List.map_cons, List.map_nil]; rfl))

end ObserverAdmission
