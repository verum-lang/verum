import VerumExternalReplay.ReferenceChecker
import Std.Tactic

/-! General laws of the executable reference checker's binder traversal.
These are about every `Term`, including constructor fields and induction
branches. They are prerequisites for typing preservation, not checker soundness.
-/
namespace VerumKernel

set_option warningAsError true

mutual

theorem mapVars_identity (cutoff : Nat) (t : Term) :
    mapVars (fun _ i => .var i) cutoff t = t := by
  cases t with
  | data name level constructors => simp only [mapVars, mapConstructors_identity cutoff constructors]
  | construct data ctor args => simp only [mapVars, mapVars_identity cutoff data, mapTerms_identity cutoff args]
  | induct value motive cases => simp only [mapVars, mapVars_identity cutoff value, mapVars_identity cutoff motive, mapTerms_identity cutoff cases]
  | letE value body => simp only [mapVars, mapVars_identity cutoff value, mapVars_identity (cutoff + 1) body]
  | var i => simp only [mapVars]
  | «universe» level => simp only [mapVars]
  | pi a b => simp only [mapVars, mapVars_identity cutoff a, mapVars_identity (cutoff + 1) b]
  | lam a b => simp only [mapVars, mapVars_identity cutoff a, mapVars_identity (cutoff + 1) b]
  | sigma a b => simp only [mapVars, mapVars_identity cutoff a, mapVars_identity (cutoff + 1) b]
  | app a b => simp only [mapVars, mapVars_identity cutoff a, mapVars_identity cutoff b]
  | pair a b => simp only [mapVars, mapVars_identity cutoff a, mapVars_identity cutoff b]
  | fst value => simp only [mapVars, mapVars_identity cutoff value]
  | snd value => simp only [mapVars, mapVars_identity cutoff value]
  | refl value => simp only [mapVars, mapVars_identity cutoff value]
  | identity a b c => simp only [mapVars, mapVars_identity cutoff a, mapVars_identity cutoff b, mapVars_identity cutoff c]
  | transport a b c => simp only [mapVars, mapVars_identity cutoff a, mapVars_identity cutoff b, mapVars_identity cutoff c]
termination_by sizeOf t

theorem mapTerms_identity (cutoff : Nat) (terms : List Term) :
    mapTerms (fun _ i => .var i) cutoff terms = terms := by
  cases terms with
  | nil => simp only [mapTerms]
  | cons term rest => simp only [mapTerms, mapVars_identity cutoff term, mapTerms_identity cutoff rest]
termination_by sizeOf terms

theorem mapFields_identity (cutoff : Nat) (fields : List (DataField Term)) (preceding : Nat := 0) :
    mapFields (fun _ i => .var i) cutoff fields preceding = fields := by
  cases fields with
  | nil => simp only [mapFields]
  | cons field rest =>
    cases field with
    | recursive => simp only [mapFields, mapFields_identity cutoff rest (preceding + 1)]
    | value term => simp only [mapFields, mapVars_identity cutoff term, mapFields_identity cutoff rest (preceding + 1)]
    | dependent term => simp only [mapFields, mapVars_identity (cutoff + preceding) term, mapFields_identity cutoff rest (preceding + 1)]
termination_by sizeOf fields

theorem mapConstructors_identity (cutoff : Nat) (constructors : List (List (DataField Term))) :
    mapConstructors (fun _ i => .var i) cutoff constructors = constructors := by
  cases constructors with
  | nil => simp only [mapConstructors]
  | cons fields rest => simp only [mapConstructors, mapFields_identity cutoff fields, mapConstructors_identity cutoff rest]
termination_by sizeOf constructors

end

mutual

theorem mapVars_fusion (f g h : Nat → Nat → Term)
    (law : ∀ cutoff i, mapVars g cutoff (f cutoff i) = h cutoff i)
    (cutoff : Nat) (t : Term) :
    mapVars g cutoff (mapVars f cutoff t) = mapVars h cutoff t := by
  cases t with
  | data name level constructors => simp only [mapVars, mapConstructors_fusion f g h law cutoff constructors]
  | construct data ctor args => simp only [mapVars, mapVars_fusion f g h law cutoff data, mapTerms_fusion f g h law cutoff args]
  | induct value motive cases => simp only [mapVars, mapVars_fusion f g h law cutoff value, mapVars_fusion f g h law cutoff motive, mapTerms_fusion f g h law cutoff cases]
  | letE value body => simp only [mapVars, mapVars_fusion f g h law cutoff value, mapVars_fusion f g h law (cutoff + 1) body]
  | var i => simp only [mapVars, law cutoff i]
  | «universe» level => simp only [mapVars]
  | pi a b => simp only [mapVars, mapVars_fusion f g h law cutoff a, mapVars_fusion f g h law (cutoff + 1) b]
  | lam a b => simp only [mapVars, mapVars_fusion f g h law cutoff a, mapVars_fusion f g h law (cutoff + 1) b]
  | sigma a b => simp only [mapVars, mapVars_fusion f g h law cutoff a, mapVars_fusion f g h law (cutoff + 1) b]
  | app a b => simp only [mapVars, mapVars_fusion f g h law cutoff a, mapVars_fusion f g h law cutoff b]
  | pair a b => simp only [mapVars, mapVars_fusion f g h law cutoff a, mapVars_fusion f g h law cutoff b]
  | fst value => simp only [mapVars, mapVars_fusion f g h law cutoff value]
  | snd value => simp only [mapVars, mapVars_fusion f g h law cutoff value]
  | refl value => simp only [mapVars, mapVars_fusion f g h law cutoff value]
  | identity a b c => simp only [mapVars, mapVars_fusion f g h law cutoff a, mapVars_fusion f g h law cutoff b, mapVars_fusion f g h law cutoff c]
  | transport a b c => simp only [mapVars, mapVars_fusion f g h law cutoff a, mapVars_fusion f g h law cutoff b, mapVars_fusion f g h law cutoff c]
termination_by sizeOf t

theorem mapTerms_fusion (f g h : Nat → Nat → Term)
    (law : ∀ cutoff i, mapVars g cutoff (f cutoff i) = h cutoff i)
    (cutoff : Nat) (terms : List Term) :
    mapTerms g cutoff (mapTerms f cutoff terms) = mapTerms h cutoff terms := by
  cases terms with
  | nil => simp only [mapTerms]
  | cons term rest => simp only [mapTerms, mapVars_fusion f g h law cutoff term, mapTerms_fusion f g h law cutoff rest]
termination_by sizeOf terms

theorem mapFields_fusion (f g h : Nat → Nat → Term)
    (law : ∀ cutoff i, mapVars g cutoff (f cutoff i) = h cutoff i)
    (cutoff : Nat) (fields : List (DataField Term)) (preceding : Nat := 0) :
    mapFields g cutoff (mapFields f cutoff fields preceding) preceding = mapFields h cutoff fields preceding := by
  cases fields with
  | nil => simp only [mapFields]
  | cons field rest =>
    cases field with
    | recursive => simp only [mapFields, mapFields_fusion f g h law cutoff rest (preceding + 1)]
    | value term => simp only [mapFields, mapVars_fusion f g h law cutoff term, mapFields_fusion f g h law cutoff rest (preceding + 1)]
    | dependent term => simp only [mapFields, mapVars_fusion f g h law (cutoff + preceding) term, mapFields_fusion f g h law cutoff rest (preceding + 1)]
termination_by sizeOf fields

theorem mapConstructors_fusion (f g h : Nat → Nat → Term)
    (law : ∀ cutoff i, mapVars g cutoff (f cutoff i) = h cutoff i)
    (cutoff : Nat) (constructors : List (List (DataField Term))) :
    mapConstructors g cutoff (mapConstructors f cutoff constructors) = mapConstructors h cutoff constructors := by
  cases constructors with
  | nil => simp only [mapConstructors]
  | cons fields rest => simp only [mapConstructors, mapFields_fusion f g h law cutoff fields, mapConstructors_fusion f g h law cutoff rest]
termination_by sizeOf constructors

end

/-- Pointwise cancellation extends to every term, at every binder depth. -/
theorem mapVars_cancel (f g : Nat → Nat → Term)
    (law : ∀ cutoff i, mapVars g cutoff (f cutoff i) = .var i)
    (cutoff : Nat) (t : Term) : mapVars g cutoff (mapVars f cutoff t) = t :=
  (mapVars_fusion f g (fun _ i => .var i) law cutoff t).trans (mapVars_identity cutoff t)

theorem shiftUp_zero (cutoff : Nat) (t : Term) : shiftUp 0 cutoff t = t := by
  unfold shiftUp
  simp only [Nat.add_zero, ite_self, mapVars_identity]

/-- Lifting twice at the same slot adds the amounts, including under binders. -/
theorem shiftUp_compose (first second cutoff : Nat) (t : Term) :
    shiftUp second cutoff (shiftUp first cutoff t) = shiftUp (first + second) cutoff t := by
  unfold shiftUp
  apply mapVars_fusion
  intro c i
  by_cases bound : c ≤ i
  · have lifted : c ≤ i + first := by omega
    simp [bound, mapVars, lifted, Nat.add_assoc]
  · simp [bound, mapVars]

/-- Lowering cancels lifting; the reverse direction would need a scope premise. -/
theorem shiftDown_shiftUp (amount cutoff : Nat) (t : Term) :
    shiftDown amount cutoff (shiftUp amount cutoff t) = t := by
  unfold shiftUp shiftDown
  apply mapVars_cancel
  intro c i
  by_cases bound : c ≤ i
  · have lifted : c + amount ≤ i + amount := by omega
    simp [bound, mapVars, lifted]
  · have below : ¬ c + amount ≤ i := by omega
    simp [bound, mapVars, below]

/-- Substitution at a newly inserted slot leaves the original term unchanged,
    regardless of the replacement. This includes open terms and nested binders. -/
theorem subst_shiftUp (target : Nat) (replacement t : Term) :
    subst target replacement (shiftUp 1 target t) = t := by
  unfold shiftUp subst
  apply mapVars_cancel
  intro c i
  by_cases bound : c ≤ i
  · have different : i + 1 ≠ c := by omega
    have above : i + 1 > c := by omega
    simp [bound, mapVars, different, above]
  · have different : i ≠ c := by omega
    have below : ¬ i > c := by omega
    simp [bound, mapVars, different, below]

/-- Fuel is an explicit total reduction budget, not a normalization theorem. -/
theorem whnfFuel_zero (t : Term) : whnfFuel 0 t = t := by
  simp only [whnfFuel]

theorem whnfFuel_let (fuel : Nat) (value body : Term) :
    whnfFuel (fuel + 1) (.letE value body) = whnfFuel fuel (subst 0 value body) := by
  simp only [whnfFuel]

theorem whnfFuel_lambda (fuel : Nat) (domain body : Term) :
    whnfFuel fuel (.lam domain body) = .lam domain body := by
  cases fuel <;> simp only [whnfFuel]

theorem whnfFuel_beta (fuel : Nat) (domain body argument : Term) :
    whnfFuel (fuel + 1) (.app (.lam domain body) argument) =
      whnfFuel fuel (subst 0 argument body) := by
  simp only [whnfFuel, whnfFuel_lambda]

/-- info: 'VerumKernel.mapVars_identity' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms mapVars_identity
/-- info: 'VerumKernel.mapVars_fusion' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms mapVars_fusion
/-- info: 'VerumKernel.shiftUp_zero' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms shiftUp_zero
/-- info: 'VerumKernel.shiftUp_compose' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms shiftUp_compose
/-- info: 'VerumKernel.shiftDown_shiftUp' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms shiftDown_shiftUp
/-- info: 'VerumKernel.whnfFuel_beta' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms whnfFuel_beta
/-- info: 'VerumKernel.subst_shiftUp' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms subst_shiftUp

end VerumKernel
