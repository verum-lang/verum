import Lean

/-! Finite empirical expressibility, conditional on the supplied representation
and labelled cohort. Indices name declared fibres/classes, not datasets. This
proves no population accuracy, label truth, learning rate, or native extraction.
Any decoder state/history used to distinguish rows belongs in the representation. -/
namespace FiniteDecoder
set_option warningAsError true

def total : {n : Nat} → (Fin n → Nat) → Nat
  | 0, _ => 0
  | n + 1, f => f 0 + total (fun i : Fin n => f i.succ)

theorem total_zero (n : Nat) : total (fun _ : Fin n => 0) = 0 := by
  induction n with
  | zero => rfl
  | succ n ih => simp only [total, ih, Nat.zero_add]

theorem total_add (f g : Fin n → Nat) :
    total (fun i => f i + g i) = total f + total g := by
  induction n with
  | zero => rfl
  | succ n ih => simp only [total, ih]; omega

theorem total_mono (f g : Fin n → Nat) (bound : ∀ i, f i ≤ g i) : total f ≤ total g := by
  induction n with
  | zero => exact Nat.le_refl _
  | succ n ih =>
    exact Nat.add_le_add (bound 0) (ih _ _ (fun i => bound i.succ))

theorem total_indicator (selected : Fin n) (value : Nat) :
    total (fun i : Fin n => if i = selected then value else 0) = value := by
  induction n with
  | zero => exact Fin.elim0 selected
  | succ n ih =>
    refine Fin.cases ?_ (fun i => ?_) selected
    · simp [total, Fin.succ_ne_zero, total_zero]
    · simpa [total, Ne.symm (Fin.succ_ne_zero i)] using ih i

-- Fin (classes + 1) makes the declared label domain nonempty without choice.
def best : (classes : Nat) → (Fin (classes + 1) → Nat) → Fin (classes + 1)
  | 0, _ => 0
  | classes + 1, f =>
    let other := (best classes (fun i => f i.succ)).succ
    if f other ≤ f 0 then 0 else other

theorem best_bounds (f : Fin (classes + 1) → Nat) (i : Fin (classes + 1)) :
    f i ≤ f (best classes f) := by
  induction classes with
  | zero =>
    have h : i = 0 := by
      apply Fin.ext
      have := i.isLt
      omega
    subst i
    exact Nat.le_refl _
  | succ classes ih =>
    simp only [best]
    split
    · rename_i h
      refine Fin.cases (Nat.le_refl _) (fun j => ?_) i
      exact Nat.le_trans (ih (fun x => f x.succ) j) h
    · rename_i h
      refine Fin.cases ?_ (fun j => ih (fun x => f x.succ) j) i
      omega

-- Occurrences are counted, including repeated observations and contradictions.
def count [DecidableEq F] [DecidableEq L] : List (F × L) → F → L → Nat
  | [], _, _ => 0
  | row :: rest, key, label =>
    (if row.1 = key ∧ row.2 = label then 1 else 0) + count rest key label

def correct [DecidableEq L] (decode : F → L) : List (F × L) → Nat
  | [] => 0
  | row :: rest => (if decode row.1 = row.2 then 1 else 0) + correct decode rest

theorem correct_bounded [DecidableEq L] (decode : F → L) (rows : List (F × L)) :
    correct decode rows ≤ rows.length := by
  induction rows with
  | nil => exact Nat.le_refl _
  | cons row rest ih => simp only [correct, List.length_cons]; split <;> omega

theorem correct_perfect [DecidableEq L] (decode : F → L) (rows : List (F × L)) :
    correct decode rows = rows.length ↔ ∀ row ∈ rows, decode row.1 = row.2 := by
  induction rows with
  | nil =>
    constructor
    · intro _ row member; cases member
    · intro _; rfl
  | cons row rest ih =>
    by_cases head : decode row.1 = row.2
    · simp only [correct, ite_eq_left head, List.length_cons]
      constructor
      · intro full item member
        cases List.mem_cons.mp member with
        | inl same => subst item; exact head
        | inr tail => exact ih.mp (by omega) item tail
      · intro agree
        have tail := ih.mpr (fun item member => agree item (List.mem_cons_of_mem row member))
        omega
    · constructor
      · intro full
        have bound := correct_bounded decode rest
        simp only [correct, ite_eq_right head, Nat.zero_add, List.length_cons] at full
        omega
      · intro agree
        exact False.elim (head (agree row List.mem_cons_self))

theorem correct_by_fibre (decode : Fin n → Fin (classes + 1))
    (rows : List (Fin n × Fin (classes + 1))) :
    correct decode rows = total (fun key => count rows key (decode key)) := by
  induction rows with
  | nil => exact (total_zero n).symm
  | cons row rest ih =>
    have oneRow :
        (fun key : Fin n => if row.1 = key ∧ row.2 = decode key then 1 else 0) =
        (fun key : Fin n => if key = row.1 then (if decode row.1 = row.2 then 1 else 0) else 0) := by
      funext key
      by_cases same : key = row.1
      · subst key; simp only [true_and, ite_true, eq_self]; simp only [eq_comm]
      · have other : row.1 ≠ key := Ne.symm same
        simp only [other, false_and, same, ite_false]
    simp only [correct, count, total_add, oneRow, total_indicator, ih]

/-- A finite cohort encoded into nominal fibres and a nonempty label domain.
Faithful equality-preserving encoding of external keys and labels is an adapter
obligation. Unobserved indices contribute zero, not invented observations. -/
abbrev Cohort (fibres classes : Nat) := List (Fin fibres × Fin (classes + 1))

def majority (rows : Cohort fibres classes) (key : Fin fibres) : Fin (classes + 1) :=
  best classes (count rows key)

/-- Sum of per-fibre maximum label multiplicities; `best_bounds` proves each
selected count is maximal and the selected class is an explicit witness. -/
def ceiling (rows : Cohort fibres classes) : Nat :=
  total (fun key => count rows key (majority rows key))

theorem decoder_bound (rows : Cohort fibres classes) (decode : Fin fibres → Fin (classes + 1)) :
    correct decode rows ≤ ceiling rows := by
  rw [correct_by_fibre]
  exact total_mono _ _ (fun key => best_bounds (count rows key) (decode key))

theorem majority_attains (rows : Cohort fibres classes) : correct (majority rows) rows = ceiling rows :=
  correct_by_fibre _ _

theorem ceiling_bounded (rows : Cohort fibres classes) : ceiling rows ≤ rows.length := by
  rw [← majority_attains]
  exact correct_bounded _ _

theorem perfect_separation (rows : Cohort fibres classes) :
    ceiling rows = rows.length ↔
      ∃ decode : Fin fibres → Fin (classes + 1), ∀ row ∈ rows, decode row.1 = row.2 := by
  constructor
  · intro full
    exact ⟨majority rows, (correct_perfect _ _).mp ((majority_attains rows).trans full)⟩
  · intro ⟨decode, agree⟩
    have bound := decoder_bound rows decode
    rw [(correct_perfect decode rows).mpr agree] at bound
    exact Nat.le_antisymm (ceiling_bounded rows) bound

theorem conflicting_fibre_prevents_perfection (rows : Cohort fibres classes)
    (key : Fin fibres) (left right : Fin (classes + 1))
    (first : (key,left) ∈ rows) (second : (key,right) ∈ rows) (different : left ≠ right) :
    ceiling rows < rows.length := by
  have bound := ceiling_bounded rows
  have unequal : ceiling rows ≠ rows.length := by
    intro full
    obtain ⟨decode, agree⟩ := (perfect_separation rows).mp full
    exact different ((agree _ first).symm.trans (agree _ second))
  omega

-- A factor map witnesses refinement; no heuristic accuracy comparison is used.
def project (coarsen : F → G) (rows : List (F × L)) : List (G × L) :=
  rows.map (fun row => (coarsen row.1, row.2))

theorem project_correct [DecidableEq L] (coarsen : F → G) (decode : G → L) (rows : List (F × L)) :
    correct decode (project coarsen rows) = correct (fun x => decode (coarsen x)) rows := by
  induction rows with
  | nil => rfl
  | cons row rest ih => simp only [project, List.map_cons, correct] at *; rw [ih]

theorem refinement_monotone (coarsen : Fin fine → Fin coarse) (rows : Cohort fine classes) :
    ceiling (project coarsen rows) ≤ ceiling rows := by
  rw [← majority_attains, project_correct]
  exact decoder_bound rows _

theorem factorized_decoder_bound (representation : Input → Fin fibres)
    (decode : Fin fibres → Fin (classes + 1)) (rows : List (Input × Fin (classes + 1))) :
    correct (fun input => decode (representation input)) rows ≤ ceiling (project representation rows) := by
  rw [← project_correct]
  exact decoder_bound _ _

theorem unavoidable_errors (rows : Cohort fibres classes) (decode : Fin fibres → Fin (classes + 1)) :
    rows.length - ceiling rows ≤ rows.length - correct decode rows :=
  Nat.sub_le_sub_left (decoder_bound rows decode) _

end FiniteDecoder
