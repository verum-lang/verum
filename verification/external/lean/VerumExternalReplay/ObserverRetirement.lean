import Lean

/- Conditional transition contract, not a native implementation certificate.
   Language identifies the vocabulary and epoch; each layer holds its CURRENT
   observer state separately. Physical includes
   the complete committed world/action/outcome/boundary/stamp record. Layers
   retain exact prior vocabulary/epoch identities in installation order. Their fidelity, receipt
   production, work accounting and atomicity require independent native evidence.
   A rejected receipt supplies no retirement transition; it does not refund work.
   Post-action callers must supply the already committed physical record. -/
namespace ObserverRetirement
set_option warningAsError true

inductive Failure where
  | undefined | resource | foreignSource
  deriving DecidableEq

structure Receipt (Owner : Type) where
  owner : Owner
  cut : Nat
  failed : Nat
  reason : Failure
  remaining : Nat
  deriving DecidableEq

structure Layer (Language Observer : Type) where
  prior : Language
  observer : Observer
  deriving DecidableEq

structure State (Owner Language Observer Physical : Type) where
  owner : Owner
  cut : Nat
  language : Language
  layers : List (Layer Language Observer)
  physical : Physical
  work : Nat
  receipts : List (Receipt Owner)
  capacity : Nat
  deriving DecidableEq

variable {O L S P C V : Type} [DecidableEq O]

def Admissible (s : State O L S P) (r : Receipt O) : Prop :=
  r.owner = s.owner ∧ r.cut = s.cut ∧ r.reason = .undefined ∧
  r.remaining < s.work ∧ s.receipts.length < s.capacity

instance (s : State O L S P) (r : Receipt O) : Decidable (Admissible s r) :=
  inferInstanceAs (Decidable (_ ∧ _ ∧ _ ∧ _ ∧ _))

def restored (s : State O L S P) (r : Receipt O) (prior : L) : State O L S P :=
  { s with
    cut := s.cut + 1
    language := prior
    layers := s.layers.take r.failed
    work := r.remaining
    receipts := s.receipts ++ [r] }

def retire (s : State O L S P) (r : Receipt O) : Option (State O L S P) :=
  if Admissible s r then s.layers[r.failed]?.map (fun layer => restored s r layer.prior) else none

theorem success_exact (s next : State O L S P) (r : Receipt O) :
    retire s r = some next ↔ Admissible s r ∧
      ∃ prior, s.layers[r.failed]? = some prior ∧ restored s r prior.prior = next := by
  simp only [retire]
  split <;> simp_all [Option.map_eq_some_iff]

theorem resource_not_negative (s : State O L S P) (r : Receipt O)
    (reason : r.reason = .resource) : retire s r = none := by
  simp [retire, Admissible, reason]

theorem foreign_source_not_negative (s : State O L S P) (r : Receipt O)
    (reason : r.reason = .foreignSource) : retire s r = none := by
  simp [retire, Admissible, reason]

theorem foreign_owner_refused (s : State O L S P) (r : Receipt O)
    (foreign : r.owner ≠ s.owner) : retire s r = none := by
  simp [retire, Admissible, foreign]

theorem stale_cut_refused (s : State O L S P) (r : Receipt O)
    (stale : r.cut ≠ s.cut) : retire s r = none := by
  simp [retire, Admissible, stale]

theorem full_receipts_refused (s : State O L S P) (r : Receipt O)
    (full : s.capacity ≤ s.receipts.length) : retire s r = none := by
  simp [retire, Admissible, Nat.not_lt_of_ge full]

theorem no_budget_reset (s : State O L S P) (r : Receipt O)
    (reset : s.work ≤ r.remaining) : retire s r = none := by
  simp [retire, Admissible, Nat.not_lt_of_ge reset]

theorem physical_conservation (s next : State O L S P) (r : Receipt O)
    (h : retire s r = some next) : next.physical = s.physical := by
  obtain ⟨_, prior, _, rfl⟩ := (success_exact s next r).mp h
  rfl

theorem suffix_rollback (s next : State O L S P) (r : Receipt O)
    (h : retire s r = some next) : next.layers = s.layers.take r.failed ∧
      ∃ layer, s.layers[r.failed]? = some layer ∧ next.language = layer.prior := by
  obtain ⟨_, prior, found, rfl⟩ := (success_exact s next r).mp h
  exact ⟨rfl, prior, found, rfl⟩

theorem strictly_fewer_layers (s next : State O L S P) (r : Receipt O)
    (h : retire s r = some next) : next.layers.length < s.layers.length := by
  obtain ⟨_, prior, found, rfl⟩ := (success_exact s next r).mp h
  obtain ⟨bound, _⟩ := List.getElem?_eq_some_iff.mp found
  simpa [restored, List.length_take, Nat.min_eq_left (Nat.le_of_lt bound)] using bound

theorem strictly_decreasing_work (s next : State O L S P) (r : Receipt O)
    (h : retire s r = some next) : next.work < s.work := by
  obtain ⟨valid, prior, _, rfl⟩ := (success_exact s next r).mp h
  exact valid.2.2.2.1

theorem bounded_receipts (s next : State O L S P) (r : Receipt O)
    (h : retire s r = some next) : next.receipts.length ≤ next.capacity := by
  obtain ⟨valid, prior, _, rfl⟩ := (success_exact s next r).mp h
  simpa [restored] using Nat.succ_le_of_lt valid.2.2.2.2

theorem receipt_retained (s next : State O L S P) (r : Receipt O)
    (h : retire s r = some next) : next.receipts = s.receipts ++ [r] := by
  obtain ⟨_, prior, _, rfl⟩ := (success_exact s next r).mp h
  rfl

theorem receipt_cannot_retire_twice (s next : State O L S P) (r : Receipt O)
    (h : retire s r = some next) : retire next r = none := by
  obtain ⟨valid, prior, _, rfl⟩ := (success_exact s next r).mp h
  apply stale_cut_refused
  change r.cut ≠ s.cut + 1
  rw [valid.2.1]
  exact Nat.ne_of_lt (Nat.lt_succ_self _)

-- Retrying selection/prepare is conditional on its availability in the exact
-- restored language and the remaining budget. No claim of total acquisition.
theorem conservative_retry (s next : State O L S P) (r : Receipt O) (prior : Layer L S)
    (h : retire s r = some next) (saved : s.layers[r.failed]? = some prior)
    (prepare : L → List (Layer L S) → P → Nat → Option C) (candidate : C)
    (available : prepare prior.prior (s.layers.take r.failed) s.physical r.remaining = some candidate) :
    prepare next.language next.layers next.physical next.work = some candidate := by
  obtain ⟨_, old, found, rfl⟩ := (success_exact s next r).mp h
  have same : old = prior := Option.some.inj (found.symm.trans saved)
  simpa [restored, same] using available

def replay : State O L S P → List (Receipt O) → Option (State O L S P)
  | s, [] => some s
  | s, r :: rs => (retire s r).bind (fun next => replay next rs)

theorem replay_conserves_physical (s next : State O L S P) (rs : List (Receipt O))
    (h : replay s rs = some next) : next.physical = s.physical := by
  induction rs generalizing s with
  | nil => simpa [replay] using congrArg (Option.map State.physical) h.symm
  | cons r rs ih =>
    obtain ⟨middle, step, rest⟩ := Option.bind_eq_some_iff.mp h
    exact (ih middle rest).trans (physical_conservation s middle r step)

theorem replay_retains_receipts (s next : State O L S P) (rs : List (Receipt O))
    (h : replay s rs = some next) : next.receipts = s.receipts ++ rs := by
  induction rs generalizing s with
  | nil => simp only [replay, Option.some.injEq] at h; subst next; simp
  | cons r rs ih =>
    obtain ⟨middle, step, rest⟩ := Option.bind_eq_some_iff.mp h
    rw [ih middle rest, receipt_retained s middle r step]
    simp [List.append_assoc]

-- Every retry spends work from the same allowance. In particular, an accepted
-- chain cannot be infinite, even if a caller keeps producing semantic failures.
theorem retry_count_bounded (s next : State O L S P) (rs : List (Receipt O))
    (h : replay s rs = some next) : next.work + rs.length ≤ s.work := by
  induction rs generalizing s with
  | nil => simp only [replay, Option.some.injEq] at h; subst next; simp
  | cons r rs ih =>
    obtain ⟨middle, step, rest⟩ := Option.bind_eq_some_iff.mp h
    have tail := ih middle rest
    have decrease := strictly_decreasing_work s middle r step
    simp only [List.length_cons]
    omega

-- Original cut is retained across rollback. Both projections use the restored
-- language (including epoch); next uses committed observer/physical state.
-- This function has no physical-action or observer-update callback.
def edge (s : State O L S P) (original : C)
    (input : L → C → Option V) (readNext : L → List (Layer L S) → P → Option V) : Option (V × V) := do
  let before ← input s.language original
  let after ← readNext s.language s.layers s.physical
  pure (before, after)

omit [DecidableEq O] in
theorem edge_exact (s : State O L S P) (original : C)
    (input : L → C → Option V) (readNext : L → List (Layer L S) → P → Option V) (before after : V) :
    edge s original input readNext = some (before, after) ↔
      input s.language original = some before ∧
      readNext s.language s.layers s.physical = some after := by
  simp [edge, Option.bind_eq_some_iff, Prod.mk.injEq]

omit [DecidableEq O] in
theorem missing_prior_no_edge (s : State O L S P) (original : C)
    (input : L → C → Option V) (readNext : L → List (Layer L S) → P → Option V)
    (missing : input s.language original = none) : edge s original input readNext = none := by
  simp [edge, missing]

omit [DecidableEq O] in
theorem missing_next_no_edge (s : State O L S P) (original : C)
    (input : L → C → Option V) (readNext : L → List (Layer L S) → P → Option V)
    (missing : readNext s.language s.layers s.physical = none) : edge s original input readNext = none := by
  simp [edge, missing]

end ObserverRetirement
