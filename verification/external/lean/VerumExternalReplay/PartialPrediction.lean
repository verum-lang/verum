import VerumExternalReplay.StoppedProcess

/-! Subprobability accounting for partial probabilistic selectors. Every known
choice is locally normalized. Semantic refusal and pending work retain their
own leaves and their incoming mass; neither is a successful zero-mass result.
The bounded tree is proof data, not a proposed native enumerator or VM. -/
namespace PartialPrediction
open ExactMass
set_option warningAsError true

structure Choice where
  weight : Bool → Fraction
  normalized : Equal (add (weight false) (weight true)) one

inductive Step (State Error : Type) where
  | complete
  | pending
  | refused : Error → Step State Error
  | choose : Choice → (Bool → State) → Step State Error

inductive Failure (Error : Type) where
  | refused : Error → Failure Error
  | pending | incomplete | pastTerminal
  deriving DecidableEq

/-- A successful prefix need not have a defined or terminating continuation. -/
def walk (step : State → Step State Error) (state : State) :
    List Bool → Except (Failure Error) (Fraction × State)
  | [] => .ok (one,state)
  | bit :: rest => match step state with
    | .complete => .error .pastTerminal
    | .pending => .error .pending
    | .refused error => .error (.refused error)
    | .choose choice next => (walk step (next bit) rest).map
        (fun (mass,final) => (mul (choice.weight bit) mass,final))

/-- Only a finished, fully defined trace yields a point mass. -/
def point (step : State → Step State Error) (state : State) (bits : List Bool) :
    Except (Failure Error) Fraction := do
  let (mass,final) ← walk step state bits
  match step final with
  | .complete => .ok mass
  | .pending => .error .pending
  | .refused error => .error (.refused error)
  | .choose .. => .error .incomplete

theorem walk_append (step : State → Step State Error) (state : State) (past future : List Bool) :
    walk step state (past ++ future) = (do
      let (p,next) ← walk step state past
      (walk step next future).map (fun (q,final) => (mul p q,final))) := by
  induction past generalizing state with
  | nil =>
    cases h : walk step state future <;>
      simp [walk,h,Except.map,Bind.bind,Except.bind,mul,one]
  | cons bit rest ih =>
    cases h : step state with
    | complete => simp [walk,h,Bind.bind,Except.bind]
    | pending => simp [walk,h,Bind.bind,Except.bind]
    | refused error => simp [walk,h,Bind.bind,Except.bind]
    | choose choice next =>
      simp only [List.cons_append,walk,h,ih]
      cases hp : walk step (next bit) rest with
      | error error => rfl
      | ok result =>
        rcases result with ⟨p,final⟩
        simp only [Except.map,Bind.bind,Except.bind]
        cases hf : walk step final future <;>
          simp [mul_assoc_exact]

/-- Defined path factors compose; an undefined suffix keeps its typed failure. -/
theorem point_extension (step : State → Step State Error) (state : State) (past future : List Bool)
    (weight : Fraction) (final : State) (defined : walk step state past = .ok (weight,final)) :
    point step state (past ++ future) = (point step final future).map (mul weight) := by
  simp only [point,walk_append,defined,Except.map,Bind.bind,Except.bind]
  cases hf : walk step final future with
  | error error => rfl
  | ok result =>
    rcases result with ⟨p,last⟩
    dsimp only
    cases step last <;> rfl

theorem point_cons (step : State → Step State Error) (state : State)
    (choice : Choice) (next : Bool → State) (known : step state = .choose choice next)
    (bit : Bool) (rest : List Bool) :
    point step state (bit::rest) = (point step (next bit) rest).map (mul (choice.weight bit)) := by
  simp only [point,walk,known]
  cases hp : walk step (next bit) rest with
  | error error => rfl
  | ok result =>
    rcases result with ⟨mass,final⟩
    simp only [Except.map,Bind.bind,Except.bind]
    cases step final <;> rfl

theorem refused_point (step : State → Step State Error) (state : State)
    (error : Error) (refused : step state = .refused error) :
    point step state [] = .error (.refused error) := by simp [point,walk,Bind.bind,Except.bind,refused]

theorem pending_point (step : State → Step State Error) (state : State)
    (pending : step state = .pending) : point step state [] = .error .pending := by
  simp [point,walk,Bind.bind,Except.bind,pending]

inductive Outcome (State Error : Type) where
  | complete
  | pending : State → Outcome State Error
  | refused : Error → Outcome State Error
  deriving DecidableEq

structure Entry (State Error : Type) where
  path : List Bool
  outcome : Outcome State Error
  weight : Fraction
  deriving DecidableEq

/-- Preserve the path, outcome and exact incoming factor of each descendant. -/
def branch (bit : Bool) (weight : Fraction) (entries : List (Entry State Error)) : List (Entry State Error) :=
  entries.map (fun entry => ⟨bit::entry.path,entry.outcome,mul weight entry.weight⟩)

/-- Exhausting the proof horizon creates pending mass, never completion. -/
def frontier (step : State → Step State Error) : Nat → State → List (Entry State Error)
  | 0, state => [⟨[],.pending state,one⟩]
  | n+1, state => match step state with
    | .complete => [⟨[],.complete,one⟩]
    | .pending => [⟨[],.pending state,one⟩]
    | .refused error => [⟨[],.refused error,one⟩]
    | .choose choice next =>
        branch false (choice.weight false) (frontier step n (next false)) ++
        branch true (choice.weight true) (frontier step n (next true))

def mass (entries : List (Entry State Error)) : Fraction := sum (entries.map Entry.weight)

def completed (entries : List (Entry State Error)) : Fraction :=
  sum (entries.filterMap (fun entry => match entry.outcome with
    | .complete => some entry.weight | _ => none))

theorem branch_mass (bit : Bool) (weight : Fraction) (entries : List (Entry State Error)) :
    Equal (mass (branch bit weight entries)) (mul weight (mass entries)) := by
  simp only [mass,branch,List.map_map,Function.comp_def]
  exact sum_scale _ _ _

theorem mass_append (first rest : List (Entry State Error)) :
    Equal (mass (first ++ rest)) (add (mass first) (mass rest)) := by
  induction first with
  | nil => exact eq_symm (eq_trans (add_comm _ _) (add_zero _))
  | cons entry first ih => exact eq_trans (add_congr_left _ ih) (eq_symm (add_assoc _ _ _))

theorem frontier_conserved (step : State → Step State Error) (n : Nat) (state : State) :
    Equal (mass (frontier step n state)) one := by
  induction n generalizing state with
  | zero => exact add_zero _
  | succ n ih =>
    simp only [frontier]
    cases step state with
    | complete => exact add_zero _
    | pending => exact add_zero _
    | refused error => exact add_zero _
    | choose choice next =>
      apply eq_trans (mass_append _ _)
      apply eq_trans (add_congr_right _ (eq_trans (branch_mass _ _ _)
        (eq_trans (mul_congr_left _ (ih _)) (mul_one _))))
      apply eq_trans (add_congr_left _ (eq_trans (branch_mass _ _ _)
        (eq_trans (mul_congr_left _ (ih _)) (mul_one _))))
      exact choice.normalized

theorem completed_le_mass (entries : List (Entry State Error)) : AtMost (completed entries) (mass entries) := by
  induction entries with
  | nil => exact le_refl _
  | cons entry rest ih =>
    cases h : entry.outcome with
    | complete => simpa only [completed,List.filterMap_cons,h,mass,List.map_cons,sum] using add_mono_left entry.weight ih
    | pending state =>
      simpa only [completed,List.filterMap_cons,h,mass,List.map_cons,sum] using
        le_trans ih (le_trans (le_add (mass rest) entry.weight) (eq_le (add_comm _ _)))
    | refused error =>
      simpa only [completed,List.filterMap_cons,h,mass,List.map_cons,sum] using
        le_trans ih (le_trans (le_add (mass rest) entry.weight) (eq_le (add_comm _ _)))

/-- The mass of all completed traces in any finite cut is at most one.
Refusals, unresolved work and arbitrarily long executions need no totality premise. -/
theorem subprobability (step : State → Step State Error) (n : Nat) (state : State) :
    AtMost (completed (frontier step n state)) one :=
  le_trans (completed_le_mass _) (eq_le (frontier_conserved step n state))

/-- Every counted complete leaf is an actual successful path product. -/
theorem completed_trace (step : State → Step State Error) (n : Nat) (state : State)
    (entry : Entry State Error) (member : entry ∈ frontier step n state)
    (done : entry.outcome = .complete) : point step state entry.path = .ok entry.weight := by
  induction n generalizing state entry with
  | zero => simp [frontier] at member; subst entry; contradiction
  | succ n ih =>
    cases h : step state with
    | complete => simp [frontier,h] at member; subst entry; simp [point,walk,Bind.bind,Except.bind,h]
    | pending => simp [frontier,h] at member; subst entry; contradiction
    | refused error => simp [frontier,h] at member; subst entry; contradiction
    | choose choice next =>
      simp only [frontier,h,List.mem_append] at member
      rcases member with left | right
      all_goals
        simp only [branch,List.mem_map] at *
        obtain ⟨child,member,rfl⟩ := ‹∃ _, _›
        rw [point_cons step state choice next h]
        simp only [ih _ child member done,Except.map]

/-- Every successful finite trace occurs in a sufficiently deep cut. No
successful path is excluded to obtain the subprobability bound. -/
theorem trace_in_frontier (step : State → Step State Error) (state : State)
    (bits : List Bool) (weight : Fraction) (success : point step state bits = .ok weight)
    (horizon : Nat) (enough : bits.length < horizon) :
    (⟨bits,.complete,weight⟩ : Entry State Error) ∈ frontier step horizon state := by
  induction bits generalizing state weight horizon with
  | nil =>
    cases horizon with
    | zero => simp at enough
    | succ n =>
      cases h : step state <;> simp [point,walk,Bind.bind,Except.bind,h] at success
      subst weight
      simp [frontier,h]
  | cons bit rest ih =>
    cases horizon with
    | zero => simp at enough
    | succ n =>
      have remaining : rest.length < n := by simp only [List.length_cons] at enough; omega
      cases h : step state with
      | complete => simp [point,walk,Bind.bind,Except.bind,h] at success
      | pending => simp [point,walk,Bind.bind,Except.bind,h] at success
      | refused error => simp [point,walk,Bind.bind,Except.bind,h] at success
      | choose choice next =>
        rw [point_cons step state choice next h] at success
        cases hp : point step (next bit) rest with
        | error error => simp [hp,Except.map] at success
        | ok value =>
          simp only [hp,Except.map,Except.ok.injEq] at success
          subst weight
          have found := ih (next bit) value hp n remaining
          simp only [frontier,h]
          cases bit with
          | false => exact List.mem_append.mpr (Or.inl
              (List.mem_map.mpr ⟨⟨rest,.complete,value⟩,found,rfl⟩))
          | true => exact List.mem_append.mpr (Or.inr
              (List.mem_map.mpr ⟨⟨rest,.complete,value⟩,found,rfl⟩))

/-- A defined prefix buys an upper bound on completed continuations, not their
exact mass. Failed and pending descendants remain in the frontier separately. -/
theorem prefix_upper_bound (step : State → Step State Error) (state : State)
    (bits : List Bool) (weight : Fraction) (final : State)
    (defined : walk step state bits = .ok (weight,final)) (horizon : Nat) :
    (walk step state bits).map (fun (p,s) => mul p (completed (frontier step horizon s))) =
      .ok (mul weight (completed (frontier step horizon final))) ∧
    AtMost (mul weight (completed (frontier step horizon final))) weight := by
  constructor
  · rw [defined]; rfl
  · exact le_trans (mul_mono_left _ (subprobability step horizon final)) (eq_le (mul_one _))

inductive Phase (State : Type) where
  | boundary : State → Phase State
  /-- The current bit plus this many additional bits remain in the block. -/
  | bits : State → Nat → Phase State
  | done
  deriving DecidableEq

inductive Selection (Error : Type) where
  | ready : Choice → Selection Error
  | pending
  | refused : Error → Selection Error

def Selection.step (selection : Selection Error) (next : Bool → State) : Step State Error :=
  match selection with
  | .ready choice => .choose choice next
  | .pending => .pending
  | .refused error => .refused error

structure Model (State Error : Type) where
  unitBits : Nat
  positiveWidth : 0 < unitBits
  stop : State → Selection Error
  bit : State → Selection Error
  next : State → Bool → State

/-- Partial STOP/BIT selection uses the same block protocol. No probability is
requested at a refused or unresolved selection. -/
def execute (model : Model State Error) : Phase State → Step (Phase State) Error
  | .done => .complete
  | .boundary state => (model.stop state).step
      (fun stop => if stop then .done else .bits state (model.unitBits-1))
  | .bits state remaining => (model.bit state).step
      (fun bit => if remaining = 0 then .boundary (model.next state bit)
        else .bits (model.next state bit) (remaining-1))

/-- The existing total stopped model embeds without changing its local laws.
This does not assert that an arbitrary partial selector has such a certificate. -/
def ofStopped (model : StoppedProcess.Model State) : Model State Empty where
  unitBits := model.unitBits
  positiveWidth := model.positiveWidth
  stop state := .ready ⟨model.stop state,model.normalized state⟩
  bit state := .ready ⟨model.body.probability state,model.body.normalized state⟩
  next := model.body.next

def stopped (model : StoppedProcess.Model State) := execute (ofStopped model)

end PartialPrediction
