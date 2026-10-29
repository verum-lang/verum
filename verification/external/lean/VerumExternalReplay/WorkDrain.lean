import VerumExternalReplay.RetainedExecutionTrace

/-! Drain a retained service's existing work grant, including unpaid lifecycle
handoffs. Work never increases; a pending handoff at unchanged work must lower
a natural-valued phase rank. Paid steps may reset that rank. This is a reusable
termination contract, not a proof that a foreign callback satisfies it or a
wall-clock bound. Native binding must retain the whole state and every effect.
-/
namespace WorkDrain
set_option warningAsError true

inductive Status (E : Type) where
  /-- Continue this grant; its work/phase progress must be justified. -/
  | pending
  /-- Retain the callback result and return control to the caller. -/
  | yielded
  | ready
  | refused (error : E)
  deriving DecidableEq, Repr

structure Observation (S E A : Type) where
  state : S
  remaining : Nat
  status : Status E
  events : List A
  deriving DecidableEq, Repr

structure Machine (S E A : Type) where
  step : S → Nat → Observation S E A
  rank : S → Nat
  bounded : ∀ s work, (step s work).remaining ≤ work
  progress : ∀ s work, work ≠ 0 → (step s work).status = .pending →
    (step s work).remaining = work → rank (step s work).state < rank s

variable {S E A T : Type}

/-- Zero grants do not call the service. Yielded, ready and refused results stop
with the callback's exact state, work, status and effects. No timeout can become
success, and no hidden iteration charge competes with the actual operation. -/
def drain (m : Machine S E A) (s : S) (work : Nat) : Observation S E A :=
  if work = 0 then ⟨s,work,.pending,[]⟩ else
  let next := m.step s work
  match _status : next.status with
  | .pending =>
      let result := drain m next.state next.remaining
      {result with events := next.events ++ result.events}
  | _ => next
termination_by (work, m.rank s)
decreasing_by
  have bound := m.bounded s work
  by_cases less : (m.step s work).remaining < work
  · exact Prod.Lex.left _ _ less
  · have equal : (m.step s work).remaining = work := by omega
    have lower := m.progress s work (by assumption) (by assumption) equal
    simpa only [equal] using (Prod.Lex.right work lower)

/-- The full actual callback sequence, including its final refusal, is the
semantic witness. A missing callback or a discarded effect cannot be hidden by
matching only the last state or total paid work. -/
inductive Execution (m : Machine S E A) : S → Nat → Observation S E A → Prop where
  | zero (s) : Execution m s 0 ⟨s,0,.pending,[]⟩
  | stop {s work} (positive : work ≠ 0) (terminal : (m.step s work).status ≠ .pending) :
      Execution m s work (m.step s work)
  | more {s work result} (positive : work ≠ 0) (pending : (m.step s work).status = .pending)
      (rest : Execution m (m.step s work).state (m.step s work).remaining result) :
      Execution m s work {result with events := (m.step s work).events ++ result.events}

theorem drain_execution (m : Machine S E A) (s : S) (work : Nat) :
    Execution m s work (drain m s work) := by
  fun_induction drain m s work with
  | case1 s => exact .zero s
  | case2 s work positive next pending result ih => exact .more positive pending ih
  | case3 s work positive next terminal => exact .stop positive terminal

theorem Execution.bounded {m : Machine S E A} (run : Execution m s work result) :
    result.remaining ≤ work := by
  induction run with
  | zero => exact Nat.le_refl _
  | stop => exact m.bounded _ _
  | more _ _ _ ih => exact Nat.le_trans ih (m.bounded _ _)

theorem Execution.stopped {m : Machine S E A} (run : Execution m s work result) :
    result.remaining = 0 ∨ result.status ≠ .pending := by
  induction run with
  | zero => exact Or.inl rfl
  | stop _ terminal => exact Or.inr terminal
  | more _ _ _ ih => exact ih

theorem Execution.preserves {m : Machine S E A} (P : S → Nat → Prop)
    (preserve : ∀ s work, work ≠ 0 → P s work →
      P (m.step s work).state (m.step s work).remaining)
    (run : Execution m s work result) (before : P s work) : P result.state result.remaining := by
  induction run with
  | zero => exact before
  | stop positive _ => exact preserve _ _ positive before
  | more positive _ _ ih => exact ih (preserve _ _ positive before)

/-- A callback's accounting law composes across all paid and unpaid steps,
including an eventual yield or refusal. -/
theorem Execution.accounting {m : Machine S E A} (spent : S → Nat)
    (conserve : ∀ s work, work ≠ 0 →
      (m.step s work).remaining + spent (m.step s work).state = work + spent s)
    (run : Execution m s work result) :
    result.remaining + spent result.state = work + spent s := by
  induction run with
  | zero => rfl
  | stop positive _ => exact conserve _ _ positive
  | more positive _ _ ih => exact ih.trans (conserve _ _ positive)

/-- Compose already justified microsteps, without weakening their full-state
projection or erasing effects from a partial or refused callback. -/
theorem Execution.refine {m : Machine S E A} {step : T → Option (T × List A)}
    (project : S → T)
    (simulate : ∀ s work, work ≠ 0 → RetainedExecution.Trace step
      (project s) (m.step s work).events (project (m.step s work).state))
    (run : Execution m s work result) :
    RetainedExecution.Trace step (project s) result.events (project result.state) := by
  induction run with
  | zero => exact .nil _
  | stop positive _ => exact simulate _ _ positive
  | more positive _ _ ih => exact (simulate _ _ positive).append ih

theorem drain_bounded (m : Machine S E A) (s : S) (work : Nat) :
    (drain m s work).remaining ≤ work := (drain_execution m s work).bounded

theorem drain_stopped (m : Machine S E A) (s : S) (work : Nat) :
    (drain m s work).remaining = 0 ∨ (drain m s work).status ≠ .pending :=
  (drain_execution m s work).stopped

theorem zero_inert (m : Machine S E A) (s : S) :
    drain m s 0 = ⟨s,0,.pending,[]⟩ := by rw [drain]; rfl

theorem terminal_exact (m : Machine S E A) (s : S) (work : Nat)
    (positive : work ≠ 0) (terminal : (m.step s work).status ≠ .pending) :
    drain m s work = m.step s work := by
  rw [drain, ite_eq_right positive]
  dsimp only
  split <;> simp_all

/-- A finite witness cannot stop early while a pending callback still has
work. Checking this relation is equivalent to evaluating the same driver. -/
theorem Execution.exact {m : Machine S E A} (run : Execution m s work result) :
    drain m s work = result := by
  induction run with
  | zero => exact zero_inert _ _
  | stop positive terminal => exact terminal_exact _ _ _ positive terminal
  | more positive pending _ ih =>
      rw [drain, ite_eq_right positive]
      dsimp only
      rw [pending, ih]

theorem execution_iff (m : Machine S E A) (s : S) (work : Nat)
    (result : Observation S E A) : Execution m s work result ↔ drain m s work = result := by
  constructor
  · exact Execution.exact
  · intro equal
    rw [← equal]
    exact drain_execution m s work

/-- Unchanged work and state cannot constitute a pending callback. The rank
obligation rules out a zero-cost cycle rather than hiding it behind fuel. -/
theorem no_pending_stutter (m : Machine S E A) (s : S) (work : Nat)
    (positive : work ≠ 0) (sameWork : (m.step s work).remaining = work)
    (sameState : (m.step s work).state = s) : (m.step s work).status ≠ .pending := by
  intro pending
  have lower := m.progress s work positive pending sameWork
  rw [sameState] at lower
  exact (Nat.lt_irrefl _) lower

end WorkDrain
