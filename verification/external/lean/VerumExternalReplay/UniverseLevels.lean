import Std

/- Independent natural-number semantics for structural universe levels.
   Variables are opaque identities with independent, unbounded assignments.
   Normal forms are finite maxima of a constant and variable-plus-offset terms.
   A finite witness must be checked against its exact source expression before
   its order result can imply anything about that expression. -/
namespace Verum.UniverseLevels

inductive Formula where
  | concrete (level : Nat)
  | var (id : Nat)
  | succ (level : Formula)
  | max (left right : Formula)
  deriving DecidableEq

def denote (valuation : Nat → Nat) : Formula → Nat
  | .concrete n => n
  | .var id => valuation id
  | .succ a => denote valuation a + 1
  | .max a b => max (denote valuation a) (denote valuation b)

structure NormalLevel where
  constant : Nat
  terms : List (Nat × Nat)
  deriving DecidableEq

def evalTerms (valuation : Nat → Nat) : List (Nat × Nat) → Nat → Nat
  | [], constant => constant
  | (id, offset) :: rest, constant =>
      max (valuation id + offset) (evalTerms valuation rest constant)

def evalNormal (valuation : Nat → Nat) (level : NormalLevel) : Nat :=
  evalTerms valuation level.terms level.constant

def insertTerm (term : Nat × Nat) : List (Nat × Nat) → List (Nat × Nat)
  | [] => [term]
  | head :: rest =>
      if term.1 = head.1 then (term.1, max term.2 head.2) :: rest
      else if term.1 < head.1 then term :: head :: rest
      else head :: insertTerm term rest

def mergeTerms : List (Nat × Nat) → List (Nat × Nat) → List (Nat × Nat)
  | [], right => right
  | term :: rest, right => insertTerm term (mergeTerms rest right)

def successor (level : NormalLevel) : NormalLevel :=
  ⟨level.constant + 1, level.terms.map (fun term => (term.1, term.2 + 1))⟩

def join (left right : NormalLevel) : NormalLevel :=
  ⟨max left.constant right.constant, mergeTerms left.terms right.terms⟩

def normalize : Formula → NormalLevel
  | .concrete n => ⟨n, []⟩
  | .var id => ⟨0, [(id, 0)]⟩
  | .succ a => successor (normalize a)
  | .max a b => join (normalize a) (normalize b)

theorem constant_le_evalTerms (v : Nat → Nat) (terms : List (Nat × Nat)) (c : Nat) :
    c ≤ evalTerms v terms c := by
  induction terms with
  | nil => exact Nat.le_refl c
  | cons term rest ih =>
    simp only [evalTerms]
    exact Nat.le_trans ih (Nat.le_max_right _ _)

theorem evalTerms_insert (v : Nat → Nat) (term : Nat × Nat)
    (terms : List (Nat × Nat)) (c : Nat) :
    evalTerms v (insertTerm term terms) c =
      max (v term.1 + term.2) (evalTerms v terms c) := by
  induction terms with
  | nil => rfl
  | cons head rest ih =>
    unfold insertTerm
    split
    · rename_i same
      simp only [evalTerms]
      rw [same]
      omega
    · split
      · rfl
      · simp only [evalTerms]
        rw [ih]
        omega

theorem evalTerms_merge (v : Nat → Nat)
    (left right : List (Nat × Nat)) (c : Nat) :
    evalTerms v (mergeTerms left right) c =
      max (evalTerms v left c) (evalTerms v right c) := by
  induction left with
  | nil =>
    simp only [mergeTerms, evalTerms]
    exact (Nat.max_eq_right (constant_le_evalTerms v right c)).symm
  | cons term rest ih =>
    simp only [mergeTerms, evalTerms_insert, evalTerms]
    rw [ih]
    omega

theorem evalTerms_max_constant (v : Nat → Nat)
    (terms : List (Nat × Nat)) (a b : Nat) :
    evalTerms v terms (max a b) = max (evalTerms v terms a) b := by
  induction terms with
  | nil => rfl
  | cons term rest ih =>
    simp only [evalTerms]
    rw [ih]
    omega

theorem successor_sound (v : Nat → Nat) (level : NormalLevel) :
    evalNormal v (successor level) = evalNormal v level + 1 := by
  obtain ⟨c, terms⟩ := level
  simp only [evalNormal, successor]
  induction terms with
  | nil => rfl
  | cons term rest ih =>
    simp only [List.map_cons, evalTerms]
    rw [ih]
    omega

theorem join_sound (v : Nat → Nat) (left right : NormalLevel) :
    evalNormal v (join left right) = max (evalNormal v left) (evalNormal v right) := by
  simp only [evalNormal, join, evalTerms_merge]
  rw [evalTerms_max_constant, Nat.max_comm left.constant right.constant,
    evalTerms_max_constant]
  have hl := constant_le_evalTerms v left.terms left.constant
  have hr := constant_le_evalTerms v right.terms right.constant
  omega

theorem normalize_sound (v : Nat → Nat) (source : Formula) :
    evalNormal v (normalize source) = denote v source := by
  induction source with
  | concrete n => rfl
  | var id => simp [normalize, evalNormal, evalTerms, denote]
  | succ source ih => simpa only [normalize, denote, successor_sound] using congrArg (· + 1) ih
  | max left right hl hr => simp only [normalize, denote, join_sound, hl, hr]

def minimum (level : NormalLevel) : Nat := evalNormal (fun _ => 0) level

theorem minimum_le_eval (v : Nat → Nat) (level : NormalLevel) :
    minimum level ≤ evalNormal v level := by
  obtain ⟨c, terms⟩ := level
  simp only [minimum, evalNormal]
  induction terms with
  | nil => exact Nat.le_refl c
  | cons term rest ih =>
    simp only [evalTerms]
    omega

theorem term_le_evalTerms (v : Nat → Nat) (term : Nat × Nat)
    (terms : List (Nat × Nat)) (c : Nat) (member : term ∈ terms) :
    v term.1 + term.2 ≤ evalTerms v terms c := by
  induction terms with
  | nil => simp at member
  | cons head rest ih =>
    rcases List.mem_cons.mp member with same | member
    · subst head
      exact Nat.le_max_left _ _
    · exact Nat.le_trans (ih member) (Nat.le_max_right _ _)

theorem evalTerms_le (v : Nat → Nat) (terms : List (Nat × Nat)) (c bound : Nat)
    (hc : c ≤ bound) (ht : ∀ term ∈ terms, v term.1 + term.2 ≤ bound) :
    evalTerms v terms c ≤ bound := by
  induction terms with
  | nil => exact hc
  | cons term rest ih =>
    apply Nat.max_le.mpr
    exact ⟨ht term (by simp), ih (fun other member => ht other (by simp [member]))⟩

/- This is the universal order criterion, not satisfiability under one model. -/
def Below (left right : NormalLevel) : Bool :=
  decide (left.constant ≤ minimum right) &&
    left.terms.all (fun term => right.terms.any (fun peer =>
      decide (term.1 = peer.1 ∧ term.2 ≤ peer.2)))

theorem below_sound {left right : NormalLevel} (accepted : Below left right = true) :
    ∀ v, evalNormal v left ≤ evalNormal v right := by
  simp only [Below, Bool.and_eq_true, List.all_eq_true, List.any_eq_true,
    decide_eq_true_eq] at accepted
  intro v
  apply evalTerms_le
  · exact Nat.le_trans accepted.1 (minimum_le_eval v right)
  · intro term member
    obtain ⟨peer, present, same, offset⟩ := accepted.2 term member
    have covered := term_le_evalTerms v peer right.terms right.constant present
    rw [same]
    exact Nat.le_trans (Nat.add_le_add_left offset _) covered

/- Equivalence binds a Rust-produced normal form to its exact source, including
   opaque IDs, successors, maxima and constants. It also tolerates harmless
   redundant constants without trusting the implementation's normalization. -/
def Normalizes (source : Formula) (witness : NormalLevel) : Bool :=
  Below (normalize source) witness && Below witness (normalize source)

theorem normalizes_sound {source : Formula} {witness : NormalLevel}
    (accepted : Normalizes source witness = true) :
    ∀ v, evalNormal v witness = denote v source := by
  simp only [Normalizes, Bool.and_eq_true] at accepted
  intro v
  have forward := below_sound accepted.1 v
  have backward := below_sound accepted.2 v
  rw [normalize_sound] at forward backward
  exact Nat.le_antisymm backward forward

theorem checked_le {source target : Formula} {left right : NormalLevel}
    (sourceMatches : Normalizes source left = true)
    (targetMatches : Normalizes target right = true)
    (accepted : Below left right = true) :
    ∀ v, denote v source ≤ denote v target := by
  intro v
  have result := below_sound accepted v
  simpa only [normalizes_sound sourceMatches v, normalizes_sound targetMatches v] using result

theorem checked_lt {source target : Formula} {left right : NormalLevel}
    (sourceMatches : Normalizes source left = true)
    (targetMatches : Normalizes target right = true)
    (accepted : Below (successor left) right = true) :
    ∀ v, denote v source < denote v target := by
  intro v
  have result := below_sound accepted v
  rw [successor_sound, normalizes_sound sourceMatches v, normalizes_sound targetMatches v] at result
  omega

end Verum.UniverseLevels
