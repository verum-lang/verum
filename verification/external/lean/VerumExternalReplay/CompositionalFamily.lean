import VerumExternalReplay.EnumerableMass
/-! Exact cardinality of finite compositional source names. Each instruction
selects an alternative, operands and every child-program name. The closed
count avoids enumerating the family; the proofs relate it to a duplicate-free
finite index space. Child signatures come from earlier frozen declarations.
These laws do not establish native enumeration order, a word codec, VM
semantics, scheduler completeness, allocation costs or empirical accuracy. -/
namespace CompositionalFamily
set_option warningAsError true

def selections : List Nat → List (List Nat)
  | [] => [[]]
  | n::ns => (List.range n).flatMap (fun i => (selections ns).map (i::·))

theorem selections_length (ns : List Nat) : (selections ns).length = ns.prod := by
  induction ns with
  | nil => rfl
  | cons n ns ih => simp [selections,ih,List.map_const',List.sum_replicate_nat]

theorem range_nodup (n : Nat) : (List.range n).Nodup := by
  induction n with
  | zero => simp
  | succ n ih =>
    rw [List.range_succ]
    apply List.pairwise_append.mpr
    refine ⟨ih,by simp,?_⟩
    intro a ha b hb eq
    have below := List.mem_range.mp ha
    have same : b = n := by simpa using hb
    omega

theorem selections_nodup (ns : List Nat) : (selections ns).Nodup := by
  induction ns with
  | nil => simp [selections]
  | cons n ns ih =>
    apply List.pairwise_flatMap.mpr
    constructor
    · intro i _
      exact ih.map (i::·) (fun _ _ h eq => h (List.cons.inj eq).2)
    · apply (range_nodup n).imp
      intro a b h x hx y hy eq
      obtain ⟨xs,_,rfl⟩ := List.mem_map.mp hx
      obtain ⟨ys,_,rfl⟩ := List.mem_map.mp hy
      exact h (List.cons.inj eq).1

def tagged {A : Type} : List (List A) → List (Nat × A)
  | [] => []
  | xs::rest => xs.map (0,·) ++ (tagged rest).map (fun p => (p.1+1,p.2))

theorem tagged_length {A : Type} (xs : List (List A)) :
    (tagged xs).length = (xs.map List.length).sum := by
  induction xs with
  | nil => rfl
  | cons x xs ih => simp [tagged,ih]

theorem tagged_nodup {A : Type} (xs : List (List A)) (unique : ∀ x ∈ xs, x.Nodup) :
    (tagged xs).Nodup := by
  induction xs with
  | nil => simp [tagged]
  | cons x xs ih =>
    apply List.pairwise_append.mpr
    refine ⟨?_,?_,?_⟩
    · exact (unique x (by simp)).map (0,·) (fun _ _ h eq => h (Prod.mk.inj eq).2)
    · apply (ih (fun y hy => unique y (by simp [hy]))).map
      intro a b h eq
      apply h
      exact Prod.ext (Nat.add_right_cancel (Prod.mk.inj eq).1) (Prod.mk.inj eq).2
    · intro a ha b hb eq
      obtain ⟨a,_,rfl⟩ := List.mem_map.mp ha
      obtain ⟨b,_,rfl⟩ := List.mem_map.mp hb
      have h := (Prod.mk.inj eq).1
      omega

structure Alternative where
  arity : Nat
  children : List Nat
  deriving DecidableEq

def dimensions (a : Alternative) (pool : Nat) := List.replicate a.arity pool ++ a.children

def instructionCount (forms : List Alternative) (pool : Nat) : Nat :=
  (forms.map (fun a => pool^a.arity * a.children.prod)).sum

def instructions (forms : List Alternative) (pool : Nat) :=
  tagged (forms.map (fun a => selections (dimensions a pool)))

theorem instructions_length (forms : List Alternative) (pool : Nat) :
    (instructions forms pool).length = instructionCount forms pool := by
  simp [instructions,tagged_length,selections_length,dimensions,instructionCount,List.prod_replicate_nat,List.map_map,Function.comp_def]

theorem instructions_nodup (forms : List Alternative) (pool : Nat) :
    (instructions forms pool).Nodup := by
  apply tagged_nodup
  intro x hx
  obtain ⟨a,_,rfl⟩ := List.mem_map.mp hx
  exact selections_nodup _

def widths (forms : List Alternative) (inputs literals length : Nat) : List Nat :=
  length :: (List.range length).map (fun position => instructionCount forms (inputs+literals+position))

def programCount (forms : List Alternative) (inputs literals length : Nat) : Nat :=
  length * ((List.range length).map (fun position => instructionCount forms (inputs+literals+position))).prod

def count (forms : List Alternative) (inputs literals steps : Nat) : Nat :=
  ((List.range steps).map (fun n => programCount forms inputs literals (n+1))).sum

def names (forms : List Alternative) (inputs literals steps : Nat) :=
  tagged ((List.range steps).map (fun n => selections (widths forms inputs literals (n+1))))

theorem names_length (forms : List Alternative) (inputs literals steps : Nat) :
    (names forms inputs literals steps).length = count forms inputs literals steps := by
  simp [names,tagged_length,selections_length,widths,programCount,count,List.map_map,Function.comp_def]

theorem names_nodup (forms : List Alternative) (inputs literals steps : Nat) :
    (names forms inputs literals steps).Nodup := by
  apply tagged_nodup
  intro x hx
  obtain ⟨a,_,rfl⟩ := List.mem_map.mp hx
  exact selections_nodup _

structure Signature where
  owner : Nat
  inputs : Nat
  names : Nat
  deriving DecidableEq

inductive Control where
  | branch : Nat → Nat → Control
  | fold : Nat → Control
  deriving DecidableEq

structure Declaration where
  owner : Nat
  inputs : Nat
  literals : Nat
  steps : Nat
  arities : List Nat
  controls : List Control
  deriving DecidableEq

/-- Preserve source identity until the native declaration conditions have been
checked. Equal arities do not make primitive identities interchangeable; equal
literal values cannot occupy different source-pool positions. Captured calls
remain distinct alternatives even when their bodies happen to be equal. -/
def checkedDeclaration {P V : Type} [DecidableEq P] [DecidableEq V]
    (owner inputs steps : Nat) (operations : List (P × Nat)) (literals : List V)
    (calls : List Nat) (controls : List Control) : Option Declaration :=
  if decide (operations.map Prod.fst).Nodup && decide literals.Nodup &&
      decide (0 < operations.length + calls.length + controls.length) then
    some ⟨owner,inputs,literals.length,steps,operations.map Prod.snd ++ calls,controls⟩
  else none

theorem checked_declaration_spec {P V : Type} [DecidableEq P] [DecidableEq V]
    (owner inputs steps : Nat) (operations : List (P × Nat)) (literals : List V)
    (calls : List Nat) (controls : List Control) (declaration : Declaration) :
    checkedDeclaration owner inputs steps operations literals calls controls = some declaration ↔
    (operations.map Prod.fst).Nodup ∧ literals.Nodup ∧
      0 < operations.length + calls.length + controls.length ∧
      declaration = ⟨owner,inputs,literals.length,steps,operations.map Prod.snd ++ calls,controls⟩ := by
  unfold checkedDeclaration
  constructor
  · intro success
    split at success
    · rename_i valid
      obtain ⟨both,nonempty⟩ := Bool.and_eq_true_iff.mp valid
      obtain ⟨ops,lits⟩ := Bool.and_eq_true_iff.mp both
      exact ⟨of_decide_eq_true ops,of_decide_eq_true lits,of_decide_eq_true nonempty,
        (Option.some.inj success).symm⟩
    · cases success
  · rintro ⟨ops,lits,nonempty,rfl⟩
    rw [decide_eq_true ops,decide_eq_true lits,decide_eq_true nonempty]
    rfl

/-- A branch adds a condition to the common capture row. The exclusive capacity
is explicit so the shape law applies to any native index width. -/
def branchArity (capacity yes no : Nat) : Option Nat :=
  if decide (yes = no) && decide (yes + 1 < capacity) then some (yes + 1) else none

theorem branch_arity_spec (capacity yes no arity : Nat) :
    branchArity capacity yes no = some arity ↔
      yes = no ∧ arity = yes + 1 ∧ arity < capacity := by
  unfold branchArity
  constructor
  · intro success
    split at success
    · rename_i valid
      obtain ⟨same,bounded⟩ := Bool.and_eq_true_iff.mp valid
      have arity_eq := (Option.some.inj success).symm
      exact ⟨of_decide_eq_true same,arity_eq,arity_eq ▸ of_decide_eq_true bounded⟩
    · cases success
  · rintro ⟨same,rfl,bounded⟩
    rw [decide_eq_true same,decide_eq_true bounded]
    rfl

/-- The frozen source profile uses 64-bit usize. All supplied signature widths
are decoded within that range; the extra branch condition must also fit. -/
def control (prior : List Signature) : Control → Option Alternative
  | .branch yes no => do
    let yes ← prior.find? (fun p => p.owner = yes)
    let no ← prior.find? (fun p => p.owner = no)
    let arity ← branchArity (2^64) yes.inputs no.inputs
    pure ⟨arity,[yes.names,no.names]⟩
  | .fold body => do
    let body ← prior.find? (fun p => p.owner = body)
    if 2 ≤ body.inputs then some ⟨body.inputs,[body.names]⟩ else none

def alternatives (prior : List Signature) (d : Declaration) : Option (List Alternative) := do
  let controls ← d.controls.mapM (control prior)
  pure (d.arities.map (fun n => ⟨n,[]⟩) ++ controls)

/-- Dependencies must precede their parent. Captured calls contribute their
fixed arity to arities; automatic branch/fold bodies contribute their entire
family size. Repeated owners, missing bodies and invalid widths are refused. -/
def extend (prior : List Signature) (d : Declaration) : Option (List Signature) := do
  if prior.any (fun p => p.owner = d.owner) then none else
  let forms ← alternatives prior d
  pure (prior ++ [⟨d.owner,d.inputs,count forms d.inputs d.literals d.steps⟩])

def census : List Signature → List Declaration → Option (List Signature)
  | prior,[] => some prior
  | prior,d::ds => (extend prior d).bind (fun next => census next ds)

theorem extension_retains_prior (prior : List Signature) (d : Declaration)
    (after : List Signature) (success : extend prior d = some after) :
    ∃ current, after = prior ++ [current] := by
  unfold extend at success
  split at success
  · cases success
  · cases h : alternatives prior d with
    | none => simp [h] at success
    | some forms =>
      simp [h] at success
      exact ⟨_,success.symm⟩


theorem uniform_normalized {A : Type} (xs : List A) (nonempty : 0 < xs.length) :
    ExactMass.Equal (ExactMass.sum (xs.map (fun _ => ExactMass.unit xs.length nonempty)))
      ExactMass.one := by
  have h := ExactMass.sum_common_denominator (List.replicate xs.length 1) xs.length nonempty
  have same : xs.map (fun _ => ExactMass.unit xs.length nonempty) =
      (List.replicate xs.length 1).map (fun n => (⟨n,xs.length,nonempty⟩ : ExactMass.Fraction)) := by
    simp [List.map_const',ExactMass.unit]
  rw [same]
  exact ExactMass.eq_trans h (by simp [ExactMass.Equal,ExactMass.one,List.sum_replicate_nat])

end CompositionalFamily
