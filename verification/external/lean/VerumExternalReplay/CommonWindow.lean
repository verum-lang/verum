import VerumExternalReplay.CategoricalPrediction

/-! Descriptive comparison on a fixed common observation space.
Window keys stand for retained allocation identities. Source extraction must
bind them to actual immutable rows, fixed support, completed native program
readings, and original name/look masses. These laws grant no fresh evidence,
installation authority, statistical admission, or native resource guarantee.
-/
namespace CommonWindow
open ExactMass CategoricalPrediction
set_option warningAsError true

/-- The complete declared alphabet, including labels absent from the window.
Indices have the same meaning for both programs. Constructing this declaration
before intake is an external causality obligation, not inferred from a list. -/
structure Alphabet (Label : Type) where
  labels : List Label
  distinct : labels.Nodup
  nonempty : 0 < labels.length

structure Partition (alphabet : Alphabet Label) (Cell : Type) where
  counts : Cell → Histogram
  fixed : FixedSupport counts alphabet.labels.length
  choose : List Nat → Cell

def likelihood [DecidableEq Cell] (partition : Partition alphabet Cell)
    (past labels : List Nat) : Option Fraction :=
  (contextPath partition.choose partition.counts past labels).map Prod.fst

theorem likelihood_domain [DecidableEq Cell] (partition : Partition alphabet Cell)
    (past labels : List Nat) :
    (likelihood partition past labels).isSome =
      labels.all (fun label => label < alphabet.labels.length) := by
  rw [likelihood, Option.isSome_map]
  exact context_path_domain _ _ _ _ _ partition.fixed

/-- Both causal partitions have exactly the same finite word domain even
when their cell sets, counts, and prefix-dependent selectors differ. -/
theorem shared_domain [DecidableEq Old] [DecidableEq New]
    (old : Partition alphabet Old) (new : Partition alphabet New) (past labels : List Nat) :
    (likelihood old past labels).isSome = (likelihood new past labels).isSome := by
  rw [likelihood_domain, likelihood_domain]

theorem child_normalized [DecidableEq Cell] (partition : Partition alphabet Cell)
    (past : List Nat) (horizon : Nat) :
    Equal (contextFutureMass partition.choose partition.counts past horizon) one :=
  contextual_future_normalized _ _ _ _

theorem both_normalized [DecidableEq Old] [DecidableEq New]
    (old : Partition alphabet Old) (new : Partition alphabet New) (past : List Nat) (horizon : Nat) :
    Equal (contextFutureMass old.choose old.counts past horizon) one ∧
      Equal (contextFutureMass new.choose new.counts past horizon) one :=
  ⟨child_normalized old past horizon, child_normalized new past horizon⟩

theorem shared_words_normalized [DecidableEq Old] [DecidableEq New]
    (old : Partition alphabet Old) (new : Partition alphabet New) (past : List Nat) (horizon : Nat) :
    let space := words alphabet.labels.length horizon
    Equal (sum (space.map (contextMass old.choose old.counts past))) one ∧
      Equal (sum (space.map (contextMass new.choose new.counts past))) one :=
  ⟨context_words_normalized _ _ _ _ _ old.fixed, context_words_normalized _ _ _ _ _ new.fixed⟩

theorem child_positive [DecidableEq Cell] (partition : Partition alphabet Cell)
    (past labels : List Nat) (mass : Fraction) (accepted : likelihood partition past labels = some mass) :
    0 < mass.numerator := by
  unfold likelihood at accepted
  cases path : contextPath partition.choose partition.counts past labels with
  | none => simp [path] at accepted
  | some result =>
    rcases result with ⟨actual, final⟩
    simp only [path, Option.map_some, Option.some.injEq] at accepted
    subst mass
    exact context_path_positive _ _ _ _ _ _ path

/-- Replay supplied cell readings against the declared alphabet, starting with
empty counts. This checks their scoring, not the program that produced them or
whether it read future outcomes. The total selector's fallback is never used
on the supplied word: its position is the length of the already scored prefix. -/
def recordedPartition (alphabet : Alphabet Label) (first : Cell)
    (readings : List Cell) : Partition alphabet Cell where
  counts := fun _ => ⟨List.replicate alphabet.labels.length 0, by
    simpa using alphabet.nonempty⟩
  fixed := by intro c; simp
  choose := fun past => readings[past.length]?.getD first

def recordedMass [DecidableEq Label] [DecidableEq Cell] (alphabet : Alphabet Label)
    (rows : List (Cell × Label)) : Option Fraction :=
  match rows with
  | [] => some one
  | (first, _) :: _ =>
    likelihood (recordedPartition alphabet first (rows.map Prod.fst)) []
      (rows.map (fun row => alphabet.labels.idxOf row.2))

-- Use the constructive indexing witnesses; the standard combined equivalence
-- currently has Classical.choice in its transitive axiom graph.
theorem named_index_domain [DecidableEq Label] (labels : List Label) (label : Label) :
    labels.idxOf label < labels.length ↔ label ∈ labels := by
  constructor
  · intro within
    simpa only [List.getElem_idxOf within] using List.getElem_mem within
  · exact List.idxOf_lt_length_of_mem

theorem recorded_mass_domain [DecidableEq Label] [DecidableEq Cell]
    (alphabet : Alphabet Label) (rows : List (Cell × Label)) :
    (recordedMass alphabet rows).isSome = rows.all (fun row => row.2 ∈ alphabet.labels) := by
  cases rows with
  | nil => rfl
  | cons row rest =>
    simp only [recordedMass, likelihood_domain, List.all_map, Function.comp_def, named_index_domain]

theorem recorded_mass_positive [DecidableEq Label] [DecidableEq Cell]
    (alphabet : Alphabet Label) (rows : List (Cell × Label)) (mass : Fraction)
    (accepted : recordedMass alphabet rows = some mass) : 0 < mass.numerator := by
  cases rows with
  | nil => cases Option.some.inj accepted; exact Nat.zero_lt_one
  | cons row rest => exact child_positive _ _ _ _ accepted

theorem recorded_selector_exact (alphabet : Alphabet Label) (first : Cell)
    (readings : List Cell) (past : List Nat) (within : past.length < readings.length) :
    (recordedPartition alphabet first readings).choose past = readings[past.length] := by
  simp [recordedPartition, List.getElem?_eq_getElem within]

/-- Keep the original program-name and paid-look allocations as separate
factors. Reassessment cannot mint a new allocation or erase one of them. -/
structure Price where
  name : Fraction
  look : Fraction
  namePositive : 0 < name.numerator
  lookPositive : 0 < look.numerator

def Price.mass (price : Price) : Fraction := mul price.name price.look

theorem Price.positive (price : Price) : 0 < price.mass.numerator :=
  Nat.mul_pos price.namePositive price.lookPositive

structure Assessment (Window Label : Type) where
  window : Window
  alphabet : Alphabet Label
  child : Fraction
  reference : Fraction
  price : Price
  childPositive : 0 < child.numerator
  referencePositive : 0 < reference.numerator

def compression (a : Assessment Window Label) : Fraction :=
  divide a.child a.reference a.referencePositive

theorem compression_positive (a : Assessment Window Label) : 0 < (compression a).numerator :=
  divide_positive _ _ a.childPositive a.referencePositive

/-- This is a proof-obligation checker. Native pointer equality alone relies
on immutable Window construction to supply equal support and reference. -/
def compatible [DecidableEq Window] [DecidableEq Label]
    (old new : Assessment Window Label) : Bool :=
  decide (old.window = new.window ∧ old.alphabet.labels = new.alphabet.labels ∧
    Equal old.reference new.reference)

def ratio (old new : Assessment Window Label) : Fraction :=
  mul (divide (compression new) (compression old) (compression_positive old))
    (divide new.price.mass old.price.mass old.price.positive)

def compare [DecidableEq Window] [DecidableEq Label]
    (old new : Assessment Window Label) : Option Fraction :=
  if compatible old new then some (ratio old new) else none

theorem compatible_exact [DecidableEq Window] [DecidableEq Label]
    (old new : Assessment Window Label) : compatible old new = true ↔
    old.window = new.window ∧ old.alphabet.labels = new.alphabet.labels ∧
      Equal old.reference new.reference := by simp [compatible]

theorem foreign_window_refused [DecidableEq Window] [DecidableEq Label]
    (old new : Assessment Window Label) (foreign : old.window ≠ new.window) :
    compare old new = none := by simp [compare, compatible, foreign]

theorem foreign_support_refused [DecidableEq Window] [DecidableEq Label]
    (old new : Assessment Window Label) (foreign : old.alphabet.labels ≠ new.alphabet.labels) :
    compare old new = none := by simp [compare, compatible, foreign]

theorem foreign_reference_refused [DecidableEq Window] [DecidableEq Label]
    (old new : Assessment Window Label) (foreign : ¬ Equal old.reference new.reference) :
    compare old new = none := by simp [compare, compatible, foreign]

theorem reference_cancels (old new : Assessment Window Label)
    (same : Equal old.reference new.reference) :
    Equal (divide (compression new) (compression old) (compression_positive old))
      (divide new.child old.child old.childPositive) := by
  have newReference := divide_congr (eq_refl new.child) (eq_symm same)
    new.referencePositive old.referencePositive
  exact eq_trans (divide_congr newReference (eq_refl (compression old))
    (compression_positive old) (compression_positive old))
    (divide_common_reference new.child old.child old.reference old.childPositive old.referencePositive)

theorem weighted_ratio_exact (old new : Assessment Window Label)
    (same : Equal old.reference new.reference) :
    Equal (ratio old new)
      (divide (mul new.child new.price.mass) (mul old.child old.price.mass)
        (Nat.mul_pos old.childPositive old.price.positive)) := by
  exact eq_trans (mul_congr_right _ (reference_cancels old new same))
    (divide_products new.child old.child new.price.mass old.price.mass
      old.childPositive old.price.positive)

theorem comparison_exact [DecidableEq Window] [DecidableEq Label]
    (old new : Assessment Window Label) (mass : Fraction) (accepted : compare old new = some mass) :
    old.window = new.window ∧ old.alphabet.labels = new.alphabet.labels ∧
      Equal mass (divide (mul new.child new.price.mass) (mul old.child old.price.mass)
        (Nat.mul_pos old.childPositive old.price.positive)) := by
  unfold compare at accepted
  split at accepted
  · rename_i guard
    have facts := (compatible_exact old new).mp guard
    cases Option.some.inj accepted
    exact ⟨facts.1, facts.2.1, weighted_ratio_exact old new facts.2.2⟩
  · cases accepted

end CommonWindow
