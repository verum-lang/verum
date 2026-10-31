import VerumExternalReplay.PhysicalClock
import VerumExternalReplay.ObserverOwnership

/-! Sufficient conditions for advancing an installed finite-phase observer
with receipts from the same physical dispatch. Identity carriers stand for
retained native owners, not equal metadata or values. An adapter must establish
that the private dispatch identity authenticates both receipts and the actual
input. This model supplies neither that Rust correspondence nor world closure.
-/
namespace SourceCycle
set_option warningAsError true
open ObserverOwnership

/-- A closed phase factor on an admitted invariant. Reads are autonomous over
every query, not merely equal at the current physical age. This need not be a
minimal observable period or a total update on inputs outside the invariant. -/
structure Cycle (V E Q R : Type) where
  period : Nat
  positive : 0 < period
  inside : V → Prop
  phase : V → Nat
  bounded : ∀ v, inside v → phase v < period
  update : V → E → Option V
  read : V → Q → Option R
  view : Nat → Option R
  advances : ∀ v event next, inside v → update v event = some next →
    inside next ∧ phase next = (phase v + 1) % period
  observes : ∀ v query, inside v → read v query = view (phase v)

structure Binding (Origin Definition Instance State : Type) where
  anchorPoint : PhysicalClock.Point Origin
  anchorCut : Cut Definition Instance State
  point : PhysicalClock.Point Origin
  current : Cut Definition Instance State
  ticks : Nat
  deriving DecidableEq

variable {O D I S T V E Q R : Type}

local instance [DecidableEq A] [DecidableEq B] : DecidableEq (Except A B)
  | .ok a, .ok b => decidable_of_iff (a = b) ⟨congrArg Except.ok, Except.ok.inj⟩
  | .error a, .error b => decidable_of_iff (a = b) ⟨congrArg Except.error, Except.error.inj⟩
  | .ok _, .error _ => .isFalse (by intro h; cases h)
  | .error _, .ok _ => .isFalse (by intro h; cases h)

structure Aligned (cycle : Cycle V E Q R) (value : S → V) (b : Binding O D I S) : Prop where
  anchorInside : cycle.inside (value b.anchorCut.state)
  currentInside : cycle.inside (value b.current.state)
  definition : b.current.definition = b.anchorCut.definition
  instanceOwner : b.current.owner = b.anchorCut.owner
  origin : b.point.origin = b.anchorPoint.origin
  episode : b.point.time.episode = b.anchorPoint.time.episode
  age : b.point.time.age = b.anchorPoint.time.age + b.ticks
  phase : cycle.phase (value b.current.state) =
    (cycle.phase (value b.anchorCut.state) + b.ticks) % cycle.period

def anchor (point : PhysicalClock.Point O) (cut : Cut D I S) : Binding O D I S :=
  ⟨point,cut,point,cut,0⟩

theorem anchor_aligned (cycle : Cycle V E Q R) (value : S → V)
    (point : PhysicalClock.Point O) (cut : Cut D I S) (inside : cycle.inside (value cut.state)) :
    Aligned cycle value (anchor point cut) where
  anchorInside := inside
  currentInside := inside
  definition := rfl
  instanceOwner := rfl
  origin := rfl
  episode := rfl
  age := (Nat.add_zero _).symm
  phase := by simpa [anchor] using (Nat.mod_eq_of_lt (cycle.bounded _ inside)).symm

/-- Readiness and finite time addressability are required when establishing
an anchor. The current cut is supplied by the actual installed owner. -/
def anchorAt (maximum : Nat) (clock : PhysicalClock.Clock O) (cut : Cut D I S) :
    Except PhysicalClock.Refusal (Binding O D I S) :=
  (PhysicalClock.point maximum clock).map (fun point => anchor point cut)

structure Pair (Origin Definition Instance State Ticket Event : Type) where
  physicalDispatch : Ticket
  observerDispatch : Ticket
  event : Event
  physical : PhysicalClock.Receipt Origin
  observer : Committed Definition Instance State
  deriving DecidableEq

/-- Authentic same-dispatch pairing is a native constructor obligation. These
equalities additionally bind exact source point, before/after owners, committed
clock state and semantic update; a matching sample number alone is insufficient. -/
structure Paired [DecidableEq O] [DecidableEq I] [DecidableEq S]
    (cycle : Cycle V E Q R) (value : S → V) (maximum : Nat)
    (clock after : PhysicalClock.Clock O) (b : Binding O D I S)
    (dispatch : T) (event : E) (pair : Pair O D I S T E) : Prop where
  physicalIdentity : pair.physicalDispatch = dispatch
  observerIdentity : pair.observerDispatch = dispatch
  input : pair.event = event
  ready : PhysicalClock.point maximum clock = .ok b.point
  captured : PhysicalClock.observe maximum b.point pair.physical.sample pair.physical.transition =
    some pair.physical
  clockCommitted : PhysicalClock.commit (PhysicalClock.start maximum clock b.point).1 pair.physical =
    some (after, .ok ())
  before : pair.observer.before = b.current
  observerCommitted : (ObserverOwnership.commit b.current ⟨b.current,pair.observer.after.state⟩).receipt =
    .ok pair.observer
  evaluated : cycle.update (value b.current.state) event = some (value pair.observer.after.state)

instance [DecidableEq O] [DecidableEq D] [DecidableEq I] [DecidableEq S]
    [DecidableEq T] [DecidableEq E] [DecidableEq V]
    (cycle : Cycle V E Q R) (value : S → V) (maximum : Nat)
    (clock after : PhysicalClock.Clock O) (b : Binding O D I S)
    (dispatch : T) (event : E) (pair : Pair O D I S T E) :
    Decidable (Paired cycle value maximum clock after b dispatch event pair) :=
  decidable_of_iff
    (pair.physicalDispatch = dispatch ∧ pair.observerDispatch = dispatch ∧ pair.event = event ∧
     PhysicalClock.point maximum clock = .ok b.point ∧
     PhysicalClock.observe maximum b.point pair.physical.sample pair.physical.transition = some pair.physical ∧
     PhysicalClock.commit (PhysicalClock.start maximum clock b.point).1 pair.physical = some (after,.ok ()) ∧
     pair.observer.before = b.current ∧
     (ObserverOwnership.commit b.current ⟨b.current,pair.observer.after.state⟩).receipt = .ok pair.observer ∧
     cycle.update (value b.current.state) event = some (value pair.observer.after.state))
    ⟨fun h => ⟨h.1,h.2.1,h.2.2.1,h.2.2.2.1,h.2.2.2.2.1,h.2.2.2.2.2.1,
      h.2.2.2.2.2.2.1,h.2.2.2.2.2.2.2.1,h.2.2.2.2.2.2.2.2⟩,
     fun h => ⟨h.physicalIdentity,h.observerIdentity,h.input,h.ready,h.captured,
      h.clockCommitted,h.before,h.observerCommitted,h.evaluated⟩⟩

def tickBinding (b : Binding O D I S) (committed : Committed D I S) : Binding O D I S :=
  ⟨b.anchorPoint,b.anchorCut,⟨b.point.origin,b.point.time.episode,b.point.time.age+1⟩,
    committed.after,b.ticks+1⟩

structure Advanced (Origin Definition Instance State : Type) where
  current : Cut Definition Instance State
  binding : Option (Binding Origin Definition Instance State)
  deriving DecidableEq

/-- This transports a validated pair. A terminal update still happened; only
the age anchor is invalidated. Rejected or missing pairs cannot justify a call. -/
def follow (b : Binding O D I S) (pair : Pair O D I S T E) : Advanced O D I S :=
  ⟨pair.observer.after, match pair.physical.transition with
    | .tick => some (tickBinding b pair.observer)
    | .terminal _ => none⟩

variable [DecidableEq O] [DecidableEq I] [DecidableEq S]

theorem paired_after_cut (cycle : Cycle V E Q R) (value : S → V) (maximum : Nat)
    (clock after : PhysicalClock.Clock O) (b : Binding O D I S) (dispatch : T) (event : E)
    (pair : Pair O D I S T E) (h : Paired cycle value maximum clock after b dispatch event pair) :
    pair.observer.after = {b.current with state := pair.observer.after.state} := by
  have hc := h.observerCommitted
  simp only [ObserverOwnership.commit, ne_eq, not_true_eq_false, ↓reduceIte] at hc
  exact (congrArg Committed.after (Except.ok.inj hc)).symm

private theorem tick_aligned (cycle : Cycle V E Q R) (value : S → V) (maximum : Nat)
    (clock after : PhysicalClock.Clock O) (b : Binding O D I S) (dispatch : T) (event : E)
    (pair : Pair O D I S T E) (valid : Aligned cycle value b)
    (h : Paired cycle value maximum clock after b dispatch event pair) :
    Aligned cycle value (tickBinding b pair.observer) := by
  have update := cycle.advances _ event _ valid.currentInside h.evaluated
  have cut := paired_after_cut cycle value maximum clock after b dispatch event pair h
  refine ⟨valid.anchorInside, update.1, ?_, ?_, valid.origin, valid.episode, ?_, ?_⟩
  · exact (congrArg Cut.definition cut).trans valid.definition
  · exact (congrArg Cut.owner cut).trans valid.instanceOwner
  · exact (congrArg (·+1) valid.age).trans (Nat.add_assoc _ _ _)
  · change cycle.phase (value pair.observer.after.state) = _
    rw [update.2, valid.phase, Nat.mod_add_mod]
    simp [tickBinding, Nat.add_assoc]

/-- The transported tick point is the actually committed clock coordinate,
even at the last age where a subsequent point request will be refused. -/
theorem tick_clock_correspondence (cycle : Cycle V E Q R) (value : S → V) (maximum : Nat)
    (clock after : PhysicalClock.Clock O) (b : Binding O D I S) (dispatch : T) (event : E)
    (pair : Pair O D I S T E) (h : Paired cycle value maximum clock after b dispatch event pair)
    (tick : pair.physical.transition = .tick) :
    after.origin = (tickBinding b pair.observer).point.origin ∧
    after.episode = some (tickBinding b pair.observer).point.time.episode ∧
    after.age = (tickBinding b pair.observer).point.time.age ∧ after.phase = .ready := by
  have canonical := PhysicalClock.captured_tick_advances_once maximum clock b.point pair.physical.sample h.ready
  have capture := h.captured
  rw [tick, canonical.1] at capture
  have pairEq := Option.some.inj capture
  have committed := h.clockCommitted
  rw [← pairEq, canonical.2] at committed
  have clockEq := congrArg Prod.fst (Option.some.inj committed)
  have ready := (PhysicalClock.point_exact maximum clock b.point).mp h.ready
  cases clockEq
  exact ⟨ready.2.2.1, ready.2.1, congrArg (·+1) ready.2.2.2.1, rfl⟩

theorem tick_preserves_binding (cycle : Cycle V E Q R) (value : S → V) (maximum : Nat)
    (clock after : PhysicalClock.Clock O) (b : Binding O D I S) (dispatch : T) (event : E)
    (pair : Pair O D I S T E) (valid : Aligned cycle value b)
    (h : Paired cycle value maximum clock after b dispatch event pair)
    (tick : pair.physical.transition = .tick) :
    follow b pair = ⟨pair.observer.after,some (tickBinding b pair.observer)⟩ ∧
    Aligned cycle value (tickBinding b pair.observer) ∧
    after.age = b.point.time.age+1 ∧ after.phase = .ready := by
  have hc := tick_clock_correspondence cycle value maximum clock after b dispatch event pair h tick
  exact ⟨by simp [follow,tick], tick_aligned cycle value maximum clock after b dispatch event pair valid h,
    hc.2.2.1,hc.2.2.2⟩

theorem foreign_dispatch_refused (cycle : Cycle V E Q R) (value : S → V) (maximum : Nat)
    (clock after : PhysicalClock.Clock O) (b : Binding O D I S) (dispatch : T) (event : E)
    (pair : Pair O D I S T E) (foreign : pair.physicalDispatch ≠ pair.observerDispatch) :
    ¬ Paired cycle value maximum clock after b dispatch event pair := by
  intro h
  exact foreign (h.physicalIdentity.trans h.observerIdentity.symm)

omit [DecidableEq O] [DecidableEq I] [DecidableEq S] in
theorem terminal_keeps_update (b : Binding O D I S) (pair : Pair O D I S T E)
    (kind : PhysicalClock.TerminalKind) (terminal : pair.physical.transition = .terminal kind) :
    follow b pair = ⟨pair.observer.after, none⟩ := by simp [follow, terminal]

theorem terminal_clock_correspondence (cycle : Cycle V E Q R) (value : S → V) (maximum : Nat)
    (clock after : PhysicalClock.Clock O) (b : Binding O D I S) (dispatch : T) (event : E)
    (pair : Pair O D I S T E) (h : Paired cycle value maximum clock after b dispatch event pair)
    (kind : PhysicalClock.TerminalKind) (terminal : pair.physical.transition = .terminal kind) :
    after.age = 0 ∧ after.episode = clock.episode ∧ after.phase = .terminalPending := by
  have source := (PhysicalClock.observed_receipt_retains_source maximum b.point pair.physical.sample
    pair.physical.transition pair.physical h.captured).1
  have ready := (PhysicalClock.point_exact maximum clock b.point).mp h.ready
  have owns : PhysicalClock.owns (PhysicalClock.start maximum clock b.point).1 pair.physical := by
    simp [PhysicalClock.start,h.ready,PhysicalClock.owns,source,ready.2.1,ready.2.2.1,ready.2.2.2.1]
  have expected := PhysicalClock.terminal_commit_retains_episode
    (PhysicalClock.start maximum clock b.point).1 pair.physical kind owns terminal
  have actual := h.clockCommitted
  rw [expected] at actual
  have eq := congrArg Prod.fst (Option.some.inj actual)
  cases eq
  exact ⟨rfl,by simp [PhysicalClock.start,h.ready],rfl⟩

omit [DecidableEq O] [DecidableEq I] [DecidableEq S] in
/-- Re-anchoring establishes a new relative offset from the actual retained
state. It cannot turn that state's phase into zero by changing metadata. -/
theorem reanchor_retains_phase (cycle : Cycle V E Q R) (value : S → V)
    (point : PhysicalClock.Point O) (cut : Cut D I S) :
    cycle.phase (value (anchor point cut).current.state) = cycle.phase (value cut.state) := rfl

def beginRun (maximum : Nat) (fresh : O) (journal : Option PhysicalClock.JournalGate)
    (clock : PhysicalClock.Clock O) (current : Cut D I S) (binding : Option (Binding O D I S)) :=
  PhysicalClock.beginRun maximum fresh journal clock (current,binding)
    (fun payload => (payload.1,none))

omit [DecidableEq O] [DecidableEq I] [DecidableEq S] in
theorem restart_invalidates_without_reset (maximum : Nat) (fresh : O)
    (journal : Option PhysicalClock.JournalGate) (clock : PhysicalClock.Clock O)
    (current : Cut D I S) (binding : Option (Binding O D I S))
    (admitted : PhysicalClock.lifecycleGuard journal clock = .ok ()) :
    beginRun maximum fresh journal clock current binding =
      ((⟨fresh,clock.episode.bind (PhysicalClock.checkedNext maximum),0,.ready⟩,(current,none)), .ok ()) :=
  PhysicalClock.admitted_run_resets_projection _ _ _ _ _ _ admitted

omit [DecidableEq O] [DecidableEq I] [DecidableEq S] in
theorem refused_restart_preserves (maximum : Nat) (fresh : O)
    (journal : Option PhysicalClock.JournalGate) (clock : PhysicalClock.Clock O)
    (current : Cut D I S) (binding : Option (Binding O D I S)) (reason : PhysicalClock.LifecycleRefusal)
    (refused : PhysicalClock.lifecycleGuard journal clock = .error reason) :
    beginRun maximum fresh journal clock current binding = ((clock,(current,binding)),.error reason) :=
  PhysicalClock.refused_run_preserves_all _ _ _ _ _ _ _ refused

theorem query_independent (cycle : Cycle V E Q R) (v : V) (inside : cycle.inside v) (q r : Q) :
    cycle.read v q = cycle.read v r := (cycle.observes v q inside).trans (cycle.observes v r inside).symm

theorem changed_query_refutes_autonomy (cycle : Cycle V E Q R) (v : V)
    (inside : cycle.inside v) (q r : Q) (different : cycle.read v q ≠ cycle.read v r) : False :=
  different (query_independent cycle v inside q r)

omit [DecidableEq O] [DecidableEq I] [DecidableEq S] in
/-- An aligned binding predicts the actual retained observer read for every
query, using its anchor's phase rather than assuming physical age starts at zero. -/
theorem aligned_read (cycle : Cycle V E Q R) (value : S → V)
    (b : Binding O D I S) (valid : Aligned cycle value b) (query : Q) :
    cycle.read (value b.current.state) query =
      cycle.view ((cycle.phase (value b.anchorCut.state) + b.ticks) % cycle.period) := by
  rw [cycle.observes _ query valid.currentInside, valid.phase]

end SourceCycle
