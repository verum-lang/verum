import VerumExternalReplay.TermOperations

/-! Scope preservation for the reference checker's actual substitution code.
A scope judgment checks variable binding, not type formation or inhabitation.
The theorems do not establish typing preservation or Rust correspondence.
-/
namespace VerumKernel
set_option warningAsError true

mutual

theorem scope_mapVars (n m : Nat) (replace : Nat → Nat → Term)
    (law : ∀ depth i, i < n + depth → scopeError (m + depth) (replace depth i) = none)
    (cutoff : Nat) (t : Term) (good : scopeError (n + cutoff) t = none) :
    scopeError (m + cutoff) (mapVars replace cutoff t) = none := by
  cases t with
  | var i =>
    simp only [mapVars]
    by_cases bound : i < n + cutoff
    · exact law cutoff i bound
    · simp [scopeError, bound] at good
  | «universe» level => simp only [mapVars, scopeError]
  | data name level constructors =>
    simp only [mapVars, scopeError] at good ⊢
    exact scope_mapConstructors n m replace law cutoff constructors good
  | construct data ctor args =>
    simp only [mapVars, scopeError, Option.or_eq_none_iff] at good ⊢
    exact ⟨scope_mapVars n m replace law cutoff data good.1, scope_mapTerms n m replace law cutoff args good.2⟩
  | induct value motive cases =>
    simp only [mapVars, scopeError, Option.or_eq_none_iff] at good ⊢
    exact ⟨scope_mapVars n m replace law cutoff value good.1, scope_mapVars n m replace law cutoff motive good.2.1, scope_mapTerms n m replace law cutoff cases good.2.2⟩
  | letE value body =>
    simp only [mapVars, scopeError, Option.or_eq_none_iff, Nat.add_assoc] at good ⊢
    exact ⟨scope_mapVars n m replace law cutoff value good.1, scope_mapVars n m replace law (cutoff + 1) body good.2⟩
  | pi a b =>
    simp only [mapVars, scopeError, Option.or_eq_none_iff, Nat.add_assoc] at good ⊢
    exact ⟨scope_mapVars n m replace law cutoff a good.1, scope_mapVars n m replace law (cutoff + 1) b good.2⟩
  | lam a b =>
    simp only [mapVars, scopeError, Option.or_eq_none_iff, Nat.add_assoc] at good ⊢
    exact ⟨scope_mapVars n m replace law cutoff a good.1, scope_mapVars n m replace law (cutoff + 1) b good.2⟩
  | sigma a b =>
    simp only [mapVars, scopeError, Option.or_eq_none_iff, Nat.add_assoc] at good ⊢
    exact ⟨scope_mapVars n m replace law cutoff a good.1, scope_mapVars n m replace law (cutoff + 1) b good.2⟩
  | app a b =>
    simp only [mapVars, scopeError, Option.or_eq_none_iff] at good ⊢
    exact ⟨scope_mapVars n m replace law cutoff a good.1, scope_mapVars n m replace law cutoff b good.2⟩
  | pair a b =>
    simp only [mapVars, scopeError, Option.or_eq_none_iff] at good ⊢
    exact ⟨scope_mapVars n m replace law cutoff a good.1, scope_mapVars n m replace law cutoff b good.2⟩
  | fst value =>
    simp only [mapVars, scopeError] at good ⊢
    exact scope_mapVars n m replace law cutoff value good
  | snd value =>
    simp only [mapVars, scopeError] at good ⊢
    exact scope_mapVars n m replace law cutoff value good
  | refl value =>
    simp only [mapVars, scopeError] at good ⊢
    exact scope_mapVars n m replace law cutoff value good
  | identity a b c =>
    simp only [mapVars, scopeError, Option.or_eq_none_iff] at good ⊢
    exact ⟨scope_mapVars n m replace law cutoff a good.1, scope_mapVars n m replace law cutoff b good.2.1, scope_mapVars n m replace law cutoff c good.2.2⟩
  | transport a b c =>
    simp only [mapVars, scopeError, Option.or_eq_none_iff] at good ⊢
    exact ⟨scope_mapVars n m replace law cutoff a good.1, scope_mapVars n m replace law cutoff b good.2.1, scope_mapVars n m replace law cutoff c good.2.2⟩
termination_by sizeOf t

theorem scope_mapTerms (n m : Nat) (replace : Nat → Nat → Term)
    (law : ∀ depth i, i < n + depth → scopeError (m + depth) (replace depth i) = none)
    (cutoff : Nat) (terms : List Term) (good : scopeTerms (n + cutoff) terms = none) :
    scopeTerms (m + cutoff) (mapTerms replace cutoff terms) = none := by
  cases terms with
  | nil => simp only [mapTerms, scopeTerms]
  | cons term rest =>
    simp only [mapTerms, scopeTerms, Option.or_eq_none_iff] at good ⊢
    exact ⟨scope_mapVars n m replace law cutoff term good.1, scope_mapTerms n m replace law cutoff rest good.2⟩
termination_by sizeOf terms

theorem scope_mapFields (n m : Nat) (replace : Nat → Nat → Term)
    (law : ∀ depth i, i < n + depth → scopeError (m + depth) (replace depth i) = none)
    (cutoff : Nat) (fields : List (DataField Term)) (preceding : Nat)
    (good : scopeFields (n + cutoff) fields preceding = none) :
    scopeFields (m + cutoff) (mapFields replace cutoff fields preceding) preceding = none := by
  cases fields with
  | nil => simp only [mapFields, scopeFields]
  | cons field rest =>
    cases field with
    | recursive =>
      simp only [mapFields, scopeFields] at good ⊢
      exact scope_mapFields n m replace law cutoff rest (preceding + 1) good
    | value term =>
      simp only [mapFields, scopeFields, Option.or_eq_none_iff] at good ⊢
      exact ⟨scope_mapVars n m replace law cutoff term good.1,
        scope_mapFields n m replace law cutoff rest (preceding + 1) good.2⟩
    | dependent term =>
      simp only [mapFields, scopeFields, Option.or_eq_none_iff] at good ⊢
      constructor
      · have head := scope_mapVars n m replace law (cutoff + preceding) term
          (by simpa only [Nat.add_assoc] using good.1)
        simpa only [Nat.add_assoc] using head
      · exact scope_mapFields n m replace law cutoff rest (preceding + 1) good.2
termination_by sizeOf fields

theorem scope_mapConstructors (n m : Nat) (replace : Nat → Nat → Term)
    (law : ∀ depth i, i < n + depth → scopeError (m + depth) (replace depth i) = none)
    (cutoff : Nat) (constructors : List (List (DataField Term))) (good : scopeConstructors (n + cutoff) constructors = none) :
    scopeConstructors (m + cutoff) (mapConstructors replace cutoff constructors) = none := by
  cases constructors with
  | nil => simp only [mapConstructors, scopeConstructors]
  | cons fields rest =>
    simp only [mapConstructors, scopeConstructors, Option.or_eq_none_iff] at good ⊢
    exact ⟨scope_mapFields n m replace law cutoff fields 0 good.1, scope_mapConstructors n m replace law cutoff rest good.2⟩
termination_by sizeOf constructors

end

/-- Inserting new binders preserves binding of every variable, including
    captured parameters inside datatype declarations and induction branches. -/
theorem scope_shiftUp (amount n cutoff : Nat) (t : Term)
    (good : scopeError (n + cutoff) t = none) :
    scopeError (amount + n + cutoff) (shiftUp amount cutoff t) = none := by
  unfold shiftUp
  apply scope_mapVars n (amount + n) _ _ cutoff t good
  intro depth i bound
  by_cases moved : depth ≤ i
  · have fits : i + amount < amount + n + depth := by omega
    simp [moved, scopeError, fits]
  · have fits : i < amount + n + depth := by omega
    simp [moved, scopeError, fits]

/-- Capture-avoiding substitution removes one binder. The replacement is
    scoped in the outer context, below the target and its intervening binders. -/
theorem scope_subst (n target : Nat) (replacement t : Term)
    (replacementGood : scopeError n replacement = none)
    (good : scopeError (n + target + 1) t = none) :
    scopeError (n + target) (subst target replacement t) = none := by
  have bodyGood : scopeError (n + 1 + target) t = none := by
    simpa [Nat.add_assoc, Nat.add_comm, Nat.add_left_comm] using good
  unfold subst
  apply scope_mapVars (n + 1) n _ _ target t bodyGood
  intro depth i bound
  by_cases equal : i = depth
  · simp only [equal, ite_true]
    simpa [Nat.add_comm] using scope_shiftUp depth n 0 replacement replacementGood
  · by_cases above : i > depth
    · have fits : i - 1 < n + depth := by omega
      simp [equal, above, scopeError, fits]
    · have fits : i < n + depth := by omega
      simp [equal, above, scopeError, fits]

/-- Adding unused outer assumptions preserves scope. -/
theorem scope_weaken (n m : Nat) (t : Term) (growth : n ≤ m)
    (good : scopeError n t = none) : scopeError m t = none := by
  have law : ∀ depth i, i < n + depth → scopeError (m + depth) (.var i) = none := by
    intro depth i bound
    have fits : i < m + depth := by omega
    simp [scopeError, fits]
  simpa [mapVars_identity] using scope_mapVars n m (fun _ i => .var i) law 0 t good

/-- This is a necessary condition of the actual executable verifier, not an
    invented typing relation: accepted certificates have closed proof and goal. -/
theorem verifyCertificate_scoped (term claimed : Term)
    (accepted : verifyCertificate term claimed = .ok ()) :
    scopeError 0 term = none ∧ scopeError 0 claimed = none := by
  unfold verifyCertificate checkScope at accepted
  cases goalScope : scopeError 0 claimed with
  | none =>
    cases proofScope : scopeError 0 term with
    | none => exact ⟨rfl, rfl⟩
    | some index =>
      simp only [goalScope, proofScope] at accepted
      cases accepted
  | some index =>
    simp only [goalScope] at accepted
    cases accepted

theorem free_variable_rejected (index : Nat) (claimed : Term) :
    verifyCertificate (.var index) claimed ≠ .ok () := by
  intro accepted
  have closed := (verifyCertificate_scoped (.var index) claimed accepted).1
  simp [scopeError] at closed

/-- info: 'VerumKernel.scope_mapVars' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms scope_mapVars
/-- info: 'VerumKernel.scope_shiftUp' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms scope_shiftUp
/-- info: 'VerumKernel.scope_subst' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms scope_subst
/-- info: 'VerumKernel.scope_weaken' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms scope_weaken
/-- info: 'VerumKernel.verifyCertificate_scoped' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms verifyCertificate_scoped
/-- info: 'VerumKernel.free_variable_rejected' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms free_variable_rejected
end VerumKernel
