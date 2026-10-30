import Lean

/-! Exact structural counts for finite immutable declarations. Operands and child
names are independent positional choices; semantic equivalence never merges them.
Only completed counts enter the nominal memo. No bigint latency, allocation or
native VM refinement follows from these arithmetic laws. -/
namespace ExactCardinality
set_option warningAsError true
variable {Owner : Type}

structure Alternative (Owner : Type) where
  arity : Nat
  children : List Owner
  deriving DecidableEq
structure Declaration (Owner : Type) where
  initialPool : Nat
  limit : Nat
  alternatives : List (Alternative Owner)
  deriving DecidableEq
structure Term where
  arity : Nat
  choices : Nat
  deriving DecidableEq

def factor (counts : Owner → Nat) (children : List Owner) : Nat := (children.map counts).prod
def terms (counts : Owner → Nat) (alternatives : List (Alternative Owner)) : List Term :=
  alternatives.map fun a => ⟨a.arity, factor counts a.children⟩
def instructions (pool : Nat) (ts : List Term) : Nat :=
  (ts.map fun t => pool ^ t.arity * t.choices).sum
def bodies (u : Nat) (ts : List Term) : Nat → Nat
  | 0 => 1
  | n + 1 => bodies u ts n * instructions (u + n) ts
def total (u : Nat) (ts : List Term) : Nat → Nat
  | 0 => 0
  | n + 1 => total u ts n + (n + 1) * bodies u ts (n + 1)
def count (counts : Owner → Nat) (d : Declaration Owner) : Nat :=
  total d.initialPool (terms counts d.alternatives) d.limit

theorem primitive_factor (counts : Owner → Nat) : factor counts [] = 1 := rfl

theorem repeated_child (counts : Owner → Nat) (owner : Owner) :
    factor counts [owner, owner] = counts owner * counts owner := by simp [factor]

theorem factor_congr (left right : Owner → Nat) (children : List Owner)
    (same : ∀ child ∈ children, left child = right child) :
    factor left children = factor right children := by
  exact congrArg List.prod (List.map_congr_left same)

theorem body_product (u : Nat) (ts : List Term) (n : Nat) :
    bodies u ts n = ((List.range n).map fun i => instructions (u + i) ts).prod := by
  induction n with
  | zero => rfl
  | succ n ih => simp [bodies, List.range_succ, ih]

theorem count_formula (u : Nat) (ts : List Term) (k : Nat) :
    total u ts k = ((List.range k).map fun i =>
      (i + 1) * ((List.range (i + 1)).map fun j => instructions (u + j) ts).prod).sum := by
  induction k with
  | zero => rfl
  | succ k ih => simp [total, List.range_succ, ih, body_product]

theorem bounded_count_congr (u : Nat) (left right : List Term) (k : Nat)
    (same : ∀ i < k, instructions (u + i) left = instructions (u + i) right) :
    bodies u left k = bodies u right k ∧ total u left k = total u right k := by
  induction k with
  | zero => exact ⟨rfl, rfl⟩
  | succ k ih =>
    have prior := ih (fun i hi => same i (Nat.lt_trans hi (Nat.lt_succ_self k)))
    have next : bodies u left (k + 1) = bodies u right (k + 1) := by
      simp only [bodies, prior.1, same k (Nat.lt_succ_self k)]
    exact ⟨next, by simp only [total, prior.2, next]⟩

theorem count_congr (left right : Owner → Nat) (d : Declaration Owner)
    (same : ∀ a ∈ d.alternatives, ∀ child ∈ a.children, left child = right child) :
    count left d = count right d := by
  have equalTerms : terms left d.alternatives = terms right d.alternatives := by
    apply List.map_congr_left
    intro a ha
    rw [factor_congr left right a.children (same a ha)]
  simp only [count, equalTerms]

/-- The largest represented operand pool must be zero, not just the initial one. -/
def unreachable (u k arity : Nat) : Bool :=
  decide (k = 0 ∨ (0 < arity ∧ u + (k - 1) = 0))
def prune (u k : Nat) (ts : List Term) : List Term :=
  ts.map fun t => if unreachable u k t.arity then {t with choices := 0} else t

theorem unreachable_coefficient (u k arity i : Nat) (position : i < k)
    (dead : unreachable u k arity = true) : (u + i) ^ arity = 0 := by
  simp only [unreachable, decide_eq_true_eq] at dead
  rcases dead with empty | ⟨positive, zero⟩
  · omega
  · have poolZero : u + i = 0 := by omega
    rw [poolZero]
    exact Nat.zero_pow positive

theorem nullary_survives (u k : Nat) (nonempty : 0 < k) :
    unreachable u k 0 = false ∧ (0 : Nat) ^ 0 = 1 := by
  constructor
  · simp [unreachable, Nat.ne_of_gt nonempty]
  · rfl

theorem pruned_instructions (u k i : Nat) (ts : List Term) (position : i < k) :
    instructions (u + i) (prune u k ts) = instructions (u + i) ts := by
  unfold instructions prune
  rw [List.map_map]
  congr 1
  apply List.map_congr_left
  intro t _
  by_cases dead : unreachable u k t.arity = true
  · simp only [dead, ↓reduceIte, Function.comp_apply]
    rw [unreachable_coefficient u k t.arity i position dead]
    simp
  · simp [dead]

theorem pruning_preserves_count (u k : Nat) (ts : List Term) :
    total u (prune u k ts) k = total u ts k :=
  (bounded_count_congr u _ _ k (fun i hi => pruned_instructions u k i ts hi)).2

/-- Rank orders dependencies; it is never used as the memo identity. -/
def Acyclic (declarations : Owner → Declaration Owner) (rank : Owner → Nat) : Prop :=
  ∀ owner a, a ∈ (declarations owner).alternatives →
    ∀ child ∈ a.children, rank child < rank owner
def Exact (declarations : Owner → Declaration Owner) (counts : Owner → Nat) : Prop :=
  ∀ owner, counts owner = count counts (declarations owner)

/-- A finite unfolding constructs the unique solution without assuming a cache
    or a pre-existing oracle for child counts. -/
def expansion (declarations : Owner → Declaration Owner) : Nat → Owner → Nat
  | 0, _ => 0
  | fuel + 1, owner => count (expansion declarations fuel) (declarations owner)

def exactCounts (declarations : Owner → Declaration Owner) (rank : Owner → Nat)
    (owner : Owner) : Nat := expansion declarations (rank owner + 1) owner

theorem expansion_stable (declarations : Owner → Declaration Owner) (rank : Owner → Nat)
    (acyclic : Acyclic declarations rank) (fuel extra : Nat) (owner : Owner)
    (enough : rank owner < fuel) :
    expansion declarations (fuel + extra) owner = expansion declarations fuel owner := by
  induction fuel generalizing owner with
  | zero => omega
  | succ fuel ih =>
    simp only [Nat.succ_add, expansion]
    apply count_congr
    intro a ha child hc
    exact ih child (Nat.lt_of_lt_of_le (acyclic owner a ha child hc) (Nat.le_of_lt_succ enough))

theorem exact_counts_exist (declarations : Owner → Declaration Owner) (rank : Owner → Nat)
    (acyclic : Acyclic declarations rank) : Exact declarations (exactCounts declarations rank) := by
  intro owner
  change expansion declarations (rank owner + 1) owner = count _ (declarations owner)
  rw [expansion]
  apply count_congr
  intro a ha child hc
  have bound : rank child + 1 ≤ rank owner := Nat.succ_le_of_lt (acyclic owner a ha child hc)
  have stable := expansion_stable declarations rank acyclic (rank child + 1)
    (rank owner - (rank child + 1)) child (Nat.lt_succ_self _)
  simpa only [Nat.add_sub_of_le bound, exactCounts] using stable

theorem exact_counts_unique (declarations : Owner → Declaration Owner) (rank : Owner → Nat)
    (acyclic : Acyclic declarations rank) (left right : Owner → Nat)
    (hl : Exact declarations left) (hr : Exact declarations right) :
    ∀ owner, left owner = right owner := by
  intro owner
  have byRank : ∀ n owner, rank owner = n → left owner = right owner := by
    intro n
    induction n using Nat.strongRecOn with
    | ind n ih =>
      intro owner height
      rw [hl owner, hr owner]
      apply count_congr
      intro a ha child hc
      exact ih (rank child) (height ▸ acyclic owner a ha child hc) child rfl
  exact byRank (rank owner) owner rfl

abbrev Cache (Owner : Type) := List (Owner × Nat)
def lookup [DecidableEq Owner] (cache : Cache Owner) (owner : Owner) : Option Nat :=
  match cache with
  | [] => none
  | (key, value) :: rest => if key = owner then some value else lookup rest owner
def Sound (exact : Owner → Nat) (cache : Cache Owner) : Prop :=
  ∀ entry ∈ cache, entry.2 = exact entry.1

theorem lookup_sound [DecidableEq Owner] (exact : Owner → Nat) (cache : Cache Owner)
    (sound : Sound exact cache) (hit : lookup cache owner = some value) : value = exact owner := by
  induction cache with
  | nil => cases hit
  | cons entry rest ih =>
    rcases entry with ⟨key, cached⟩
    by_cases same : key = owner
    · simp only [lookup, same, ↓reduceIte, Option.some.injEq] at hit
      exact hit ▸ same ▸ sound (key, cached) (List.mem_cons_self)
    · simp only [lookup, same, ↓reduceIte] at hit
      exact ih (fun e he => sound e (List.mem_cons_of_mem _ he)) hit

theorem empty_count (counts : Owner → Nat) (d : Declaration Owner) (empty : d.limit = 0) :
    count counts d = 0 := by simp only [count, empty, total]

/-- Empty owners finish immediately and never need a memo entry. -/
def read [DecidableEq Owner] (declarations : Owner → Declaration Owner)
    (cache : Cache Owner) (owner : Owner) : Option Nat :=
  if (declarations owner).limit = 0 then some 0 else lookup cache owner

theorem read_exact [DecidableEq Owner] (declarations : Owner → Declaration Owner)
    (exact : Owner → Nat) (fixed : Exact declarations exact) (cache : Cache Owner)
    (sound : Sound exact cache) (found : read declarations cache owner = some value) :
    value = exact owner := by
  by_cases empty : (declarations owner).limit = 0
  · simp only [read, empty, ↓reduceIte, Option.some.injEq] at found
    rw [← found, fixed owner, empty_count exact _ empty]
  · simp only [read, empty, ↓reduceIte] at found
    exact lookup_sound exact cache sound found

def resolve [DecidableEq Owner] (declarations : Owner → Declaration Owner) (cache : Cache Owner) : List Owner → Option Nat
  | [] => some 1
  | owner :: rest => do
    let value ← read declarations cache owner
    let tail ← resolve declarations cache rest
    return value * tail

theorem resolved_factor [DecidableEq Owner] (declarations : Owner → Declaration Owner)
    (exact : Owner → Nat) (fixed : Exact declarations exact) (cache : Cache Owner)
    (sound : Sound exact cache) (children : List Owner)
    (resolved : resolve declarations cache children = some value) : value = factor exact children := by
  induction children generalizing value with
  | nil => simpa [resolve, factor] using resolved.symm
  | cons child rest ih =>
    cases found : read declarations cache child with
    | none => simp [resolve, found] at resolved
    | some head =>
      cases tail : resolve declarations cache rest with
      | none => simp [resolve, found, tail] at resolved
      | some tailValue =>
        have result : head * tailValue = value := by simpa [resolve, found, tail] using resolved
        rw [← result, read_exact declarations exact fixed cache sound found, ih tail]
        rfl

/-- Missing demanded dependencies keep publication pending; unreachable
    alternatives need no child count because their coefficients vanish. -/
def weigh [DecidableEq Owner] (declarations : Owner → Declaration Owner) (cache : Cache Owner) (u k : Nat) :
    List (Alternative Owner) → Option (List Term)
  | [] => some []
  | a :: rest => do
    let value ← if unreachable u k a.arity then some 0 else resolve declarations cache a.children
    let tail ← weigh declarations cache u k rest
    return ⟨a.arity, value⟩ :: tail

theorem weighed_terms [DecidableEq Owner] (declarations : Owner → Declaration Owner)
    (exact : Owner → Nat) (fixed : Exact declarations exact) (cache : Cache Owner)
    (sound : Sound exact cache) (u k : Nat) (alternatives : List (Alternative Owner))
    (ready : weigh declarations cache u k alternatives = some result) :
    result = prune u k (terms exact alternatives) := by
  induction alternatives generalizing result with
  | nil => simpa [weigh, prune, terms] using ready.symm
  | cons a rest ih =>
    cases tail : weigh declarations cache u k rest with
    | none =>
      by_cases dead : unreachable u k a.arity = true
      · simp [weigh, dead, tail] at ready
      · cases found : resolve declarations cache a.children <;> simp [weigh, dead, found, tail] at ready
    | some tailValue =>
      by_cases dead : unreachable u k a.arity = true
      · have result : ⟨a.arity, 0⟩ :: tailValue = result := by simpa [weigh, dead, tail] using ready
        rw [← result, ih tail]
        simp [prune, terms, dead]
      · cases found : resolve declarations cache a.children with
        | none => simp [weigh, dead, found] at ready
        | some value =>
          have result : ⟨a.arity, value⟩ :: tailValue = result := by
            simpa [weigh, dead, found, tail] using ready
          rw [← result, ih tail, resolved_factor declarations exact fixed cache sound a.children found]
          simp [prune, terms, dead]

def compute [DecidableEq Owner] (declarations : Owner → Declaration Owner) (cache : Cache Owner) (d : Declaration Owner) : Option Nat := do
  let ts ← weigh declarations cache d.initialPool d.limit d.alternatives
  return total d.initialPool ts d.limit

theorem computation_exact [DecidableEq Owner] (declarations : Owner → Declaration Owner)
    (exact : Owner → Nat) (fixed : Exact declarations exact) (cache : Cache Owner)
    (sound : Sound exact cache) (d : Declaration Owner)
    (completed : compute declarations cache d = some value) : value = count exact d := by
  cases ready : weigh declarations cache d.initialPool d.limit d.alternatives with
  | none => simp [compute, ready] at completed
  | some ts =>
    have result : total d.initialPool ts d.limit = value := by simpa [compute, ready] using completed
    rw [← result, weighed_terms declarations exact fixed cache sound _ _ _ ready, pruning_preserves_count]
    rfl

/-- Only a completed computation is published. Refused or partial computations
    have no publication transition. -/
inductive Publication [DecidableEq Owner] (declarations : Owner → Declaration Owner) :
    Cache Owner → Cache Owner → Prop where
  | complete {cache owner value} (nonempty : 0 < (declarations owner).limit)
      (miss : lookup cache owner = none) (ready : compute declarations cache (declarations owner) = some value) :
      Publication declarations cache (cache ++ [(owner, value)])

theorem publication_sound [DecidableEq Owner] (declarations : Owner → Declaration Owner)
    (exact : Owner → Nat) (fixed : Exact declarations exact) (sound : Sound exact cache)
    (step : Publication declarations cache next) : Sound exact next := by
  cases step with
  | complete _ _ ready =>
    intro entry member
    rcases List.mem_append.mp member with old | new
    · exact sound entry old
    · simp only [List.mem_singleton] at new
      subst entry
      exact (computation_exact declarations exact fixed _ sound _ ready).trans (fixed _).symm

inductive History [DecidableEq Owner] (declarations : Owner → Declaration Owner) : Cache Owner → Prop where
  | empty : History declarations []
  | publish {cache next} (prior : History declarations cache)
      (step : Publication declarations cache next) : History declarations next

theorem history_sound [DecidableEq Owner] (declarations : Owner → Declaration Owner)
    (exact : Owner → Nat) (fixed : Exact declarations exact)
    (history : History declarations cache) : Sound exact cache := by
  induction history with
  | empty => intro entry member; cases member
  | publish _ step ih => exact publication_sound declarations exact fixed ih step

theorem completed_reuse_exact [DecidableEq Owner] (declarations : Owner → Declaration Owner)
    (exact : Owner → Nat) (fixed : Exact declarations exact)
    (history : History declarations cache) (hit : lookup cache owner = some value) :
    value = exact owner := lookup_sound exact cache (history_sound declarations exact fixed history) hit

theorem distinct_owner_misses [DecidableEq Owner] (left right : Owner) (value : Nat)
    (distinct : left ≠ right) : lookup [(left, value)] right = none := by simp [lookup, distinct]

/-- A cache obtained from completion transitions returns the unique finite DAG
    count, including uncached empty owners. No oracle assumption is needed. -/
theorem finite_dag_reuse_exact [DecidableEq Owner] (declarations : Owner → Declaration Owner)
    (rank : Owner → Nat) (acyclic : Acyclic declarations rank)
    (history : History declarations cache) (found : read declarations cache owner = some value) :
    value = exactCounts declarations rank owner := by
  have fixed := exact_counts_exist declarations rank acyclic
  exact read_exact declarations _ fixed cache (history_sound declarations _ fixed history) found

end ExactCardinality
