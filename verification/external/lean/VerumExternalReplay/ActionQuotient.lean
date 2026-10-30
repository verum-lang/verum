import VerumExternalReplay.PartialObservers

/-! Exact action/query-conditioned factors of partial deterministic observers.
The concrete carrier may be infinite. A finite factor STATE carrier says
nothing about the cardinality of an external input/output alphabet, autonomy,
primitive observable period, learning success, or correspondence to a world.
`none` means semantic undefinedness; incomplete computation is not `none`.
Native bindings, complete input domains and executable semantics are premises.
-/
namespace ActionQuotient
set_option warningAsError true
universe u v w a i o
variable {S : Type u} {Q : Type v} {R : Type w}
  {A : Type a} {I : Type i} {O : Type o}

structure Factor (step : S → A → Option S) (read : S → I → Option O)
    (next : Q → A → Option Q) (view : Q → I → Option O) (encode : S → Q) : Prop where
  update : ∀ s a, next (encode s) a = (step s a).map encode
  observe : ∀ s query, view (encode s) query = read s query

def observation (step : S → A → Option S) (read : S → I → Option O)
    (actions : List A) (s : S) (query : I) : Option O :=
  (PartialObservers.fold step actions s).bind (fun state => read state query)

/-- Every continuation and query is quantified; definedness is independently
observable even if the read is undefined on both resulting states. -/
def Equivalent (step : S → A → Option S) (read : S → I → Option O) (s t : S) : Prop :=
  ∀ actions, (PartialObservers.fold step actions s).isSome =
    (PartialObservers.fold step actions t).isSome ∧
    ∀ query, observation step read actions s query = observation step read actions t query

/-- Distinct factor states must have different future behavior. Reachability
from an installed initial state remains a separate obligation. -/
def Separating (step : Q → A → Option Q) (read : Q → I → Option O) : Prop :=
  ∀ q r, Equivalent step read q r → q = r

variable {step : S → A → Option S} {read : S → I → Option O}
  {next : Q → A → Option Q} {view : Q → I → Option O} {encode : S → Q}

theorem Factor.fold (factor : Factor step read next view encode) (actions : List A) (s : S) :
    PartialObservers.fold next actions (encode s) =
      (PartialObservers.fold step actions s).map encode := by
  simpa only [List.map_id] using
    PartialObservers.fold_transport step next encode id factor.update actions s

theorem Factor.definedness (factor : Factor step read next view encode)
    (actions : List A) (s : S) :
    (PartialObservers.fold next actions (encode s)).isSome =
      (PartialObservers.fold step actions s).isSome := by
  rw [factor.fold]
  simp

theorem Factor.observations (factor : Factor step read next view encode)
    (actions : List A) (s : S) (query : I) :
    observation next view actions (encode s) query = observation step read actions s query := by
  unfold observation
  rw [factor.fold]
  cases PartialObservers.fold step actions s with
  | none => rfl
  | some state => exact factor.observe state query

theorem Factor.kernel_congruence (factor : Factor step read next view encode)
    (same : encode s = encode t) (action : A) :
    (step s action).map encode = (step t action).map encode := by
  rw [← factor.update, ← factor.update, same]

theorem Factor.kernel_equivalent (factor : Factor step read next view encode)
    (same : encode s = encode t) : Equivalent step read s t := by
  intro actions
  constructor
  · rw [← factor.definedness actions s, ← factor.definedness actions t, same]
  · intro query
    rw [← factor.observations actions s query, ← factor.observations actions t query, same]

theorem Factor.equivalent_iff (factor : Factor step read next view encode) :
    Equivalent step read s t ↔ Equivalent next view (encode s) (encode t) := by
  constructor
  · intro same actions
    constructor
    · rw [factor.definedness, factor.definedness]
      exact (same actions).1
    · intro query
      rw [factor.observations, factor.observations]
      exact (same actions).2 query
  · intro same actions
    constructor
    · rw [← factor.definedness actions s, ← factor.definedness actions t]
      exact (same actions).1
    · intro query
      rw [← factor.observations actions s query, ← factor.observations actions t query]
      exact (same actions).2 query

theorem Factor.exact_behavioral_classes (factor : Factor step read next view encode)
    (separating : Separating next view) :
    Equivalent step read s t ↔ encode s = encode t :=
  ⟨fun same => separating _ _ (factor.equivalent_iff.mp same), factor.kernel_equivalent⟩

theorem separating_of_reads (reads : ∀ q r, (∀ query, view q query = view r query) → q = r) :
    Separating next view := by
  intro q r same
  apply reads q r
  intro query
  exact (same []).2 query

theorem query_difference_refuses_autonomous_read (s : S) (q r : I)
    (different : read s q ≠ read s r) :
    ¬ ∃ stateRead : S → Option O, ∀ state query, stateRead state = read state query := by
  rintro ⟨stateRead, agrees⟩
  exact different ((agrees s q).symm.trans (agrees s r))

theorem Factor.compose (first : Factor step read next view encode)
    (second : Factor next view next₂ view₂ encode₂) :
    Factor step read next₂ view₂ (encode₂ ∘ encode) := by
  constructor
  · intro s action
    exact ExactTransports.compose (fun s => step s action) (fun q => next q action)
      (fun r => next₂ r action) encode encode₂
      (fun s => first.update s action) (fun q => second.update q action) s
  · intro s query
    exact (second.observe (encode s) query).trans (first.observe s query)

/-- A representative computes the factor transition. Well-definedness must
hold on every entire class, not just on the representative or sampled members. -/
def quotientStep (step : S → A → Option S) (encode : S → Q) (represent : Q → S)
    (q : Q) (action : A) : Option Q := (step (represent q) action).map encode

def quotientRead (read : S → I → Option O) (represent : Q → S)
    (q : Q) (query : I) : Option O := read (represent q) query

theorem factor_of_congruence (represent : Q → S)
    (retraction : ∀ q, encode (represent q) = q)
    (updates : ∀ s t, encode s = encode t → ∀ action,
      (step s action).map encode = (step t action).map encode)
    (reads : ∀ s t, encode s = encode t → ∀ query, read s query = read t query) :
    Factor step read (quotientStep step encode represent) (quotientRead read represent) encode := by
  exact ⟨fun s a => updates _ s (retraction _) a, fun s q => reads _ s (retraction _) q⟩

/-- An executable certificate check over declared finite domains. The soundness
Its soundness law requires complete state/action/query coverage. For infinite
domains use a universal proof, not a longer list of examples. -/
def check [DecidableEq Q] [DecidableEq O] (states : List S) (actions : List A) (queries : List I)
    (step : S → A → Option S) (read : S → I → Option O)
    (next : Q → A → Option Q) (view : Q → I → Option O) (encode : S → Q) : Bool :=
  states.all fun s =>
    actions.all (fun action => decide (next (encode s) action = (step s action).map encode)) &&
    queries.all (fun query => decide (view (encode s) query = read s query))

theorem check_sound [DecidableEq Q] [DecidableEq O]
    (states : List S) (actions : List A) (queries : List I)
    (allStates : ∀ s, s ∈ states) (allActions : ∀ a, a ∈ actions) (allQueries : ∀ q, q ∈ queries)
    (accepted : check states actions queries step read next view encode = true) :
    Factor step read next view encode := by
  have rows := List.all_eq_true.mp accepted
  constructor
  · intro s a
    have row := rows s (allStates s)
    simp only [Bool.and_eq_true] at row
    have row := row.1
    exact of_decide_eq_true (List.all_eq_true.mp row a (allActions a))
  · intro s q
    have row := rows s (allStates s)
    simp only [Bool.and_eq_true] at row
    have row := row.2
    exact of_decide_eq_true (List.all_eq_true.mp row q (allQueries q))

theorem check_complete [DecidableEq Q] [DecidableEq O]
    (factor : Factor step read next view encode)
    (states : List S) (actions : List A) (queries : List I) :
    check states actions queries step read next view encode = true := by
  apply List.all_eq_true.mpr
  intro s _
  simp only [Bool.and_eq_true]
  constructor
  · exact List.all_eq_true.mpr (fun a _ => decide_eq_true (factor.update s a))
  · exact List.all_eq_true.mpr (fun q _ => decide_eq_true (factor.observe s q))

/-- Total iteration for a designated clock action, not the complete action
monoid. World-clock closure is an additional premise below. -/
def iterate (advance : S → S) : Nat → S → S
  | 0, s => s
  | n + 1, s => advance (iterate advance n s)

theorem cyclic_factor_iterate (advance : S → S) (phase : S → Nat) (period : Nat)
    (commutes : ∀ s, phase (advance s) = (phase s + 1) % period)
    (s : S) (origin : phase s = 0) (n : Nat) :
    phase (iterate advance n s) = n % period := by
  induction n with
  | zero => simpa only [iterate, Nat.zero_mod] using origin
  | succ n ih =>
    rw [iterate, commutes, ih, Nat.mod_add_mod]

theorem cyclic_factor_reaches_phases (advance : S → S) (phase : S → Nat) (period : Nat)
    (commutes : ∀ s, phase (advance s) = (phase s + 1) % period)
    (s : S) (origin : phase s = 0) (position : Nat) (within : position < period) :
    phase (iterate advance position s) = position := by
  rw [cyclic_factor_iterate advance phase period commutes s origin, Nat.mod_eq_of_lt within]

/-- A proved cyclic phase action on a closed world clock yields divisibility.
The closure premise is not inferred from an executable observer or a trace. -/
theorem cyclic_factor_divides_closed_clock (advance : S → S) (phase : S → Nat)
    (period clock : Nat) (commutes : ∀ s, phase (advance s) = (phase s + 1) % period)
    (s : S) (origin : phase s = 0) (closed : iterate advance clock s = s) : period ∣ clock := by
  have exact := cyclic_factor_iterate advance phase period commutes s origin clock
  rw [closed, origin] at exact
  exact Nat.dvd_of_mod_eq_zero exact.symm

/-- Divisibility transports to a supplied multiple. Interpreting that number as
a unit-group order requires a separate group/census proof; neither a group nor
its cardinality is constructed here. These arithmetic laws also admit zero,
so a purchased nontrivial finite clock must separately establish its bounds. -/
theorem cyclic_factor_divides_multiple (advance : S → S) (phase : S → Nat)
    (period clock bound : Nat) (commutes : ∀ s, phase (advance s) = (phase s + 1) % period)
    (s : S) (origin : phase s = 0) (closed : iterate advance clock s = s)
    (clockDivides : clock ∣ bound) : period ∣ bound :=
  Nat.dvd_trans (cyclic_factor_divides_closed_clock advance phase period clock commutes s origin closed)
    clockDivides

end ActionQuotient
