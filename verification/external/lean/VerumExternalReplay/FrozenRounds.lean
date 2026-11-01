import Std.Tactic

/-! Paid visitation of append-only slots, independent of a member's VM outcome.
`beginRound` prepares a candidate snapshot before the next native invocation;
`handoff` consumes the invocation's counted work or reserves one routing unit.
`attempt` publishes that candidate only on a successful paid handoff.
Native counted work, actual reservation, finite-return of an invocation and
recurrent eligible lane service require separate implementation evidence.
These are finite visitation bounds, not unconditional fairness or termination.
-/
namespace FrozenRounds
set_option autoImplicit false
set_option warningAsError true

structure Round where
  next : Nat
  stop : Nat
  deriving DecidableEq, Repr

def remaining (r : Round) : Nat := r.stop - r.next

def beginRound (r : Round) (members : Nat) : Round :=
  if r.next = r.stop then ⟨0, members⟩ else r

/-- `nativePaid` comes from the completed counted invocation. A zero-work
invocation must actually consume one unit of its remaining grant to rotate. -/
def handoff (r : Round) (nativePaid grant : Nat) : Option (Round × Nat) :=
  if r.next < r.stop then
    if 0 < nativePaid then some (⟨r.next + 1, r.stop⟩, grant)
    else if 0 < grant then some (⟨r.next + 1, r.stop⟩, grant - 1)
    else none
  else none

/-- Speculative restart metadata is not published on refusal. This models only
the round cursor and remaining grant, not VM mutation or wake activation. -/
def attempt (r : Round) (members nativePaid grant : Nat) : Round × Nat :=
  (handoff (beginRound r members) nativePaid grant).getD (r, grant)

theorem refused_attempt_preserves_snapshot (r : Round) (members paid grant : Nat)
    (refused : handoff (beginRound r members) paid grant = none) :
    attempt r members paid grant = (r, grant) := by
  simp [attempt, refused]

theorem committed_attempt_publishes_handoff (r after : Round)
    (members paid grant rest : Nat)
    (admitted : handoff (beginRound r members) paid grant = some (after, rest)) :
    attempt r members paid grant = (after, rest) := by
  simp [attempt, admitted]

theorem unpaid_attempt_preserves_snapshot (r : Round) (members : Nat) :
    attempt r members 0 0 = (r, 0) := by
  simp [attempt, handoff]

theorem paid_restart_visits_first_slot (r : Round) (members paid grant : Nat)
    (complete : r.next = r.stop) (present : 0 < members) (positive : 0 < paid) :
    attempt r members paid grant = (⟨1, members⟩, grant) := by
  simp [attempt, beginRound, complete, handoff, present, positive]

theorem routing_restart_visits_first_slot (r : Round) (members grant : Nat)
    (complete : r.next = r.stop) (present : 0 < members) (positive : 0 < grant) :
    attempt r members 0 grant = (⟨1, members⟩, grant - 1) := by
  simp [attempt, beginRound, complete, handoff, present, positive]

theorem unfinished_snapshot_stable (r : Round) (members : Nat)
    (unfinished : r.next < r.stop) : beginRound r members = r := by
  simp [beginRound, Nat.ne_of_lt unfinished]

theorem new_arrivals_do_not_extend (r : Round) (members arrivals : Nat)
    (unfinished : r.next < r.stop) : beginRound r (members + arrivals) = r :=
  unfinished_snapshot_stable _ _ unfinished

theorem completed_round_includes_new_slots (r : Round) (members slot : Nat)
    (complete : r.next = r.stop) (present : slot < members) :
    (beginRound r members).next ≤ slot ∧ slot < (beginRound r members).stop := by
  simp [beginRound, complete, present]

/-- The list projects stable slot identities, not mutable VM payloads. -/
theorem appended_slot_identity {A : Type} (owners arrivals : List A) (slot : Nat)
    (present : slot < owners.length) : (owners ++ arrivals)[slot]? = owners[slot]? := by
  simp [List.getElem?_append, present]

theorem unpaid_cannot_advance (r : Round) : handoff r 0 0 = none := by
  simp [handoff]

theorem native_work_pays_visit (r : Round) (paid grant : Nat)
    (unfinished : r.next < r.stop) (positive : 0 < paid) :
    handoff r paid grant = some (⟨r.next + 1, r.stop⟩, grant) := by
  simp [handoff, unfinished, positive]

theorem routing_consumes_unit (r : Round) (grant : Nat)
    (unfinished : r.next < r.stop) (positive : 0 < grant) :
    handoff r 0 grant = some (⟨r.next + 1, r.stop⟩, grant - 1) := by
  simp [handoff, unfinished, positive]

theorem admitted_exact (r after : Round) (paid grant rest : Nat)
    (admitted : handoff r paid grant = some (after, rest)) :
    after.next = r.next + 1 ∧ after.stop = r.stop ∧ r.next < r.stop := by
  unfold handoff at admitted
  split at admitted
  · split at admitted
    · cases admitted; simp_all
    · split at admitted
      · cases admitted; simp_all
      · contradiction
  · contradiction

theorem admitted_decreases_rank (r after : Round) (paid grant rest : Nat)
    (admitted : handoff r paid grant = some (after, rest)) :
    remaining after + 1 = remaining r := by
  have := admitted_exact _ _ _ _ _ admitted
  unfold remaining
  omega

theorem admitted_within_word (r after : Round) (paid grant rest wordMax : Nat)
    (admitted : handoff r paid grant = some (after, rest)) (bounded : r.stop ≤ wordMax) :
    after.next ≤ after.stop ∧ after.stop ≤ wordMax := by
  obtain ⟨next, stop, unfinished⟩ := admitted_exact _ _ _ _ _ admitted
  constructor
  · rw [next, stop]
    exact Nat.succ_le_of_lt unfinished
  · rw [stop]
    exact bounded

/-- Only actual admitted handoffs contribute slots. Unpaid scheduler retries,
arrivals and other lanes may occur between these visits; they do not reset or
extend this frozen round. Establishing such a trace from Rust is separate. -/
inductive Visits : Round → List Nat → Round → Prop where
  | nil (r : Round) : Visits r [] r
  | cons {r after final : Round} {paid grant rest : Nat} {slots : List Nat}
      (admitted : handoff r paid grant = some (after, rest))
      (tail : Visits after slots final) : Visits r (r.next :: slots) final

theorem visits_exact {r final : Round} {slots : List Nat} (visits : Visits r slots final) :
    final.next = r.next + slots.length ∧ final.stop = r.stop := by
  induction visits with
  | nil => exact ⟨rfl, rfl⟩
  | cons admitted _ ih =>
      have step := admitted_exact _ _ _ _ _ admitted
      constructor
      · simp only [List.length_cons]
        rw [ih.1, step.1]
        exact (Nat.add_assoc _ _ _).trans (congrArg (_ + ·) (Nat.add_comm _ _))
      · exact ih.2.trans step.2.1

theorem visited_slots_exact {r final : Round} {slots : List Nat}
    (visits : Visits r slots final) (slot : Nat) :
    slot ∈ slots ↔ r.next ≤ slot ∧ slot < final.next := by
  induction visits with
  | nil => simp
  | @cons r after final paid grant rest slots admitted tail ih =>
      have step := admitted_exact _ _ _ _ _ admitted
      have count := visits_exact tail
      have progressed : after.next ≤ final.next := by
        rw [count.1]
        exact Nat.le_add_right _ _
      simp only [List.mem_cons, ih]
      constructor
      · rintro (equal | ⟨lower, upper⟩)
        · subst slot
          exact ⟨Nat.le_refl _, Nat.lt_of_lt_of_le (step.1 ▸ Nat.lt_succ_self _) progressed⟩
        · exact ⟨Nat.le_trans (step.1 ▸ Nat.le_succ _) lower, upper⟩
      · rintro ⟨lower, upper⟩
        rcases Nat.eq_or_lt_of_le lower with equal | strict
        · exact Or.inl equal.symm
        · exact Or.inr ⟨step.1 ▸ Nat.succ_le_of_lt strict, upper⟩

theorem fixed_slot_bound {r final : Round} {slots : List Nat}
    (visits : Visits r slots final) (slot : Nat) (ahead : r.next ≤ slot)
    (enough : slot - r.next < slots.length) : slot ∈ slots := by
  rw [visited_slots_exact visits]
  constructor
  · exact ahead
  · rw [(visits_exact visits).1]
    have upper := Nat.add_lt_add_left enough r.next
    rw [Nat.add_sub_of_le ahead] at upper
    exact upper

theorem frozen_round_completed {r final : Round} {slots : List Nat}
    (visits : Visits r slots final) (valid : r.next ≤ r.stop)
    (enough : slots.length = remaining r) : final.next = final.stop := by
  rw [(visits_exact visits).1, (visits_exact visits).2, enough]
  exact Nat.add_sub_of_le valid

/-- Contract supplied by recurrent eligible service with returning native
invocations and usable grants. It does not assert that a VM computation ends.
The native scheduler must establish this contract; the cursor alone cannot. -/
def SufficientService (r : Round) : Prop :=
  ∀ count, count ≤ remaining r →
    ∃ slots final, Visits r slots final ∧ slots.length = count

theorem serviced_slot (r : Round) (slot : Nat) (ahead : r.next ≤ slot)
    (present : slot < r.stop) (service : SufficientService r) :
    ∃ slots final, Visits r slots final ∧ slots.length = slot - r.next + 1 ∧ slot ∈ slots := by
  have bound : slot - r.next + 1 ≤ remaining r :=
    Nat.succ_le_of_lt (Nat.sub_lt_sub_right ahead present)
  obtain ⟨slots, final, visits, count⟩ := service _ bound
  exact ⟨slots, final, visits, count,
    fixed_slot_bound visits slot ahead (by rw [count]; exact Nat.lt_succ_self _)⟩

end FrozenRounds
