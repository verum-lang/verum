import VerumExternalReplay.CausalCoding
import VerumExternalReplay.CategoricalPrediction

/-! The causal binary kernel reuses the categorical predictor's exact counts,
probabilities and updates. Its path theorem identifies both likelihood and final
state with the existing categorical path, without a second scoring formula. -/
namespace BinaryPrediction
open ExactMass CausalCoding CategoricalPrediction
set_option warningAsError true

abbrev State := { h : Histogram // h.counts.length = 2 }

def fromCounts (zeros ones : Nat) : State := ⟨⟨[zeros, ones], by simp⟩, rfl⟩

def symbol (bit : Bool) : Nat := if bit then 1 else 0

def label (state : State) (bit : Bool) : Fin state.val.counts.length :=
  ⟨symbol bit, by rw [state.property]; cases bit <;> decide⟩

def next (state : State) (bit : Bool) : State :=
  ⟨observe state.val (label state bit), (alphabet_unchanged _ _).trans state.property⟩

theorem probabilities_normalized (state : State) :
    Equal (add (predict state.val (label state false)) (predict state.val (label state true))) one := by
  rcases state with ⟨⟨counts, nonempty⟩, width⟩
  cases counts with
  | nil => simp at width
  | cons a rest =>
    cases rest with
    | nil => simp at width
    | cons b rest =>
      cases rest with
      | cons c rest => simp at width
      | nil =>
        have normalized := distribution_normalized (⟨[a, b], nonempty⟩ : Histogram)
        exact eq_trans (add_congr_left _ (eq_symm (add_zero _))) normalized

def kernel : Kernel State where
  probability state bit := predict state.val (label state bit)
  next := next
  normalized := probabilities_normalized

theorem probability_positive (state : State) (bit : Bool) :
    0 < (kernel.probability state bit).numerator := predict_positive _ _

/-- Reusing the causal kernel preserves the complete categorical result, including
all retained training counts; equality is exact, not merely rational equivalence. -/
theorem path_correspondence (state : State) (body : List Bool) :
    path state.val (body.map symbol) =
      some (likelihood kernel state body, (advance kernel state body).val) := by
  induction body generalizing state with
  | nil => rfl
  | cons bit rest ih =>
    simp only [List.map_cons, path]
    have bound : symbol bit < state.val.counts.length := (label state bit).isLt
    rw [dite_eq_left bound]
    have tail := ih (next state bit)
    simp only [next, label] at tail
    rw [tail]
    rfl

theorem counts_every_bit (state : State) (body : List Bool) :
    (advance kernel state body).val.counts.sum = state.val.counts.sum + body.length := by
  have counted := path_counts_every_row _ _ _ _ (path_correspondence state body)
  simpa only [List.length_map] using counted

/-- Half-count smoothing gives positive mass to every complete bit body. -/
theorem likelihood_positive (state : State) (body : List Bool) :
    0 < (likelihood kernel state body).numerator :=
  path_positive _ _ _ _ (path_correspondence state body)

end BinaryPrediction
