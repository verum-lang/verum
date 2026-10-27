import VerumExternalReplay.RetainedExecution

/-! Composition of admitted transitions, including exact retained states and
ordered callback results. Refused ticks stutter and remain separately observable;
this is not an unconditional addition law for discarded work grants. -/
namespace RetainedExecution
set_option warningAsError true

inductive Trace (step : S → Option (S × List A)) : S → List A → S → Prop where
  | nil (s : S) : Trace step s [] s
  | cons {s t u : S} {events rest : List A} :
      step s = some (t, events) → Trace step t rest u → Trace step s (events ++ rest) u

theorem Trace.append {step : S → Option (S × List A)}
    (first : Trace step s xs t) (second : Trace step t ys u) : Trace step s (xs ++ ys) u := by
  induction first with
  | nil => simpa using second
  | cons h _ ih => simpa [List.append_assoc] using Trace.cons h (ih second)

/-- A local, complete-state simulation composes. No terminal equality is taken
as a premise; a foreign binding must justify each actual admitted transition. -/
theorem Trace.refine {source : S → Option (S × List A)} {target : T → Option (T × List A)}
    (project : S → T)
    (simulate : ∀ s t events, source s = some (t, events) → target (project s) = some (project t, events))
    (trace : Trace source s events t) : Trace target (project s) events (project t) := by
  induction trace with
  | nil => exact .nil _
  | cons h _ ih => exact .cons (simulate _ _ _ h) ih

/-- For a deterministic fixed-source spine, completed runs have identical final
states AND callback traces, regardless of how admitted transitions were grouped. -/
theorem Trace.terminal_unique {step : S → Option (S × List A)}
    (first : Trace step s xs t) (second : Trace step s ys u)
    (stopped : step t = none) (otherStopped : step u = none) : t = u ∧ xs = ys := by
  induction first generalizing ys u with
  | nil =>
      cases second with
      | nil => exact ⟨rfl, rfl⟩
      | cons h _ => simp [stopped] at h
  | @cons s middle last events rest h tail ih =>
      cases second with
      | nil => simp [otherStopped] at h
      | @cons _ middle' _ events' rest' h' tail' =>
          have same := Option.some.inj (h.symm.trans h')
          cases same
          obtain ⟨endEq, traceEq⟩ := ih tail' stopped otherStopped
          exact ⟨endEq, congrArg (events ++ ·) traceEq⟩

theorem charge_partition (usage : FiniteExecution.Usage) (d : Demand)
    (partition : usage.expanded + usage.calls = usage.dispatch) :
    (charge usage d).expanded + (charge usage d).calls = (charge usage d).dispatch := by
  cases hd : d.dispatch with
  | none => simpa [charge, hd] using partition
  | some call => cases call <;> simp [charge, hd] <;> omega

theorem gate_work_sufficient (s : State V E) (g : Grant) (d : Demand)
    (admitted : gate s g d = none) : d.units ≤ g.work := by
  unfold gate at admitted
  split at admitted <;> try contradiction
  split at admitted <;> try contradiction
  split at admitted <;> try contradiction
  split at admitted <;> try contradiction
  split at admitted <;> try contradiction
  omega

theorem gate_dispatch_sufficient (s : State V E) (g : Grant) (d : Demand)
    (admitted : gate s g d = none) : (if d.dispatch.isSome then 1 else 0) ≤ g.dispatch := by
  unfold gate at admitted
  split at admitted <;> try contradiction
  split at admitted <;> try contradiction
  split <;> simp_all <;> omega

theorem admitted_accounting (s : State V E) (g : Grant) (d : Demand)
    (admitted : gate s g d = none) :
    g.work = (spend g d).work + ((charge s.usage d).work - s.usage.work) ∧
    g.dispatch = (spend g d).dispatch + ((charge s.usage d).dispatch - s.usage.dispatch) := by
  have work := gate_work_sufficient s g d admitted
  have dispatch := gate_dispatch_sufficient s g d admitted
  simp only [spend, charge]
  constructor <;> omega

theorem next_preserves_authority (source : FiniteCode.Code O V) (apply : O → List V → Except E V)
    (policy : Policy) (s t : State V E) (events : List (Event V E))
    (step : next source apply policy s = some (t, events)) : t.originalLimit = s.originalLimit := by
  unfold next at step
  cases hd : demand policy s.control <;> simp [hd] at step
  rename_i d
  cases he : perform source apply d.action s.control <;> simp [he] at step
  obtain ⟨rfl, _⟩ := step
  rfl

theorem next_monotone (source : FiniteCode.Code O V) (apply : O → List V → Except E V)
    (policy : Policy) (s t : State V E) (events : List (Event V E))
    (step : next source apply policy s = some (t, events)) :
    s.usage.work ≤ t.usage.work ∧ s.usage.dispatch ≤ t.usage.dispatch := by
  unfold next at step
  cases hd : demand policy s.control <;> simp [hd] at step
  rename_i d
  cases he : perform source apply d.action s.control <;> simp [he] at step
  obtain ⟨rfl, _⟩ := step
  simp [charge]

theorem terminal_stopped (source : FiniteCode.Code O V) (apply : O → List V → Except E V)
    (policy : Policy) (s : State V E) (terminal : s.control.terminal.isSome = true) :
    next source apply policy s = none := by
  simp [next, demand, terminal]

theorem poisoned_cannot_resume (source : FiniteCode.Code O V) (apply : O → List V → Except E V)
    (policy : Policy) (s : State V E) (g : Grant) (poisoned : s.control.poisoned = true) :
    tick source apply policy s g = ⟨s, g, .poisoned, []⟩ := by simp [tick, poisoned]

/-- Each actual bounded-meter tick either retains everything or follows exactly
one fixed-source transition. In particular a fresh grant cannot replace state. -/
theorem tick_refines (source : FiniteCode.Code O V) (apply : O → List V → Except E V)
    (policy : Policy) (s : State V E) (g : Grant) :
    let observed := tick source apply policy s g
    (observed.state = s ∧ observed.events = []) ∨
      next source apply policy s = some (observed.state, observed.events) := by
  unfold tick
  split
  · exact Or.inl ⟨rfl, rfl⟩
  · split
    · exact Or.inl ⟨rfl, rfl⟩
    · cases hd : demand policy s.control with
      | none => exact Or.inl ⟨rfl, rfl⟩
      | some d =>
        cases hg : gate s g d with
        | some why => exact Or.inl (by simp [hg])
        | none =>
          cases he : perform source apply d.action s.control with
          | none => exact Or.inl (by simp [hg, he])
          | some effect => exact Or.inr (by simp [next, hd, hg, he])

theorem trace_monotone (source : FiniteCode.Code O V) (apply : O → List V → Except E V)
    (policy : Policy) (trace : Trace (next source apply policy) s events t) :
    s.usage.work ≤ t.usage.work ∧ s.usage.dispatch ≤ t.usage.dispatch := by
  induction trace with
  | nil => exact ⟨Nat.le_refl _, Nat.le_refl _⟩
  | cons step _ ih =>
    have bounds := next_monotone source apply policy _ _ _ step
    exact ⟨Nat.le_trans bounds.1 ih.1, Nat.le_trans bounds.2 ih.2⟩

/-- Splitting a retained execution bills only each segment's new dispatches and
work. Previously completed instructions cannot be billed again as new ones. -/
theorem split_accounting (source : FiniteCode.Code O V) (apply : O → List V → Except E V)
    (policy : Policy) (first : Trace (next source apply policy) s xs t)
    (second : Trace (next source apply policy) t ys u) :
    u.usage.work - s.usage.work = (t.usage.work - s.usage.work) + (u.usage.work - t.usage.work) ∧
    u.usage.dispatch - s.usage.dispatch =
      (t.usage.dispatch - s.usage.dispatch) + (u.usage.dispatch - t.usage.dispatch) := by
  have a := trace_monotone source apply policy first
  have b := trace_monotone source apply policy second
  constructor <;> omega

theorem tick_trace (source : FiniteCode.Code O V) (apply : O → List V → Except E V)
    (policy : Policy) (s : State V E) (g : Grant) :
    Trace (next source apply policy) s (tick source apply policy s g).events
      (tick source apply policy s g).state := by
  rcases tick_refines source apply policy s g with ⟨state, events⟩ | step
  · rw [state, events]; exact .nil _
  · simpa using Trace.cons step (.nil _)

/-- This service proof includes denied attempts: only their semantic projection
stutters; the executable run keeps the original statuses and grant snapshots. -/
theorem run_trace (source : FiniteCode.Code O V) (apply : O → List V → Except E V)
    (policy : Policy) (n : Nat) (s : State V E) (g : Grant) :
    Trace (next source apply policy) s (run source apply policy n s g).events
      (run source apply policy n s g).state := by
  induction n generalizing s g with
  | zero => exact .nil _
  | succ n ih =>
      simp only [run]
      split
      · exact (tick_trace source apply policy s g).append (ih _ _)
      · exact tick_trace source apply policy s g

theorem tick_accounting (source : FiniteCode.Code O V) (apply : O → List V → Except E V)
    (policy : Policy) (s : State V E) (g : Grant) :
    let result := tick source apply policy s g
    result.grant.work + result.state.usage.work = g.work + s.usage.work ∧
    result.grant.dispatch + result.state.usage.dispatch = g.dispatch + s.usage.dispatch := by
  unfold tick
  split
  · exact ⟨rfl, rfl⟩
  · split
    · exact ⟨rfl, rfl⟩
    · cases hd : demand policy s.control with
      | none => exact ⟨rfl, rfl⟩
      | some d =>
        cases hg : gate s g d with
        | some why => simp [hg]
        | none =>
          cases he : perform source apply d.action s.control with
          | none => simp [hg, he]
          | some effect =>
            have work := gate_work_sufficient s g d hg
            have dispatch := gate_dispatch_sufficient s g d hg
            simp [hg, he, spend, charge]
            constructor <;> omega

theorem run_accounting (source : FiniteCode.Code O V) (apply : O → List V → Except E V)
    (policy : Policy) (n : Nat) (s : State V E) (g : Grant) :
    let result := run source apply policy n s g
    result.grant.work + result.state.usage.work = g.work + s.usage.work ∧
    result.grant.dispatch + result.state.usage.dispatch = g.dispatch + s.usage.dispatch := by
  induction n generalizing s g with
  | zero => exact ⟨rfl, rfl⟩
  | succ n ih =>
      have first := tick_accounting source apply policy s g
      simp only [run]
      split
      · have rest := ih (tick source apply policy s g).state (tick source apply policy s g).grant
        exact ⟨rest.1.trans first.1, rest.2.trans first.2⟩
      · exact first

theorem trace_authority (source : FiniteCode.Code O V) (apply : O → List V → Except E V)
    (policy : Policy) (trace : Trace (next source apply policy) s events t) :
    t.originalLimit = s.originalLimit := by
  induction trace with
  | nil => rfl
  | cons step _ ih => exact ih.trans (next_preserves_authority source apply policy _ _ _ step)

/-- Arbitrary services may supply fresh grants; each begins with the exact state
left by its predecessor. Neither a new input nor a new original bound is allowed. -/
inductive Services (source : FiniteCode.Code O V) (apply : O → List V → Except E V)
    (policy : Policy) : State V E → List (Event V E) → State V E → Prop where
  | nil (s) : Services source apply policy s [] s
  | cons (n : Nat) (g : Grant) {s t : State V E} {rest : List (Event V E)} :
      Services source apply policy (run source apply policy n s g).state rest t →
      Services source apply policy s ((run source apply policy n s g).events ++ rest) t

theorem services_trace (services : Services source apply policy s events t) :
    Trace (next source apply policy) s events t := by
  induction services with
  | nil => exact .nil _
  | cons n g _ ih => exact (run_trace source apply policy n _ g).append ih

/-- Completed whole and sliced services agree on every retained field, counter,
and ordered callback result. Completion is a premise, not a liveness claim for
inadequate grants. The executable service histories still expose refusals. -/
theorem completed_services_unique
    (first : Services source apply policy s xs t) (second : Services source apply policy s ys u)
    (done : t.control.terminal.isSome = true) (otherDone : u.control.terminal.isSome = true) :
    t = u ∧ xs = ys := by
  exact (services_trace first).terminal_unique (services_trace second)
    (terminal_stopped source apply policy t done) (terminal_stopped source apply policy u otherDone)

end RetainedExecution
