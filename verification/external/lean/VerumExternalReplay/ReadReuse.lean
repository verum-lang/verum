import VerumExternalReplay.ReadEvidence
import VerumExternalReplay.FiniteExecutionProjection
import VerumExternalReplay.ObserverAdmission
import VerumExternalReplay.ObserverOwnership

/-! Successful read-value reuse through the existing finite VM semantics.
The computed certificate checks every eager operand, including unused results
and call arguments. It is conservative, proves no totality, and represents only
primitive/lexical-call code. Native Branch/Fold require their own proved transfer.

The exact code, binding and pure/stable semantic owner must stay fixed. Each
recipient must independently satisfy binding, resource and lifecycle admission.
A successful value is reusable; the donor's Reading capability, execution
report, resource debit, update, name, price and evidence are not transferred.
Foreign Clone/allocator/primitive effects and capture authenticity remain native
correspondence obligations. In particular, a resource refusal is not a cached
semantic failure. -/
namespace ReadReuse
open FiniteCode FiniteExecution ReadEvidence ObserverAdmission
set_option warningAsError true

variable {O V E D I S : Type}

/-- A slot can be excluded only if it is absent from the whole eager footprint,
not merely from the backward slice of the selected return value. -/
def excludesSources (code : Code O V) (slots : List InputSource)
    (excluded : InputSource → Bool) : Bool :=
  (footprint code).all (fun n => slots[n]?.all (fun source => !excluded source))

def excludesState (code : Code O V) (slots : List InputSource) : Bool :=
  excludesSources code slots (fun source => match source with | .state => true | _ => false)

def excludesExternal (code : Code O V) (slots : List InputSource) (port : Nat) : Bool :=
  excludesSources code slots (fun source => match source with
    | .state => false
    | .external index => decide (index = port))

/-- The value projection of an existing binding source, including absence. -/
def sourceValue (state : V) (external : List V) : InputSource → Option V
  | .state => some state
  | .external index => external[index]?

/-- Relate a successful native-shaped binding to its exact indexed operand.
Missing slots stay missing; they cannot become invented default values. -/
theorem binding_lookup (width position n : Nat) (state : V) (external bound : List V)
    (slots : List InputSource)
    (success : bindValues width state external slots position = .ok bound) :
    bound[n]? = slots[n]?.bind (sourceValue state external) := by
  induction slots generalizing position n bound with
  | nil =>
      simp only [bindValues, Except.ok.injEq] at success
      subst bound
      simp
  | cons slot slots ih =>
      cases slot with
      | state =>
          cases rest : bindValues width state external slots (position + 1) with
          | error error => simp [bindValues, rest, Except.map] at success
          | ok values =>
              simp only [bindValues, rest, Except.map, Except.ok.injEq] at success
              subst bound
              cases n with
              | zero => rfl
              | succ n => exact ih (position + 1) n values rest
      | external index =>
          cases entry : external[index]? with
          | none => simp [bindValues, entry] at success
          | some value =>
              cases rest : bindValues width state external slots (position + 1) with
              | error error => simp [bindValues, entry, rest, Except.map] at success
              | ok values =>
                  simp only [bindValues, entry, rest, Except.map, Except.ok.injEq] at success
                  subst bound
                  cases n with
                  | zero => simpa only [List.getElem?_cons_zero, Option.bind_some, sourceValue] using entry.symm
                  | succ n => exact ih (position + 1) n values rest

theorem binding_key_agreement (code : Code O V) (binding : Binding D)
    (leftState rightState : V) (leftExternal rightExternal left right : List V)
    (excluded : InputSource → Bool)
    (certificate : excludesSources code binding.slots excluded = true)
    (stable : ∀ source ∈ binding.slots, excluded source = false →
      sourceValue leftState leftExternal source = sourceValue rightState rightExternal source)
    (leftBound : bindValues binding.width leftState leftExternal binding.slots 0 = .ok left)
    (rightBound : bindValues binding.width rightState rightExternal binding.slots 0 = .ok right) :
    key code left = key code right := by
  apply List.map_inj_left.mpr
  intro n member
  rw [binding_lookup _ _ _ _ _ _ _ leftBound, binding_lookup _ _ _ _ _ _ _ rightBound]
  have allowed := List.all_eq_true.mp certificate n member
  cases entry : binding.slots[n]? with
  | none => rfl
  | some source =>
      exact stable source (List.mem_of_getElem? entry) (by simpa [entry] using allowed)

theorem state_key_agreement (code : Code O V) (binding : Binding D)
    (leftState rightState : V) (external left right : List V)
    (certificate : excludesState code binding.slots = true)
    (leftBound : bindValues binding.width leftState external binding.slots 0 = .ok left)
    (rightBound : bindValues binding.width rightState external binding.slots 0 = .ok right) :
    key code left = key code right := by
  apply binding_key_agreement code binding leftState rightState external external left right
    (fun source => match source with | .state => true | _ => false) certificate ?_ leftBound rightBound
  intro source _ allowed
  cases source with
  | state => contradiction
  | external index => rfl

/-- Changing an excluded external port does not change a read at the same
state. All other ports retain their exact Option-valued inputs, not defaults. -/
theorem external_key_agreement (code : Code O V) (binding : Binding D)
    (state : V) (leftExternal rightExternal left right : List V) (port : Nat)
    (certificate : excludesExternal code binding.slots port = true)
    (stable : ∀ index, index ≠ port → leftExternal[index]? = rightExternal[index]?)
    (leftBound : bindValues binding.width state leftExternal binding.slots 0 = .ok left)
    (rightBound : bindValues binding.width state rightExternal binding.slots 0 = .ok right) :
    key code left = key code right := by
  apply binding_key_agreement code binding state state leftExternal rightExternal left right
    (fun source => match source with | .state => false | .external index => decide (index = port))
    certificate ?_ leftBound rightBound
  intro source _ allowed
  cases source with
  | state => rfl
  | external index => exact stable index (of_decide_eq_false allowed)

/-- Completion plus actual root selection supplies a semantic value witness.
No implication from a budget stop to undefinedness is admitted. -/
theorem successful_value (apply : O → List V → Except E V) (code : Code O V)
    (limits : Limits) (ins : List V) (used : Nat) (value : V)
    (complete : (execute apply code limits ins used).stop = .complete)
    (selected : selectRoot code (execute apply code limits ins used) = .selected value) :
    FiniteCode.run (FiniteExecutionProjection.partialApply apply) code ins = some value := by
  rw [FiniteExecutionProjection.execute_projection apply code limits ins used
    (by simp [complete]), selected]

/-- Reuse only the selected successful value. The equal-key recipient would
complete identically under the donor's execution parameters, hence its pure
partial semantics has this value independently of any recipient work allowance.
This does not claim that an actual recipient execution can bypass admission. -/
theorem successful_reuse (apply : O → List V → Except E V) (code : Code O V)
    (limits : Limits) (left right : List V) (used : Nat) (value : V)
    (agree : key code left = key code right)
    (complete : (execute apply code limits left used).stop = .complete)
    (selected : selectRoot code (execute apply code limits left used) = .selected value) :
    FiniteCode.run (FiniteExecutionProjection.partialApply apply) code right = some value := by
  have same := execute_agreement apply code limits left right used agree
  apply successful_value apply code limits right used value
  · rw [← same]; exact complete
  · rw [← same]; exact selected

theorem state_excluded_reuse (apply : O → List V → Except E V) (code : Code O V)
    (binding : Binding D) (leftState rightState : V) (external left right : List V)
    (certificate : excludesState code binding.slots = true)
    (leftBound : bindValues binding.width leftState external binding.slots 0 = .ok left)
    (rightBound : bindValues binding.width rightState external binding.slots 0 = .ok right)
    (limits : Limits) (used : Nat) (value : V)
    (complete : (execute apply code limits left used).stop = .complete)
    (selected : selectRoot code (execute apply code limits left used) = .selected value) :
    FiniteCode.run (FiniteExecutionProjection.partialApply apply) code right = some value :=
  successful_reuse apply code limits left right used value
    (state_key_agreement code binding leftState rightState external left right
      certificate leftBound rightBound) complete selected

theorem external_excluded_reuse (apply : O → List V → Except E V) (code : Code O V)
    (binding : Binding D) (state : V) (leftExternal rightExternal left right : List V) (port : Nat)
    (certificate : excludesExternal code binding.slots port = true)
    (stable : ∀ index, index ≠ port → leftExternal[index]? = rightExternal[index]?)
    (leftBound : bindValues binding.width state leftExternal binding.slots 0 = .ok left)
    (rightBound : bindValues binding.width state rightExternal binding.slots 0 = .ok right)
    (limits : Limits) (used : Nat) (value : V)
    (complete : (execute apply code limits left used).stop = .complete)
    (selected : selectRoot code (execute apply code limits left used) = .selected value) :
    FiniteCode.run (FiniteExecutionProjection.partialApply apply) code right = some value :=
  successful_reuse apply code limits left right used value
    (external_key_agreement code binding state leftExternal rightExternal left right port
      certificate stable leftBound rightBound) complete selected

/-- Equal read values do not identify distinct retained cuts. The existing
ownership constructor attaches a value only to the supplied recipient cut. -/
theorem reading_cuts_distinct (left right : ObserverOwnership.Cut D I S) (value : V)
    (distinct : left ≠ right) :
    ObserverOwnership.read (E := E) left (.ok value) ≠
      ObserverOwnership.read right (.ok value) := by
  intro same
  exact distinct (congrArg Prod.fst (Except.ok.inj same))

end ReadReuse
