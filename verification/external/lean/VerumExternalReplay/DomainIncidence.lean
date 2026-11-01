import VerumExternalReplay.DomainSupport
import Init.Data.List.Perm

/-! Immutable domain incidence, independently of scheduling and allocation.
The partition laws are general. The paid transition model describes the captured
plan/expansion lanes; its consumer must validate native index and integer bounds.
An emitted Publication is the model's required write, not evidence that Rust
performed it. Native exports must record that write to prove correspondence. -/
namespace DomainIncidence
set_option warningAsError true

abbrev Position := Nat × Nat
abbrev Entry := Nat × Position

structure Group where
  domain : Nat
  positions : List Position
  deriving DecidableEq, Inhabited

def entries (groups : List Group) : List Entry :=
  groups.flatMap fun g => g.positions.map fun p => (g.domain, p)

def insert (domain : Nat) (position : Position) : List Group → List Group
  | [] => [⟨domain, [position]⟩]
  | g :: rest => if g.domain = domain then { g with positions := g.positions ++ [position] } :: rest
                 else g :: insert domain position rest

/-- Grouping cannot lose or duplicate an original position, including its key. -/
theorem insert_partition (groups : List Group) (domain : Nat) (position : Position) :
    (entries (insert domain position groups)).Perm (entries groups ++ [(domain, position)]) := by
  induction groups with
  | nil => simp [insert, entries]
  | cons g rest ih =>
      by_cases same : g.domain = domain
      · simpa [insert, same, entries, List.append_assoc] using
          (List.perm_append_comm (l₁ := [(domain, position)]) (l₂ := entries rest)).append_left
            (g.positions.map fun p => (domain, p))
      · simpa [insert, same, entries, List.append_assoc] using
          ih.append_left (g.positions.map fun p => (g.domain, p))

def build : List Entry → List Group → List Group
  | [], groups => groups
  | (domain, position) :: rest, groups => build rest (insert domain position groups)

theorem build_partition (input : List Entry) (groups : List Group) :
    (entries (build input groups)).Perm (entries groups ++ input) := by
  induction input generalizing groups with
  | nil => simp [build]
  | cons entry rest ih =>
      exact (ih (insert entry.1 entry.2 groups)).trans (by
        simpa [List.append_assoc] using (insert_partition groups entry.1 entry.2).append_right rest)

/-- Multiplicity is preserved even if an input itself contains duplicates. -/
theorem exact_multiplicity (input : List Entry) (entry : Entry) :
    (entries (build input [])).count entry = input.count entry := by
  simpa [entries] using (build_partition input []).count_eq entry

theorem canonical_partition (groups : List Group) (input : List Entry)
    (canonical : groups = build input []) : (entries groups).Perm input := by
  rw [canonical]
  simpa [entries] using build_partition input []

def original (domains : List (List Nat)) : List Entry :=
  domains.zipIdx.flatMap fun (arguments, alternative) =>
    arguments.zipIdx.map fun (domain, anchor) => (domain, (alternative, anchor))

inductive PlanPhase where
  | position | lookup | publish | done
  deriving DecidableEq

structure Plan where
  alternative : Nat := 0
  anchor : Nat := 0
  requested : Nat := 0
  lookup : Nat := 0
  cells : Nat := 0
  phase : PlanPhase := .position
  groups : List Group := []
  deriving DecidableEq

inductive ExpansionPhase where
  | cases | positions
  deriving DecidableEq

structure Child where
  language : String
  identity : String
  arrival : Nat
  outputs : List Nat
  domain : Nat := 0
  cursor : Nat := 0
  compatible : Nat := 0
  position : Nat := 0
  phase : ExpansionPhase := .cases
  deriving DecidableEq

structure State where
  plan : Plan
  child : Option Child
  deriving DecidableEq

structure Publication where
  alternative : Nat
  newest : Nat
  anchor : Nat
  compatible : Nat
  deriving DecidableEq

inductive Outcome where
  | advanced | workRefusal | capacityRefusal
  deriving DecidableEq

structure Result where
  outcome : Outcome
  charged : Nat
  cells : Nat
  state : State
  publication : Option Publication := none
  deriving DecidableEq

def room (cells additional capacity : Nat) : Bool :=
  cells + additional ≤ capacity && cells + additional ≤ 18446744073709551615

def nextDomain (c : Child) : Child :=
  { c with domain := c.domain + 1, cursor := 0, compatible := 0, position := 0, phase := .cases }

def step (domains : List (List Nat)) (s : State) (cells grant capacity : Nat) : Result := Id.run do
  if grant == 0 then return ⟨.workRefusal, 0, cells, s, none⟩
  let refused : Result := ⟨.capacityRefusal, 1, cells, s, none⟩
  if !room cells 0 capacity then return refused
  let p := s.plan
  if p.phase != .done then
    match p.phase with
    | .position =>
        let next := if p.alternative == domains.length then { p with phase := .done }
          else if domains[p.alternative]!.isEmpty then { p with alternative := p.alternative + 1 }
          else { p with requested := domains[p.alternative]![p.anchor]!, lookup := 0, phase := .lookup }
        return ⟨.advanced, 1, cells, { s with plan := next }, none⟩
    | .lookup =>
        let next := match p.groups[p.lookup]? with
          | none => { p with phase := .publish }
          | some g => if g.domain == p.requested then { p with phase := .publish }
                      else { p with lookup := p.lookup + 1 }
        return ⟨.advanced, 1, cells, { s with plan := next }, none⟩
    | .publish =>
        let additional := if p.lookup == p.groups.length then 2 else 1
        if !room cells additional capacity then return refused
        let groups := if p.lookup == p.groups.length then p.groups ++ [⟨p.requested, [(p.alternative, p.anchor)]⟩]
          else p.groups.modify p.lookup fun g => { g with positions := g.positions ++ [(p.alternative, p.anchor)] }
        let last := p.anchor + 1 == domains[p.alternative]!.length
        let next := { p with
          groups := groups
          cells := p.cells + additional
          phase := .position
          alternative := if last then p.alternative + 1 else p.alternative
          anchor := if last then 0 else p.anchor + 1 }
        return ⟨.advanced, 1, cells + additional, { s with plan := next }, none⟩
    | .done => return ⟨.advanced, 1, cells, s, none⟩
  let some c := s.child | return ⟨.advanced, 1, cells, s, none⟩
  let some g := p.groups[c.domain]? |
    return ⟨.advanced, 1, cells - 1, { s with child := none }, none⟩
  match c.phase with
  | .cases =>
      let compatible := if c.outputs[c.cursor]! &&& g.domain == 0 then c.compatible
                        else c.compatible ||| (1 <<< c.cursor)
      let cursor := c.cursor + 1
      let next := { c with compatible := compatible, cursor := cursor }
      let next := if cursor == c.outputs.length then
          if compatible == 0 then nextDomain next else { next with phase := .positions }
        else next
      return ⟨.advanced, 1, cells, { s with child := some next }, none⟩
  | .positions =>
      let (alternative, anchor) := g.positions[c.position]!
      let empty := c.arrival == 0 && anchor > 0
      if !empty && !room cells 1 capacity then return refused
      let next := { c with position := c.position + 1 }
      let next := if next.position == g.positions.length then nextDomain next else next
      return ⟨.advanced, 1, cells + (if empty then 0 else 1), { s with child := some next },
        if empty then none else some ⟨alternative, c.arrival, anchor, c.compatible⟩⟩

def PlanSound (domains : List (List Nat)) (p : Plan) : Prop :=
  p.cells = p.groups.length + (entries p.groups).length ∧
  p.groups = build ((original domains).filter fun e =>
    e.2.1 < p.alternative || (e.2.1 == p.alternative && e.2.2 < p.anchor)) []

instance (domains : List (List Nat)) (p : Plan) : Decidable (PlanSound domains p) := by
  unfold PlanSound; infer_instance

def ChildSound (groups : List Group) (c : Child) : Prop :=
  match groups[c.domain]? with
  | none => c.cursor = 0 ∧ c.compatible = 0 ∧ c.position = 0 ∧ c.phase = .cases
  | some g => match c.phase with
      | .cases => c.compatible = DomainSupport.mask (c.outputs.take c.cursor) g.domain ∧ c.position = 0
      | .positions => c.compatible = DomainSupport.mask c.outputs g.domain ∧ c.compatible ≠ 0 ∧
          c.cursor = c.outputs.length

instance (groups : List Group) (c : Child) : Decidable (ChildSound groups c) := by
  unfold ChildSound; split <;> first | infer_instance | (split <;> infer_instance)

theorem zero_work (domains : List (List Nat)) (s : State) (cells capacity : Nat) :
    step domains s cells 0 capacity = ⟨.workRefusal, 0, cells, s, none⟩ := rfl

end DomainIncidence
