import VerumExternalReplay.ContextualStop
import VerumExternalReplay.ClassifiedMass

/-! Completed finite leaves of a partial causal STOP/BIT process form a
subprobability. Undefined selectors contribute zero; surviving branches are
never conditioned or renormalized. The finite-cut theorem needs neither total
selectors nor almost-sure termination. Interpreting `none` as native refusal or
nontermination requires a separate sound partial-evaluation relation. -/
namespace PartialStop
open ExactMass CausalCoding ContextualStop
set_option warningAsError true

/-- A causal binary subkernel; state may contain the complete revealed history. -/
structure Model (State : Type) where
  probability : Kind → State → Bool → Fraction
  next : State → Bool → State
  bounded : ∀ kind state,
    AtMost (add (probability kind state false) (probability kind state true)) one

variable {State : Type}

/-- Unnormalized expectation after exactly n further body bits. -/
def body (model : Model State) : Nat → State → (State → Fraction) → Fraction
  | 0, state, future => future state
  | n+1, state, future => add
      (mul (model.probability .bit state false) (body model n (model.next state false) future))
      (mul (model.probability .bit state true) (body model n (model.next state true) future))

theorem body_mono (model : Model State) (n : Nat) (state : State)
    (f g : State → Fraction) (ordered : ∀ s, AtMost (f s) (g s)) :
    AtMost (body model n state f) (body model n state g) := by
  induction n generalizing state with
  | zero => exact ordered state
  | succ n ih =>
    exact le_trans
      (add_mono_right _ (mul_mono_left _ (ih _)))
      (add_mono_left _ (mul_mono_left _ (ih _)))

theorem body_congr (model : Model State) (n : Nat) (state : State)
    (f g : State → Fraction) (same : ∀ s, Equal (f s) (g s)) :
    Equal (body model n state f) (body model n state g) :=
  le_antisymm (body_mono model n state f g (fun s => eq_le (same s)))
    (body_mono model n state g f (fun s => eq_le (eq_symm (same s))))

theorem body_bounded (model : Model State) (n : Nat) (state : State)
    (future : State → Fraction) (bounded : ∀ s, AtMost (future s) one) :
    AtMost (body model n state future) one := by
  induction n generalizing state with
  | zero => exact bounded state
  | succ n ih =>
    have left := le_trans (mul_mono_left (model.probability .bit state false) (ih (model.next state false)))
      (eq_le (mul_one _))
    have right := le_trans (mul_mono_left (model.probability .bit state true) (ih (model.next state true)))
      (eq_le (mul_one _))
    exact le_trans (le_trans (add_mono_right _ left) (add_mono_left _ right))
      (model.bounded .bit state)

/-- All completed rows with at most n whole blocks, including their final STOP.
An unfinished body or a prefix cylinder is not a completed leaf. -/
def completed (model : Model State) (width : Nat) : Nat → State → Fraction
  | 0, state => model.probability .stop state true
  | n+1, state => add (model.probability .stop state true)
      (mul (model.probability .stop state false) (body model width state (completed model width n)))

theorem completed_bounded (model : Model State) (width n : Nat) (state : State) :
    AtMost (completed model width n state) one := by
  induction n generalizing state with
  | zero =>
    exact le_trans (le_add _ _)
      (le_trans (eq_le (add_comm _ _)) (model.bounded .stop state))
  | succ n ih =>
    exact le_trans (add_mono_left _ (le_trans
      (mul_mono_left _ (body_bounded model width state _ ih)) (eq_le (mul_one _))))
      (le_trans (eq_le (add_comm _ _)) (model.bounded .stop state))

theorem completed_mono (model : Model State) (width n : Nat) (state : State) :
    AtMost (completed model width n state) (completed model width (n+1) state) := by
  induction n generalizing state with
  | zero => exact le_add _ _
  | succ n ih => exact add_mono_left _ (mul_mono_left _ (body_mono model width state _ _ ih))

def likelihood (model : Model State) (state : State) : List Bool → Fraction
  | [] => one
  | bit :: rest => mul (model.probability .bit state bit)
      (likelihood model (model.next state bit) rest)

def advance (model : Model State) (state : State) : List Bool → State
  | [] => state
  | bit :: rest => advance model (model.next state bit) rest

theorem body_as_sum (model : Model State) (n : Nat) (state : State) (future : State → Fraction) :
    Equal (body model n state future) (sum ((words n).map (fun bits =>
      mul (likelihood model state bits) (future (advance model state bits))))) := by
  induction n generalizing state with
  | zero => exact eq_symm (eq_trans (add_zero _)
      (eq_trans (mul_comm _ _) (mul_one _)))
  | succ n ih =>
    have branch (bit : Bool) :
        Equal (mul (model.probability .bit state bit) (body model n (model.next state bit) future))
          (sum ((words n).map (fun bits =>
            mul (mul (model.probability .bit state bit) (likelihood model (model.next state bit) bits))
              (future (advance model (model.next state bit) bits))))) := by
      apply eq_trans (mul_congr_left _ (ih _))
      apply eq_trans (eq_symm (sum_scale _ _ _))
      apply sum_congr
      intro bits _
      rw [mul_assoc_exact]
      exact eq_refl _
    simp only [body,words,List.map_append,List.map_map,Function.comp_def,likelihood,advance]
    exact eq_trans (add_congr_right _ (branch false))
      (eq_trans (add_congr_left _ (branch true)) (eq_symm (sum_append _ _)))

/-- Enumeration preserves block boundaries and includes STOP exactly once. -/
def rows (width : Nat) : Nat → List (List (List Bool))
  | 0 => [[]]
  | n+1 => [] :: (words width).flatMap (fun bits => (rows width n).map (bits :: ·))

def rowMass (model : Model State) (state : State) : List (List Bool) → Fraction
  | [] => model.probability .stop state true
  | bits :: rest => mul (model.probability .stop state false)
      (mul (likelihood model state bits) (rowMass model (advance model state bits) rest))

theorem completed_as_sum (model : Model State) (width n : Nat) (state : State) :
    Equal (completed model width n state) (sum ((rows width n).map (rowMass model state))) := by
  induction n generalizing state with
  | zero => exact eq_symm (add_zero _)
  | succ n ih =>
    simp only [completed,rows,List.map_cons,rowMass,sum]
    apply add_congr_left
    apply eq_trans (mul_congr_left _ (body_congr model width state _ _ ih))
    apply eq_trans (mul_congr_left _ (body_as_sum model width state _))
    apply eq_trans (eq_symm (sum_scale _ _ _))
    simp only [List.map_flatMap,List.map_map,Function.comp_def,rowMass]
    apply eq_trans _ (eq_symm (sum_flatMap _ _))
    apply sum_congr
    intro bits _
    apply eq_trans (mul_congr_left _ (eq_symm (sum_scale _ _ _)))
    exact eq_symm (sum_scale _ _ _)

theorem rows_nodup (width n : Nat) : (rows width n).Nodup := by
  induction n with
  | zero => simp [rows]
  | succ n ih =>
    apply List.nodup_cons.mpr
    constructor
    · simp only [List.mem_flatMap,List.mem_map]
      intro ⟨bits,_,rest,_,impossible⟩
      contradiction
    · apply List.pairwise_flatMap.mpr
      constructor
      · intro bits _
        simpa [List.Nodup,List.pairwise_map] using ih
      · apply (words_nodup width).imp
        intro a b distinct x hx y hy
        obtain ⟨xs,_,rfl⟩ := List.mem_map.mp hx
        obtain ⟨ys,_,rfl⟩ := List.mem_map.mp hy
        intro same
        exact distinct (List.cons.inj same).1

theorem rows_valid (width n : Nat) (row : List (List Bool)) (member : row ∈ rows width n) :
    ∀ bits ∈ row, bits.length = width := by
  induction n generalizing row with
  | zero =>
    have empty : row = [] := by simpa [rows] using member
    simp [empty]
  | succ n ih =>
    simp only [rows,List.mem_cons,List.mem_flatMap,List.mem_map] at member
    rcases member with rfl | ⟨bits,hbits,rest,hrest,rfl⟩
    · simp
    · intro block hblock
      rcases List.mem_cons.mp hblock with rfl | hblock
      · exact word_length hbits
      · exact ih rest hrest block hblock

theorem rows_complete (width n : Nat) (row : List (List Bool))
    (valid : ∀ bits ∈ row, bits.length = width) (depth : row.length ≤ n) :
    row ∈ rows width n := by
  induction n generalizing row with
  | zero =>
    have empty : row = [] := List.length_eq_zero_iff.mp (Nat.eq_zero_of_le_zero depth)
    simp [empty,rows]
  | succ n ih =>
    cases row with
    | nil => simp [rows]
    | cons bits rest =>
      apply List.mem_cons_of_mem
      apply List.mem_flatMap.mpr
      refine ⟨bits, ?_, List.mem_map.mpr ⟨rest, ?_, rfl⟩⟩
      · simpa only [valid bits (by simp)] using word_complete bits
      · exact ih rest (fun block member => valid block (List.mem_cons_of_mem _ member))
          (Nat.le_of_succ_le_succ depth)

/-- Positive fixed-width blocks make complete wire names injective. -/
theorem flatten_injective (width : Nat) (positive : 0 < width)
    (a b : List (List Bool)) (ha : ∀ bits ∈ a, bits.length = width)
    (hb : ∀ bits ∈ b, bits.length = width) (same : a.flatten = b.flatten) : a = b := by
  induction a generalizing b with
  | nil =>
    cases b with
    | nil => rfl
    | cons bits rest =>
      have empty : bits = [] := (List.append_eq_nil_iff.mp same.symm).1
      have width_zero : width = 0 := (hb bits (by simp)).symm.trans (by simp [empty])
      exact False.elim ((Nat.ne_of_gt positive) width_zero)
  | cons bits rest ih =>
    cases b with
    | nil =>
      have empty : bits = [] := (List.append_eq_nil_iff.mp same).1
      have width_zero : width = 0 := (ha bits (by simp)).symm.trans (by simp [empty])
      exact False.elim ((Nat.ne_of_gt positive) width_zero)
    | cons other tail =>
      have lengths := (ha bits (by simp)).trans (hb other (by simp)).symm
      have first := List.append_inj_left same lengths
      have remaining := List.append_inj_right same lengths
      cases first
      exact congrArg (bits :: ·) (ih tail
        (fun block member => ha block (List.mem_cons_of_mem _ member))
        (fun block member => hb block (List.mem_cons_of_mem _ member)) remaining)

/-- A complete finite catalogue can directly supply the classified-mass laws.
`rows_valid` and `flatten_injective` justify flattened names at positive width;
native byte parsing and callback correspondence remain separate obligations. -/
def weights (model : Model State) (width n : Nat) (state : State) :
    ClassifiedMass.FiniteWeights (List (List Bool)) where
  names := rows width n
  distinct := rows_nodup width n
  weight := rowMass model state
  bounded := le_trans (eq_le (eq_symm (completed_as_sum model width n state)))
    (completed_bounded model width n state)

abbrev Selector (Key : Type) := Kind → List Bool → Option (Selection Key)

def probability (select : Selector Key) (bank : Bank Key) (kind : Kind)
    (past : List Bool) (raw : Bool) : Fraction :=
  match select kind past with
  | none => zero
  | some cell => ContextualStop.probability (fun _ _ => cell) bank kind past raw

theorem undefined_zero (select : Selector Key) (bank : Bank Key) (kind : Kind)
    (past : List Bool) (raw : Bool) (undefined : select kind past = none) :
    probability select bank kind past raw = zero := by simp [probability,undefined]

theorem defined_normalized (select : Selector Key) (bank : Bank Key) (kind : Kind)
    (past : List Bool) (cell : Selection Key) (defined : select kind past = some cell) :
    Equal (add (probability select bank kind past false) (probability select bank kind past true)) one := by
  simpa only [probability,defined] using
    ContextualStop.probabilities_normalized (fun _ _ => cell) bank kind past

theorem probabilities_bounded (select : Selector Key) (bank : Bank Key)
    (kind : Kind) (past : List Bool) :
    AtMost (add (probability select bank kind past false) (probability select bank kind past true)) one := by
  cases selected : select kind past with
  | none => simp [probability,selected,AtMost,add,zero,one]
  | some cell => exact eq_le (defined_normalized select bank kind past cell selected)

/-- Each factor reads the same bank. The selector receives only the kind and
revealed raw prefix, before the next outcome is chosen. -/
def contextual (select : Selector Key) (bank : Bank Key) : Model (List Bool) where
  probability := probability select bank
  next past bit := bit :: past
  bounded := probabilities_bounded select bank

end PartialStop
