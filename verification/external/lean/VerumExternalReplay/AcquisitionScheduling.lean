import VerumExternalReplay.EnumerableMass

/- Finite scheduling contract, not a certificate for the Rust scheduler.
   Addresses use depth = offset + 1. Declaration steps, paid openings and service
   turns are different events. A service turn requires a usable grant after
   capture/routing, and adequate atomic primitive work or a real continuation.
   Applying the bounds requires retained owners and compatible intake profiles;
   persistent-frontier progress permits revisions between declarations.
   Tail arrivals and unfinished work must fit available storage; this model
   does not promise infinite retention in fixed RAM or eventual acceptance. -/
namespace AcquisitionScheduling
set_option warningAsError true

structure Address where
  offset : Nat
  evidence : Nat
  deriving DecidableEq, Repr

def triangle : Nat → Nat
  | 0 => 0
  | n + 1 => triangle n + n + 1

def index (a : Address) : Nat := triangle (a.offset + a.evidence) + a.offset

def next (a : Address) : Address :=
  match a.evidence with
  | 0 => ⟨0, a.offset + 1⟩
  | e + 1 => ⟨a.offset + 1, e⟩

def addressAt : Nat → Address
  | 0 => ⟨0, 0⟩
  | n + 1 => next (addressAt n)

theorem triangle_twice (n : Nat) : 2 * triangle n = n * (n + 1) := by
  induction n with
  | zero => rfl
  | succ n ih =>
    simp only [triangle, Nat.mul_add, Nat.mul_one, ih, Nat.add_mul, Nat.one_mul]
    omega

theorem triangle_formula (n : Nat) : triangle n = n * (n + 1) / 2 := by
  rw [← triangle_twice]
  omega

theorem triangle_monotone {a b : Nat} (h : a ≤ b) : triangle a ≤ triangle b := by
  induction b with
  | zero =>
    have : a = 0 := by omega
    simp [this]
  | succ b ih =>
    by_cases equal : a = b + 1
    · simp [equal]
    · have lower : a ≤ b := by omega
      have := ih lower
      simp only [triangle]
      omega

theorem index_interval (a : Address) :
    triangle (a.offset + a.evidence) ≤ index a ∧
    index a < triangle (a.offset + a.evidence + 1) := by
  constructor
  · exact Nat.le_add_right _ _
  · simp only [index, triangle]; omega

theorem index_injective (a b : Address) (h : index a = index b) : a = b := by
  have ha := index_interval a
  have hb := index_interval b
  have sums : a.offset + a.evidence = b.offset + b.evidence := by
    by_cases ab : a.offset + a.evidence < b.offset + b.evidence
    · have mono := triangle_monotone (Nat.succ_le_of_lt ab)
      change triangle (a.offset + a.evidence + 1) ≤ triangle (b.offset + b.evidence) at mono
      omega
    · by_cases ba : b.offset + b.evidence < a.offset + a.evidence
      · have mono := triangle_monotone (Nat.succ_le_of_lt ba)
        change triangle (b.offset + b.evidence + 1) ≤ triangle (a.offset + a.evidence) at mono
        omega
      · omega
  have offsets : a.offset = b.offset := by simp only [index, sums] at h; omega
  have evidence : a.evidence = b.evidence := by omega
  cases a; cases b; simp_all

theorem next_index (a : Address) : index (next a) = index a + 1 := by
  rcases a with ⟨d, e⟩
  cases e with
  | zero => simp [next, index, triangle]
  | succ e => simp [next, index, Nat.add_assoc, Nat.add_left_comm, Nat.add_comm]

theorem addressAt_index (n : Nat) : index (addressAt n) = n := by
  induction n with
  | zero => rfl
  | succ n ih => simp only [addressAt, next_index, ih]

theorem reaches_exact_address (a : Address) : addressAt (index a) = a :=
  index_injective _ _ (addressAt_index _)

theorem positive_depth_index (depth evidence : Nat) (positive : 0 < depth) :
    index ⟨depth - 1, evidence⟩ =
      ((depth - 1 + evidence) * (depth - 1 + evidence + 1)) / 2 + depth - 1 := by
  simp only [index, triangle_formula]
  omega

theorem every_finite_pair (depth evidence : Nat) (positive : 0 < depth) :
    ∃ n, (addressAt n).offset + 1 = depth ∧ (addressAt n).evidence = evidence := by
  refine ⟨index ⟨depth - 1, evidence⟩, ?_⟩
  rw [reaches_exact_address]
  exact ⟨by change depth - 1 + 1 = depth; omega, rfl⟩

def declarations : EnumerableMass.Enumeration Address where
  decode n := some (addressAt n)
  index := index
  exact a := congrArg some (reaches_exact_address a)

theorem finite_work_reachability (cost : Nat → Nat) (a : Address) :
    ∃ threshold, ∀ budget, threshold ≤ budget →
      EnumerableMass.reached declarations cost budget a = true ∧
      declarations.decode (declarations.index a) = some a :=
  EnumerableMass.every_code_reachable declarations cost a

-- A is a stable owned-attempt identity, not merely a search address. This is
-- the queue's identity projection: continuation payloads and progress require
-- separate runtime evidence. An unfinished attempt retains its identity.
structure Turn (A : Type) where
  unfinished : Bool
  arrivals : List A
  deriving DecidableEq

def service (queue : List A) (turn : Turn A) : List A :=
  match queue with
  | [] => turn.arrivals
  | head :: tail => tail ++ (if turn.unfinished then [head] else []) ++ turn.arrivals

def selected (queue : List A) (plan : Nat → Turn A) : Nat → List (Option A)
  | 0 => []
  | n + 1 => queue.head? :: selected (service queue (plan 0)) (fun i => plan (i + 1)) n

theorem tail_arrivals_keep_predecessors (head target : A) (ahead suffix : List A) (turn : Turn A) :
    service (head :: (ahead ++ target :: suffix)) turn =
      ahead ++ target :: (suffix ++ (if turn.unfinished then [head] else []) ++ turn.arrivals) := by
  simp [service, List.append_assoc]

theorem fifo_service_bound (target : A) (ahead suffix : List A) (plan : Nat → Turn A) :
    some target ∈ selected (ahead ++ target :: suffix) plan (ahead.length + 1) := by
  induction ahead generalizing suffix plan with
  | nil => simp [selected]
  | cons head ahead ih =>
    simp only [List.cons_append, List.length_cons]
    change some target ∈ some head :: selected
      (service (head :: (ahead ++ target :: suffix)) (plan 0))
      (fun i => plan (i + 1)) (ahead.length + 1)
    apply List.mem_cons_of_mem
    rw [tail_arrivals_keep_predecessors]
    exact ih _ _

theorem unfinished_owner_retained (head : A) (tail : List A) (arrivals : List A) :
    head ∈ service (head :: tail) ⟨true, arrivals⟩ := by simp [service]

theorem arrivals_cannot_overtake (head : A) (tail arrivals : List A) :
    service (head :: tail) ⟨false, arrivals⟩ = tail ++ arrivals := by simp [service]

-- A source round freezes nominal owners, not their positions in a movable
-- backing store. This is the unserved suffix of that roster. New owners enter
-- a subsequent round; a missing owner refuses selection without advancing it.
def roundSelect [DecidableEq A] (roster live : List A) : Option (A × List A) :=
  match roster with
  | [] => none
  | owner :: rest => if owner ∈ live then some (owner, rest) else none

def roundSelections [DecidableEq A] (roster : List A) (live : Nat → List A) :
    Nat → Option (List A)
  | 0 => some []
  | n + 1 => do
    let (owner, rest) ← roundSelect roster (live 0)
    let later ← roundSelections rest (fun i => live (i + 1)) n
    pure (owner :: later)

theorem round_retained_owner [DecidableEq A] (owner : A) (rest live : List A)
    (retained : owner ∈ live) :
    roundSelect (owner :: rest) live = some (owner, rest) := by
  simp [roundSelect, retained]

theorem round_missing_owner_refused [DecidableEq A] (owner : A) (rest live : List A)
    (missing : owner ∉ live) : roundSelect (owner :: rest) live = none := by
  simp [roundSelect, missing]

theorem round_storage_independent [DecidableEq A] (roster left right : List A)
    (sameOwners : ∀ owner, owner ∈ left ↔ owner ∈ right) :
    roundSelect roster left = roundSelect roster right := by
  cases roster with
  | nil => rfl
  | cons owner rest => simp only [roundSelect, sameOwners owner]

theorem fixed_round_prefix [DecidableEq A] (roster : List A) (live : Nat → List A)
    (n : Nat) (within : n ≤ roster.length)
    (retained : ∀ i < n, ∀ owner ∈ roster, owner ∈ live i) :
    roundSelections roster live n = some (roster.take n) := by
  induction n generalizing roster live with
  | zero => rfl
  | succ n ih =>
    cases roster with
    | nil => simp at within
    | cons owner rest =>
      have here := retained 0 (by omega) owner (by simp)
      have later : ∀ i < n, ∀ a ∈ rest, a ∈ live (i + 1) := by
        intro i bound a member
        exact retained (i + 1) (by omega) a (List.mem_cons_of_mem owner member)
      simp only [roundSelections, round_retained_owner owner rest (live 0) here]
      simp [ih rest (fun i => live (i + 1)) (by simpa using within) later]

theorem fixed_round_exact [DecidableEq A] (roster : List A) (live : Nat → List A)
    (retained : ∀ i < roster.length, ∀ owner ∈ roster, owner ∈ live i) :
    roundSelections roster live roster.length = some roster := by
  simpa using fixed_round_prefix roster live roster.length (Nat.le_refl _) retained

-- Persistent syntax search, independent of the address representation. A
-- revision replaces the revisit cursor but preserves the uninterrupted
-- frontier and turn. The archived cursor's storage/payload is outside this
-- projection and must remain owned in the implementation.
structure SearchState (Owner Cursor : Type) where
  owner : Owner
  frontier : Cursor
  revisit : Option Cursor
  revisitTurn : Bool
  deriving DecidableEq, Repr

inductive SearchAction (Cursor : Type) where
  | declaration
  | revision (start : Cursor)
  | suspended
  deriving DecidableEq, Repr

-- Declaration means a newly retained address and a successful cursor
-- successor, even if subsequent opening runs out of resources. A poll, failed
-- successor, or resumed opening of an existing address is not a declaration.
def searchStep (advance : C → C) (s : SearchState O C) : SearchAction C → SearchState O C
  | .revision start => { s with revisit := some start }
  | .suspended => s
  | .declaration =>
    if s.revisitTurn then
      match s.revisit with
      | some cursor => { s with revisit := some (advance cursor), revisitTurn := false }
      | none => { s with frontier := advance s.frontier, revisitTurn := false }
    else { s with frontier := advance s.frontier, revisitTurn := true }

def searchTrace (advance : C → C) (s : SearchState O C) : List (SearchAction C) → SearchState O C
  | [] => s
  | event :: rest => searchTrace advance (searchStep advance s event) rest

def completedDeclarations : List (SearchAction C) → Nat
  | [] => 0
  | .declaration :: rest => completedDeclarations rest + 1
  | _ :: rest => completedDeclarations rest

def frontierPotential (rank : C → Nat) (s : SearchState O C) : Nat :=
  2 * rank s.frontier + if s.revisitTurn then 0 else 1

theorem revision_preserves_frontier_and_turn (advance : C → C) (s : SearchState O C) (start : C) :
    (searchStep advance s (.revision start)).frontier = s.frontier ∧
    (searchStep advance s (.revision start)).revisitTurn = s.revisitTurn := ⟨rfl, rfl⟩

theorem suspended_search_unchanged (advance : C → C) (s : SearchState O C) :
    searchStep advance s .suspended = s := rfl

theorem search_step_owner (advance : C → C) (s : SearchState O C) (action : SearchAction C) :
    (searchStep advance s action).owner = s.owner := by
  cases action with
  | revision _ => rfl
  | suspended => rfl
  | declaration => cases s with
    | mk owner frontier revisit turn =>
      cases turn <;> cases revisit <;> rfl

theorem search_trace_owner (advance : C → C) (s : SearchState O C) (events : List (SearchAction C)) :
    (searchTrace advance s events).owner = s.owner := by
  induction events generalizing s with
  | nil => rfl
  | cons event rest ih =>
    exact (ih (searchStep advance s event)).trans (search_step_owner advance s event)

theorem frontier_step_potential (advance : C → C) (rank : C → Nat)
    (successor : ∀ cursor, rank (advance cursor) = rank cursor + 1)
    (s : SearchState O C) (action : SearchAction C) :
    frontierPotential rank s + completedDeclarations [action] ≤
      frontierPotential rank (searchStep advance s action) := by
  cases action with
  | revision _ => exact Nat.le_refl _
  | suspended => exact Nat.le_refl _
  | declaration => cases s with
    | mk owner frontier revisit turn =>
      cases turn <;> cases revisit <;>
        simp [searchStep, frontierPotential, completedDeclarations, successor] <;> omega

theorem frontier_trace_potential (advance : C → C) (rank : C → Nat)
    (successor : ∀ cursor, rank (advance cursor) = rank cursor + 1)
    (s : SearchState O C) (events : List (SearchAction C)) :
    frontierPotential rank s + completedDeclarations events ≤
      frontierPotential rank (searchTrace advance s events) := by
  induction events generalizing s with
  | nil => exact Nat.le_refl _
  | cons event rest ih =>
    have head := frontier_step_potential advance rank successor s event
    have tail := ih (searchStep advance s event)
    change _ ≤ frontierPotential rank (searchTrace advance (searchStep advance s event) rest)
    cases event <;> simp only [completedDeclarations] at * <;> omega

theorem completed_declarations_advance_frontier (advance : C → C) (rank : C → Nat)
    (successor : ∀ cursor, rank (advance cursor) = rank cursor + 1)
    (s : SearchState O C) (events : List (SearchAction C)) :
    rank s.frontier + completedDeclarations events / 2 ≤
      rank (searchTrace advance s events).frontier := by
  have progress := frontier_trace_potential advance rank successor s events
  simp only [frontierPotential] at progress
  split at progress <;> split at progress <;> omega

theorem two_declarations_advance_frontier (advance : C → C) (rank : C → Nat)
    (successor : ∀ cursor, rank (advance cursor) = rank cursor + 1)
    (s : SearchState O C) (events : List (SearchAction C))
    (completed : completedDeclarations events = 2) :
    rank s.frontier < rank (searchTrace advance s events).frontier := by
  have progress := completed_declarations_advance_frontier advance rank successor s events
  rw [completed] at progress
  exact progress

inductive Event (S : Type) where
  | opening (source : S)
  | resource (source : S)
  | service
  deriving DecidableEq

structure Charge (S : Type) where
  source : S
  look : Nat
  deriving DecidableEq

def pay (look : Nat) (event : Event S) : Nat × Option (Charge S) :=
  match event with
  | .opening source => (look + 1, some ⟨source, look⟩)
  | .resource _ | .service => (look, none)

def openings : List (Event S) → Nat
  | [] => 0
  | .opening _ :: rest => openings rest + 1
  | _ :: rest => openings rest

def replay (look : Nat) : List (Event S) → Nat × List (Charge S)
  | [] => (look, [])
  | event :: rest =>
    let step := pay look event
    let tail := replay step.1 rest
    (tail.1, step.2.toList ++ tail.2)

def isOpening : Event S → Bool
  | .opening _ => true
  | _ => false

theorem only_opening_consumes (look : Nat) (event : Event S) :
    (pay look event).1 = look + 1 ↔ isOpening event = true := by
  cases event with
  | opening _ => exact ⟨fun _ => rfl, fun _ => rfl⟩
  | resource _ =>
    exact ⟨fun h => False.elim (Nat.ne_of_lt (Nat.lt_succ_self look) h),
      fun h => Bool.noConfusion h⟩
  | service =>
    exact ⟨fun h => False.elim (Nat.ne_of_lt (Nat.lt_succ_self look) h),
      fun h => Bool.noConfusion h⟩

theorem resource_unopened_uncharged (look : Nat) (source : S) :
    pay look (.resource source) = (look, none) := rfl

theorem look_independent_of_address (look : Nat) (a b : Address) :
    (pay look (.opening a)).2.map Charge.look = (pay look (.opening b)).2.map Charge.look := rfl

theorem replay_look_exact (look : Nat) (events : List (Event S)) :
    (replay look events).1 = look + openings events := by
  induction events generalizing look with
  | nil => rfl
  | cons event rest ih => cases event <;> simp [replay, pay, openings, ih, Nat.add_comm, Nat.add_left_comm]

theorem charges_exact (look : Nat) (events : List (Event S)) :
    (replay look events).2.length = openings events := by
  induction events generalizing look with
  | nil => rfl
  | cons event rest ih => cases event <;> simp [replay, pay, openings, ih]

theorem deferred_opening_current_price (look : Nat) (source : S) (intervening : List (Event S)) :
    pay (replay look (.resource source :: intervening)).1 (.opening source) =
      (look + openings intervening + 1, some ⟨source, look + openings intervening⟩) := by
  simp [replay, pay, replay_look_exact]

-- A resource result has no evidential verdict. A completed semantic assessment
-- is a different constructor; the scheduler cannot turn the former into it.
inductive Result where
  | resource
  | assessed (accepted : Bool)
  deriving DecidableEq

def negative : Result → Bool
  | .assessed false => true
  | _ => false

theorem resource_not_negative : negative .resource = false := rfl

theorem negative_requires_assessment (r : Result) : negative r = true ↔ r = .assessed false := by
  cases r with
  | resource => simp [negative]
  | assessed b => cases b <;> simp [negative]

end AcquisitionScheduling
