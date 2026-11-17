import VerumExternalReplay.TermScope

/-! Lookup and weakening of local definitions in the executable Lean checker.
A binding stores both its type and its optional source in the preceding context. -/

namespace VerumKernel

/-- Later assumptions and definitions leave the selected binder's type at its original slot. -/
theorem lookupCtx_after_prefix (outer : Ctx) (later : List Binding) (binding : Binding) :
    lookupCtx { outer with bindings := later ++ binding :: outer.bindings } later.length =
      some (shiftUp (later.length + 1) 0 binding.ty) := by
  simp [lookupCtx]

/-- Lookup under a telescope selects the original source and lifts it across every later binder. -/
theorem definition_after_prefix (outer later : List Binding) (binding : Binding) (depth : Nat) :
    (Conversion.mk (later ++ binding :: outer) depth).definition (depth + later.length) =
      binding.value.map (shiftUp (depth + later.length + 1) 0) := by
  simp [Conversion.definition]

/-- A reference already resolved in the outer frame denotes its weakening under new binders. -/
theorem definition_under (scope : Conversion) (fresh i : Nat) :
    (scope.under fresh).definition (fresh + i) =
      (scope.definition i).map (shiftUp fresh 0) := by
  by_cases found : scope.depth ≤ i
  · have reachable : scope.depth + fresh ≤ fresh + i := by omega
    have index : fresh + i - (scope.depth + fresh) = i - scope.depth := by omega
    simp only [Conversion.definition, Conversion.under, found, reachable, ite_true, index]
    cases entry : scope.bindings[i - scope.depth]? with
    | none => rfl
    | some binding =>
      cases source : binding.value with
      | none => simp [source]
      | some value =>
        simp only [Option.bind_some, source, Option.map_some, shiftUp_compose]
        congr 2 <;> omega
  · have hidden : ¬ (scope.depth + fresh ≤ fresh + i) := by omega
    simp [Conversion.definition, Conversion.under, found, hidden]

/-- Every newly introduced parameter remains a neutral variable at the head. -/
theorem whnfFuel_fresh_variable (scope : Conversion) (fresh i fuel : Nat) (bound : i < fresh) :
    whnfFuel (fuel + 1) (.var i) (scope.under fresh).definition = .ok (.var i) := by
  have hidden : ¬ (scope.depth + fresh ≤ i) := by omega
  simp [whnfFuel, Conversion.definition, Conversion.under, hidden, pure, Except.pure]

/-- A selected definition takes one head step and keeps the same environment for further aliases. -/
theorem whnfFuel_selected_definition (scope : Conversion) (i fuel : Nat) (value : Term)
    (selected : scope.definition i = some value) :
    whnfFuel (fuel + 1) (.var i) scope.definition = whnfFuel fuel value scope.definition := by
  simp [whnfFuel, selected]

/-- A checked source from the outer frame remains scoped at its use under arbitrary later binders. -/
theorem definition_preserves_scope (outer later : List Binding) (ty value result : Term)
    (depth : Nat) (valueScoped : scopeError outer.length value = none)
    (selected : (Conversion.mk (later ++ ⟨ty, some value⟩ :: outer) depth).definition
      (depth + later.length) = some result) :
    scopeError (depth + (later ++ ⟨ty, some value⟩ :: outer).length) result = none := by
  rw [definition_after_prefix] at selected
  simp only [Option.map_some, Option.some.injEq] at selected
  subst result
  have lifted := scope_shiftUp (depth + later.length + 1) outer.length 0 value valueScoped
  simpa only [List.length_append, List.length_cons, Nat.add_zero, Nat.add_assoc,
    Nat.add_left_comm, Nat.add_comm] using lifted

/-- info: 'VerumKernel.lookupCtx_after_prefix' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms lookupCtx_after_prefix

/-- info: 'VerumKernel.definition_after_prefix' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms definition_after_prefix

/-- info: 'VerumKernel.definition_under' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms definition_under

/-- info: 'VerumKernel.whnfFuel_fresh_variable' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms whnfFuel_fresh_variable

/-- info: 'VerumKernel.whnfFuel_selected_definition' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms whnfFuel_selected_definition

/-- info: 'VerumKernel.definition_preserves_scope' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms definition_preserves_scope

end VerumKernel
