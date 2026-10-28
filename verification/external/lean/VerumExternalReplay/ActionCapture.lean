import VerumExternalReplay.SensoryPort

/-! An action's received observation is distinct from a subsequently resumed
birth state. Eligibility never establishes availability or physical fidelity.
Failure detail retains the complete stage/context/cause without interpreting it.
Lawful action equality, dispatch-cut ownership, adapter timing and clearing the
native per-invocation slot remain correspondence obligations. Equal payloads
are not nominal identities or a second execution/learning authority. -/
namespace ActionCapture
open AcquiredObserver
set_option warningAsError true

inductive Resumption where
  | resumed | stopped
  deriving DecidableEq

inductive Observed where
  | step | refused
  | terminal : Terminal → Resumption → Observed
  deriving DecidableEq

inductive Operation (Action : Type) where
  | act : Action → Operation Action
  | begin | checkpoint | restore
  deriving DecidableEq

inductive TerminalScope where
  | episode | action
  deriving DecidableEq

structure Failure (Action Detail : Type) where
  operation : Operation Action
  terminal : Option Terminal
  scope : TerminalScope
  detail : Detail

abbrev Result (Action Detail : Type) := Except (Failure Action Detail) Observed

structure Capture (Action Detail : Type) where
  action : Action
  result : Result Action Detail
  observation : Option Sensory
  resumed : Option Sensory

inductive Refusal where
  | unownedObservation | unexpectedResumption | contradictorySuccessor
  deriving DecidableEq

variable {Action Detail Cut : Type}

def observationEligible [DecidableEq Action] (action : Action) : Result Action Detail → Bool
  | .ok _ => true
  | .error failure => decide (failure.scope = .action ∧
      failure.operation = .act action ∧ failure.terminal.isSome = true)

def resumptionEligible : Result Action Detail → Bool
  | .ok (.terminal _ .resumed) => true
  | _ => false

def Valid [DecidableEq Action] (c : Capture Action Detail) : Prop :=
  (c.observation.isSome = true → observationEligible c.action c.result = true) ∧
  (c.resumed.isSome = true → resumptionEligible c.result = true)

/-- Refusal order follows the constructor: invalid observation before invalid
resumption. No input is normalized, substituted or silently discarded. -/
def check [DecidableEq Action] (c : Capture Action Detail) : Except Refusal (Capture Action Detail) :=
  if c.observation.isSome && !observationEligible c.action c.result then
    .error .unownedObservation
  else if c.resumed.isSome && !resumptionEligible c.result then
    .error .unexpectedResumption
  else .ok c

def unavailable (action : Action) (result : Result Action Detail) : Capture Action Detail :=
  ⟨action, result, none, none⟩

/-- Only the explicitly supplied outcome is a learning target. This accessor
does not validate a raw capture; accepted captures require `check` evidence. -/
def targetObservation (c : Capture Action Detail) : Option Sensory := c.observation

theorem check_success_iff [DecidableEq Action] (c accepted : Capture Action Detail) :
    check c = .ok accepted ↔ c = accepted ∧ Valid c := by
  cases ho : c.observation.isSome <;> cases he : observationEligible c.action c.result <;>
    cases hr : c.resumed.isSome <;> cases hc : resumptionEligible c.result <;>
    simp_all [check, Valid]

theorem unavailable_accepted [DecidableEq Action] (action : Action) (result : Result Action Detail) :
    check (unavailable action result) = .ok (unavailable action result) := by
  simp [check, unavailable]

theorem accepted_preserves_every_field [DecidableEq Action]
    (c accepted : Capture Action Detail) (h : check c = .ok accepted) : accepted = c :=
  ((check_success_iff c accepted).mp h).1.symm

theorem error_target_requires_exact_action [DecidableEq Action]
    (action : Action) (failure : Failure Action Detail) (body : Sensory)
    (resumed : Option Sensory) (accepted : Capture Action Detail)
    (h : check ⟨action, .error failure, some body, resumed⟩ = .ok accepted) :
    failure.scope = .action ∧ failure.operation = .act action ∧ failure.terminal.isSome = true := by
  have valid := ((check_success_iff _ _).mp h).2.1 (by rfl)
  simpa [observationEligible] using valid

theorem resumed_requires_successful_restart [DecidableEq Action]
    (c accepted : Capture Action Detail) (body : Sensory)
    (present : c.resumed = some body) (h : check c = .ok accepted) :
    ∃ kind, c.result = .ok (.terminal kind .resumed) := by
  have eligible := ((check_success_iff _ _).mp h).2.2 (by simp [present])
  cases hr : c.result with
  | error failure => simp [resumptionEligible, hr] at eligible
  | ok result =>
    cases result with
    | step => simp [resumptionEligible, hr] at eligible
    | refused => simp [resumptionEligible, hr] at eligible
    | terminal kind state =>
      cases state with
      | stopped => simp [resumptionEligible, hr] at eligible
      | resumed => exact ⟨kind, rfl⟩

theorem action_terminal_failure_keeps_outcome [DecidableEq Action]
    (action : Action) (kind : Terminal) (detail : Detail) (body : Option Sensory) :
    check ⟨action, .error ⟨.act action, some kind, .action, detail⟩, body, none⟩ =
      .ok ⟨action, .error ⟨.act action, some kind, .action, detail⟩, body, none⟩ := by
  simp [check, observationEligible]

theorem failed_restart_cannot_have_birth [DecidableEq Action]
    (action : Action) (kind : Terminal) (detail : Detail) (body : Option Sensory) (birth : Sensory) :
    check ⟨action, .error ⟨.act action, some kind, .action, detail⟩, body, some birth⟩ =
      .error .unexpectedResumption := by
  simp [check, observationEligible, resumptionEligible]

theorem missing_outcome_is_not_birth (c : Capture Action Detail) (birth : Option Sensory) :
    targetObservation {c with observation := none, resumed := birth} = none := rfl

theorem target_independent_of_resumption (c : Capture Action Detail) (birth : Option Sensory) :
    targetObservation {c with resumed := birth} = targetObservation c := rfl

theorem resumption_independent_of_target (c : Capture Action Detail) (body : Option Sensory) :
    ({c with observation := body} : Capture Action Detail).resumed = c.resumed := rfl

/-- A legacy current frame is an outcome only for continuing Step/Refused.
Terminal bytes are unavailable even for a stopped terminal; current may be old.
A successfully resumed terminal exposes current only as the separate birth. -/
def legacy (action : Action) (result : Result Action Detail) (current : Option Sensory) :
    Capture Action Detail :=
  ⟨action, result, (match result with | .ok .step | .ok .refused => current | _ => none),
    if resumptionEligible result then current else none⟩

theorem legacy_accepted [DecidableEq Action]
    (action : Action) (result : Result Action Detail) (current : Option Sensory) :
    check (legacy action result current) = .ok (legacy action result current) := by
  cases result with
  | error failure => simp [legacy, check, resumptionEligible]
  | ok result =>
    cases result with
    | step => simp [legacy, check, observationEligible, resumptionEligible]
    | refused => simp [legacy, check, observationEligible, resumptionEligible]
    | terminal kind state => cases state <;>
        simp [legacy, check, observationEligible, resumptionEligible]

theorem legacy_terminal_has_no_target (action : Action) (kind : Terminal)
    (state : Resumption) (current : Option Sensory) :
    targetObservation (legacy (Detail := Detail) action (.ok (.terminal kind state)) current) = none := rfl

theorem legacy_failure_has_no_captures (action : Action) (failure : Failure Action Detail)
    (current : Option Sensory) :
    legacy action (.error failure) current = unavailable action (.error failure) := rfl

/-- Complete semantic targets retain the observed kind and exact sensory data.
Resumption is not an outcome kind. Wire lengths/allocation are a later boundary. -/
inductive TargetKind where
  | step | refused
  | terminal : Terminal → TargetKind
  deriving DecidableEq

structure Target where
  kind : TargetKind
  observation : Sensory
  deriving DecidableEq

def observedKind : Observed → TargetKind
  | .step => .step
  | .refused => .refused
  | .terminal kind _ => .terminal kind

def targetKind [DecidableEq Action] (action : Action) : Result Action Detail → Option TargetKind
  | .ok result => some (observedKind result)
  | .error failure => if observationEligible action (.error failure) then
      failure.terminal.map TargetKind.terminal else none

inductive TargetRefusal where
  | unobserved | missing
  deriving DecidableEq

/-- This semantic projection precedes wire materialization, which can separately
fail capacity/allocation. It never samples a current or resumed observation. -/
def semanticTarget [DecidableEq Action] (c : Capture Action Detail) : Except TargetRefusal Target :=
  match targetKind c.action c.result with
  | none => .error .unobserved
  | some kind => match c.observation with
    | none => .error .missing
    | some body => .ok ⟨kind, body⟩

theorem semantic_target_ignores_resumption [DecidableEq Action]
    (c : Capture Action Detail) (birth : Option Sensory) :
    semanticTarget {c with resumed := birth} = semanticTarget c := rfl

theorem failed_restart_target_keeps_kind [DecidableEq Action]
    (action : Action) (kind : Terminal) (detail : Detail) (body : Sensory) :
    semanticTarget ⟨action, .error ⟨.act action, some kind, .action, detail⟩, some body, none⟩ =
      .ok ⟨.terminal kind, body⟩ := by
  simp [semanticTarget, targetKind, observationEligible]

theorem absent_observation_cannot_materialize [DecidableEq Action]
    (c : Capture Action Detail) (candidate : Target) :
    semanticTarget {c with observation := none} ≠ .ok candidate := by
  cases hk : targetKind c.action c.result <;> simp [semanticTarget, hk]

theorem resumption_status_does_not_change_terminal_target [DecidableEq Action]
    (action : Action) (kind : Terminal) (body birth : Option Sensory) (first second : Resumption) :
    semanticTarget (Detail := Detail) ⟨action, .ok (.terminal kind first), body, birth⟩ =
      semanticTarget (Detail := Detail) ⟨action, .ok (.terminal kind second), body, birth⟩ := rfl

def successor (c : Capture Action Detail) : Option Sensory :=
  match c.result with
  | .ok .step | .ok .refused => c.observation
  | .ok (.terminal _ .resumed) => c.resumed
  | _ => none

/-- Missing data establish no equality. Two present successor observations must
agree exactly, including schema. This guard neither fills a slot nor learns. -/
def checkSuccessor (c : Capture Action Detail) (actual : Option Sensory) : Except Refusal Unit :=
  match successor c, actual with
  | some captured, some current =>
    if captured = current then .ok () else .error .contradictorySuccessor
  | _, _ => .ok ()

theorem successor_guard_exact (c : Capture Action Detail) (actual : Option Sensory) :
    checkSuccessor c actual = .ok () ↔
      ∀ captured current, successor c = some captured → actual = some current → captured = current := by
  cases hs : successor c <;> cases actual <;> simp [checkSuccessor, hs]

theorem failure_has_no_successor (action : Action) (failure : Failure Action Detail)
    (body birth : Option Sensory) :
    successor ⟨action, .error failure, body, birth⟩ = none := rfl

theorem successful_restart_uses_birth (action : Action) (kind : Terminal)
    (body birth : Option Sensory) :
    successor (Detail := Detail) ⟨action, .ok (.terminal kind .resumed), body, birth⟩ = birth := rfl

theorem unavailable_current_does_not_refute (c : Capture Action Detail) :
    checkSuccessor c none = .ok () := by
  cases hs : successor c <;> simp [checkSuccessor, hs]

structure Request (Action Cut : Type) where
  cut : Cut
  before : Event
  action : Action

structure Retained (Action Cut Detail : Type) where
  request : Request Action Cut
  capture : Capture Action Detail

inductive AttachmentRefusal where
  | foreignAction
  deriving DecidableEq

/-- Constructor validity and dispatch binding are distinct obligations. The
actual core supplies a constructor-validated capture and retains its own cut. -/
def attach [DecidableEq Action] (request : Request Action Cut) (capture : Capture Action Detail) :
    Except AttachmentRefusal (Retained Action Cut Detail) :=
  if capture.action = request.action then .ok ⟨request, capture⟩ else .error .foreignAction

theorem foreign_action_refused [DecidableEq Action] (request : Request Action Cut)
    (capture : Capture Action Detail) (foreign : capture.action ≠ request.action) :
    attach request capture = .error .foreignAction := by
  simp [attach, foreign]

theorem attached_request_and_capture_exact [DecidableEq Action] (request : Request Action Cut)
    (capture : Capture Action Detail) (retained : Retained Action Cut Detail)
    (h : attach request capture = .ok retained) :
    retained = ⟨request, capture⟩ ∧ capture.action = request.action := by
  simp only [attach] at h
  split at h <;> simp_all

def preActionQuery (encode : Action → UInt8) (request : Request Action Cut) : Query :=
  SensoryPort.queryAt request.before (some (encode request.action))

theorem query_retains_pre_action_sensory (encode : Action → UInt8) (request : Request Action Cut) :
    (preActionQuery encode request).sensory = request.before.sensory := rfl

theorem completion_cannot_change_query (encode : Action → UInt8) (request : Request Action Cut)
    (first second : Capture Action Detail) :
    preActionQuery encode (Retained.mk request first).request =
      preActionQuery encode (Retained.mk request second).request := rfl

theorem attached_cut_and_query_preserved [DecidableEq Action] (encode : Action → UInt8)
    (request : Request Action Cut) (capture : Capture Action Detail)
    (retained : Retained Action Cut Detail) (h : attach request capture = .ok retained) :
    retained.request.cut = request.cut ∧
      preActionQuery encode retained.request = preActionQuery encode request := by
  obtain ⟨rfl, _⟩ := attached_request_and_capture_exact request capture retained h
  exact ⟨rfl, rfl⟩

structure Slot (Action Cut : Type) where
  request : Request Action Cut
  observation : Option Sensory

/-- Starting an invocation discards the previous capture slot, including when
the next action has the same value. Native use of this transition is separate. -/
def begin (request : Request Action Cut) (_previous : Option Sensory) : Slot Action Cut :=
  ⟨request, none⟩

def record (slot : Slot Action Cut) (observation : Option Sensory) : Slot Action Cut :=
  {slot with observation := observation}

def finish [DecidableEq Action] (slot : Slot Action Cut) (result : Result Action Detail)
    (resumed : Option Sensory) : Except Refusal (Capture Action Detail) :=
  check ⟨slot.request.action, result, slot.observation, resumed⟩

theorem begin_forgets_prior_capture (request : Request Action Cut) (first second : Option Sensory) :
    begin request first = begin request second := rfl

theorem recorded_observation_is_exact (slot : Slot Action Cut) (body : Option Sensory) :
    (record slot body).request = slot.request ∧ (record slot body).observation = body := ⟨rfl, rfl⟩

theorem no_record_no_target [DecidableEq Action] (request : Request Action Cut)
    (previous resumed : Option Sensory) (result : Result Action Detail)
    (accepted : Capture Action Detail) (h : finish (begin request previous) result resumed = .ok accepted) :
    targetObservation accepted = none ∧ accepted.result = result ∧ accepted.action = request.action := by
  have same := accepted_preserves_every_field _ _ h
  rw [same]
  exact ⟨rfl, rfl, rfl⟩

end ActionCapture
