import VerumExternalReplay.ExactMass

/- Exact finite-alphabet Dirichlet(1/2) prediction. Counts may include frozen
   training observations. Evaluation uses the pre-observation count, then adds
   exactly one observation. Alphabet names/order, contexts and training protocol
   must be fixed by the caller; this model proves neither their native fidelity
   nor a statistical false-positive guarantee. -/
namespace CategoricalPrediction
open ExactMass
set_option warningAsError true

structure Histogram where
  counts : List Nat
  nonempty : 0 < counts.length
  deriving DecidableEq

def denominator (h : Histogram) : Nat := 2 * h.counts.sum + h.counts.length

theorem denominator_positive (h : Histogram) : 0 < denominator h := by
  have := h.nonempty
  unfold denominator
  omega

def predict (h : Histogram) (label : Fin h.counts.length) : Fraction :=
  ⟨2 * h.counts[label.val] + 1, denominator h, denominator_positive h⟩

theorem predict_positive (h : Histogram) (label : Fin h.counts.length) :
    0 < (predict h label).numerator := by simp [predict]

def observe (h : Histogram) (label : Fin h.counts.length) : Histogram :=
  ⟨h.counts.modify label.val (· + 1), by simpa using h.nonempty⟩

theorem alphabet_unchanged (h : Histogram) (label : Fin h.counts.length) :
    (observe h label).counts.length = h.counts.length := by simp [observe]

-- Structural proofs avoid the generic list accessor lemmas' choice dependency.
theorem increment_selected (counts : List Nat) (i : Nat) (hi : i < counts.length) :
    (counts.modify i (· + 1))[i]'(by rw [List.length_modify]; exact hi) = counts[i] + 1 := by
  induction counts generalizing i with
  | nil => exact (Nat.not_lt_zero _ hi).elim
  | cons c cs ih =>
    cases i with
    | zero => rfl
    | succ i => exact ih i (Nat.lt_of_succ_lt_succ hi)

theorem increment_other (counts : List Nat) (i j : Nat) (hj : j < counts.length) (hne : i ≠ j) :
    (counts.modify i (· + 1))[j]'(by rw [List.length_modify]; exact hj) = counts[j] := by
  induction counts generalizing i j with
  | nil => exact (Nat.not_lt_zero _ hj).elim
  | cons c cs ih =>
    cases i with
    | zero =>
      cases j with
      | zero => exact (hne rfl).elim
      | succ j => rfl
    | succ i =>
      cases j with
      | zero => rfl
      | succ j => exact ih i j (Nat.lt_of_succ_lt_succ hj) (fun eq => hne (congrArg Nat.succ eq))

theorem observed_count (h : Histogram) (label : Fin h.counts.length) :
    (observe h label).counts[label.val]'(by simp [observe]) =
      h.counts[label.val] + 1 := increment_selected _ _ label.isLt

theorem other_count (h : Histogram) (label other : Fin h.counts.length)
    (hne : label.val ≠ other.val) :
    (observe h label).counts[other.val]'(by simp [observe]) =
      h.counts[other.val] := increment_other _ _ _ other.isLt hne

theorem increment_sum (counts : List Nat) (i : Nat) (hi : i < counts.length) :
    (counts.modify i (· + 1)).sum = counts.sum + 1 := by
  induction counts generalizing i with
  | nil => simp at hi
  | cons c cs ih =>
    cases i with
    | zero => simp [Nat.add_assoc, Nat.add_comm, Nat.add_left_comm]
    | succ i =>
      have ht : i < cs.length := by simpa using hi
      simpa [Nat.add_assoc] using congrArg (c + ·) (ih i ht)

theorem total_increments (h : Histogram) (label : Fin h.counts.length) :
    (observe h label).counts.sum = h.counts.sum + 1 := increment_sum _ _ label.isLt

theorem numerator_sum (counts : List Nat) :
    (counts.map (fun c => 2 * c + 1)).sum = 2 * counts.sum + counts.length := by
  induction counts with
  | nil => rfl
  | cons c cs ih => simp [ih, Nat.mul_add]; omega

def distribution (h : Histogram) : List Fraction := List.ofFn (predict h)

theorem distribution_normalized (h : Histogram) : Equal (sum (distribution h)) one := by
  have mapped : distribution h =
      (h.counts.map (fun c => 2 * c + 1)).map
        (fun n => (⟨n, denominator h, denominator_positive h⟩ : Fraction)) := by
    simp only [distribution, List.map_map, Function.comp_def]
    have := congrArg (List.map (fun c => (⟨2 * c + 1, denominator h,
      denominator_positive h⟩ : Fraction))) (List.ofFn_getElem (xs := h.counts))
    rw [List.map_ofFn] at this
    exact this
  rw [mapped]
  apply eq_trans (sum_common_denominator _ _ _)
  simp [Equal, one, numerator_sum, denominator]

theorem continuation_normalized (h : Histogram) (next : Fin h.counts.length → Fraction)
    (normalized : ∀ i, Equal (next i) one) :
    Equal (sum (List.ofFn (fun i => mul (predict h i) (next i)))) one := by
  have hc := sum_congr (List.ofFn (fun i : Fin h.counts.length => i))
    (fun i => mul (predict h i) (next i)) (predict h)
    (fun i _ => eq_trans (mul_congr_left _ (normalized i)) (mul_one _))
  simp only [List.map_ofFn, Function.comp_def] at hc
  exact eq_trans hc (distribution_normalized h)

-- The complete future tree includes every symbol in the frozen alphabet.
-- This establishes normalization for every finite horizon, from ANY counts.
def futureMass (h : Histogram) : Nat → Fraction
  | 0 => one
  | n + 1 => sum (List.ofFn (fun i => mul (predict h i) (futureMass (observe h i) n)))

theorem future_normalized (h : Histogram) (n : Nat) : Equal (futureMass h n) one := by
  induction n generalizing h with
  | zero => exact eq_refl _
  | succ n ih =>
    change Equal (sum _) one
    exact continuation_normalized h _ (fun i => ih (observe h i))

-- Unknown labels are rejected; neither alphabet growth nor skipped observations.
def path (h : Histogram) : List Nat → Option (Fraction × Histogram)
  | [] => some (one, h)
  | label :: tail =>
    if hi : label < h.counts.length then
      match path (observe h ⟨label, hi⟩) tail with
      | none => none
      | some (q, final) => some (mul (predict h ⟨label, hi⟩) q, final)
    else none

theorem path_positive (h : Histogram) (labels : List Nat) (q : Fraction) (final : Histogram)
    (accepted : path h labels = some (q, final)) : 0 < q.numerator := by
  induction labels generalizing h q final with
  | nil => cases accepted; decide
  | cons label tail ih =>
    simp only [path] at accepted
    split at accepted
    · rename_i hi
      split at accepted
      · contradiction
      · rename_i q' final' step
        cases accepted
        exact Nat.mul_pos (predict_positive _ _) (ih _ _ _ step)
    · contradiction

theorem path_preserves_alphabet (h : Histogram) (labels : List Nat) (q : Fraction) (final : Histogram)
    (accepted : path h labels = some (q, final)) : final.counts.length = h.counts.length := by
  induction labels generalizing h q final with
  | nil => cases accepted; rfl
  | cons label tail ih =>
    simp only [path] at accepted
    split at accepted
    · split at accepted
      · contradiction
      · rename_i q' final' step
        cases accepted
        exact (ih _ _ _ step).trans (alphabet_unchanged _ _)
    · contradiction

theorem path_counts_every_row (h : Histogram) (labels : List Nat) (q : Fraction) (final : Histogram)
    (accepted : path h labels = some (q, final)) : final.counts.sum = h.counts.sum + labels.length := by
  induction labels generalizing h q final with
  | nil => cases accepted; simp
  | cons label tail ih =>
    simp only [path] at accepted
    split at accepted
    · split at accepted
      · contradiction
      · rename_i q' final' step
        cases accepted
        have := ih _ _ _ step
        rw [total_increments] at this
        simpa [Nat.add_assoc, Nat.add_comm, Nat.add_left_comm] using this
    · contradiction

-- A held-out suffix begins with the supplied training counts and has its own
-- predictive likelihood. Composing samples does not reset or skip those counts.
theorem path_append (h mid final : Histogram) (xs ys : List Nat) (q r : Fraction)
    (first : path h xs = some (q, mid)) (second : path mid ys = some (r, final)) :
    path h (xs ++ ys) = some (mul q r, final) := by
  induction xs generalizing h q with
  | nil => cases first; simpa [mul, one] using second
  | cons label tail ih =>
    simp only [path] at first
    split at first
    · rename_i hi
      split at first
      · contradiction
      · rename_i q' mid' step
        cases first
        simp only [List.cons_append, path, hi, ↓reduceDIte, ih _ _ step,
          mul_assoc_exact]
    · contradiction

-- Each context owns separate counts. A context can depend on prior observations
-- but is selected before the next symbol. This does not justify selecting it
-- using future labels, fitting on held-out outcomes or resetting paid look names.
def observeContext [DecidableEq Context] (bank : Context → Histogram) (context : Context)
    (label : Fin (bank context).counts.length) : Context → Histogram :=
  fun c => if c = context then observe (bank context) label else bank c

theorem context_updated [DecidableEq Context] (bank : Context → Histogram) (c : Context)
    (label : Fin (bank c).counts.length) : observeContext bank c label c = observe (bank c) label := by
  simp [observeContext]

theorem other_context_unchanged [DecidableEq Context] (bank : Context → Histogram) (c other : Context)
    (label : Fin (bank c).counts.length) (hne : other ≠ c) :
    observeContext bank c label other = bank other := by simp [observeContext, hne]

variable {Context : Type}

/-- A child partition changes which count cell is used, not its declared
alphabet. This invariant survives every observation in any selected cell. -/
def FixedSupport (bank : Context → Histogram) (size : Nat) : Prop :=
  ∀ c, (bank c).counts.length = size

theorem observe_fixed_support [DecidableEq Context] (bank : Context → Histogram)
    (size : Nat) (fixed : FixedSupport bank size) (c : Context)
    (label : Fin (bank c).counts.length) :
    FixedSupport (observeContext bank c label) size := by
  intro other
  by_cases same : other = c
  · subst other
    rw [context_updated, alphabet_unchanged, fixed]
  · rw [other_context_unchanged bank c other label same, fixed]

/-- Context selection sees only the prefix, never the current label. The
same ordered word can therefore be evaluated by different causal partitions. -/
def contextPath [DecidableEq Context] (choose : List Nat → Context)
    (bank : Context → Histogram) (past : List Nat) :
    List Nat → Option (Fraction × (Context → Histogram))
  | [] => some (one, bank)
  | label :: rest =>
      let c := choose past
      if valid : label < (bank c).counts.length then
        (contextPath choose (observeContext bank c ⟨label, valid⟩) (label :: past) rest).map
          (fun (mass, final) => (mul (predict (bank c) ⟨label, valid⟩) mass, final))
      else none

theorem context_path_positive [DecidableEq Context] (choose : List Nat → Context)
    (bank : Context → Histogram) (past labels : List Nat) (mass : Fraction) (final : Context → Histogram)
    (accepted : contextPath choose bank past labels = some (mass, final)) :
    0 < mass.numerator := by
  induction labels generalizing bank past mass final with
  | nil => cases accepted; decide
  | cons label rest ih =>
    simp only [contextPath] at accepted
    split at accepted
    · rename_i valid
      cases tail : contextPath choose (observeContext bank (choose past) ⟨label, valid⟩)
          (label :: past) rest with
      | none => simp [tail] at accepted
      | some result =>
        rcases result with ⟨next, final'⟩
        simp only [tail, Option.map_some, Option.some.injEq, Prod.mk.injEq] at accepted
        rcases accepted with ⟨rfl, rfl⟩
        exact Nat.mul_pos (predict_positive _ _) (ih _ _ _ _ tail)
    · cases accepted

theorem context_path_fixed_support [DecidableEq Context] (choose : List Nat → Context)
    (bank : Context → Histogram) (past labels : List Nat) (size : Nat)
    (fixed : FixedSupport bank size) (mass : Fraction) (final : Context → Histogram)
    (accepted : contextPath choose bank past labels = some (mass, final)) :
    FixedSupport final size := by
  induction labels generalizing bank past mass final with
  | nil => cases accepted; exact fixed
  | cons label rest ih =>
    simp only [contextPath] at accepted
    split at accepted
    · rename_i valid
      cases tail : contextPath choose (observeContext bank (choose past) ⟨label, valid⟩)
          (label :: past) rest with
      | none => simp [tail] at accepted
      | some result =>
        rcases result with ⟨next, final'⟩
        simp only [tail, Option.map_some, Option.some.injEq, Prod.mk.injEq] at accepted
        rcases accepted with ⟨rfl, rfl⟩
        exact ih _ _ (observe_fixed_support bank size fixed _ _) _ _ tail
    · cases accepted

/-- Exact domain equality, not merely normalization of two different trees.
Every finite word in the frozen alphabet is accepted, and no other word is. -/
theorem context_path_domain [DecidableEq Context] (choose : List Nat → Context)
    (bank : Context → Histogram) (past labels : List Nat) (size : Nat)
    (fixed : FixedSupport bank size) :
    (contextPath choose bank past labels).isSome = labels.all (fun label => label < size) := by
  induction labels generalizing bank past with
  | nil => rfl
  | cons label rest ih =>
    simp only [contextPath, List.all_cons]
    by_cases valid : label < (bank (choose past)).counts.length
    · rw [dite_eq_left valid, Option.isSome_map,
        ih _ _ (observe_fixed_support bank size fixed _ _)]
      have bound : label < size := by simpa [fixed (choose past)] using valid
      simp [bound]
    · rw [dite_eq_right valid]
      have outside : ¬ label < size := by simpa [fixed (choose past)] using valid
      simp [outside]

/-- Complete finite word space of a declared alphabet, including unseen labels. -/
def words (size : Nat) : Nat → List (List Nat)
  | 0 => [[]]
  | n+1 => (List.ofFn (fun i : Fin size => i)).flatMap
      (fun i => (words size n).map (i.val :: ·))

def contextMass [DecidableEq Context] (choose : List Nat → Context)
    (bank : Context → Histogram) (past labels : List Nat) : Fraction :=
  ((contextPath choose bank past labels).map Prod.fst).getD zero

theorem context_mass_cons [DecidableEq Context] (choose : List Nat → Context)
    (bank : Context → Histogram) (past rest : List Nat) (label : Nat)
    (valid : label < (bank (choose past)).counts.length) :
    Equal (contextMass choose bank past (label :: rest))
      (mul (predict (bank (choose past)) ⟨label,valid⟩)
        (contextMass choose (observeContext bank (choose past) ⟨label,valid⟩) (label :: past) rest)) := by
  unfold contextMass
  simp only [contextPath, valid, ↓reduceDIte]
  cases tail : contextPath choose (observeContext bank (choose past) ⟨label,valid⟩)
      (label :: past) rest with
  | none => exact eq_symm (mul_zero _)
  | some result => exact eq_refl _

/-- The sum is over the same explicit words for every fixed-support partition;
it is the accepted path likelihood, not an unrelated normalization recurrence. -/
theorem context_words_normalized [DecidableEq Context] (choose : List Nat → Context)
    (bank : Context → Histogram) (past : List Nat) (size horizon : Nat)
    (fixed : FixedSupport bank size) :
    Equal (sum ((words size horizon).map (contextMass choose bank past))) one := by
  induction horizon generalizing bank past with
  | zero => exact add_zero _
  | succ n ih =>
      simp only [words, List.map_flatMap, List.map_map, Function.comp_def]
      apply eq_trans (sum_flatMap _ _)
      have branch (i : Fin size) :
          Equal (sum ((words size n).map (fun rest => contextMass choose bank past (i.val :: rest))))
            (predict (bank (choose past)) ⟨i.val,by rw [fixed]; exact i.isLt⟩) := by
        have valid : i.val < (bank (choose past)).counts.length := by rw [fixed]; exact i.isLt
        apply eq_trans (sum_congr _ _ _ (fun rest _ => context_mass_cons choose bank past rest i.val valid))
        apply eq_trans (sum_scale (words size n)
          (contextMass choose (observeContext bank (choose past) ⟨i.val,valid⟩) (i.val :: past))
          (predict (bank (choose past)) ⟨i.val,valid⟩))
        apply eq_trans (mul_congr_left _ (ih _ _ (observe_fixed_support bank size fixed _ _)))
        exact mul_one _
      apply eq_trans (sum_congr _ _ _ (fun i _ => branch i))
      simp only [List.map_ofFn, Function.comp_def]
      have size_eq := (fixed (choose past)).symm
      subst size
      exact distribution_normalized _

def contextFutureMass [DecidableEq Context] (choose : List Nat → Context)
    (bank : Context → Histogram) (past : List Nat) : Nat → Fraction
  | 0 => one
  | n + 1 =>
    let c := choose past
    sum (List.ofFn (fun i => mul (predict (bank c) i)
      (contextFutureMass choose (observeContext bank c i) (i.val :: past) n)))

theorem contextual_future_normalized [DecidableEq Context] (choose : List Nat → Context)
    (bank : Context → Histogram) (past : List Nat) (n : Nat) :
    Equal (contextFutureMass choose bank past n) one := by
  induction n generalizing bank past with
  | zero => exact eq_refl _
  | succ n ih => exact continuation_normalized (bank (choose past)) _ (fun _ => ih _ _)

-- The arithmetic gate takes an already established parent maximum likelihood.
-- Its dominance over each null law, causal/predictable contexts, frozen sample
-- and code/look accounting are external premises, not consequences of a score.
def statisticalGate (alpha allocation child parentMaximum : Fraction) : Prop :=
  Less parentMaximum (mul (mul alpha allocation) child)

def codePreference (allocation child parentPredictive : Fraction) : Prop :=
  Less parentPredictive (mul allocation child)

instance (alpha allocation child parentMaximum : Fraction) :
    Decidable (statisticalGate alpha allocation child parentMaximum) :=
  inferInstanceAs (Decidable (Less _ _))

instance (allocation child parentPredictive : Fraction) :
    Decidable (codePreference allocation child parentPredictive) :=
  inferInstanceAs (Decidable (Less _ _))

theorem statistical_threshold (alpha allocation child parentMaximum : Fraction)
    (ha : 0 < alpha.numerator) (hw : 0 < allocation.numerator)
    (hp : 0 < parentMaximum.numerator) :
    statisticalGate alpha allocation child parentMaximum ↔
      Less (divide one (mul alpha allocation) (Nat.mul_pos ha hw))
        (divide child parentMaximum hp) := (ratio_threshold _ _ _ hp _).symm

theorem code_preference_threshold (allocation child parentPredictive : Fraction)
    (hw : 0 < allocation.numerator) (hp : 0 < parentPredictive.numerator) :
    codePreference allocation child parentPredictive ↔
      Less (divide one allocation hw) (divide child parentPredictive hp) :=
  (ratio_threshold _ _ _ hp hw).symm

end CategoricalPrediction
