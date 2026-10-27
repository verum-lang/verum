import VerumExternalReplay.CategoricalPrediction

/- Conditional finite-sample error bound. Every outcome below is an entire
   finite experiment path (including the information needed for each paid look),
   not a single marginal row. Predictor mass and null dominance are supplied on
   this SAME sample space, conditional on a frozen training protocol if used.
   The contract does not derive them from counts, IID, native ML or causal data.
   No infinite-horizon, optional-stopping or native correspondence claim follows. -/
namespace FiniteAdmission
open ExactMass CategoricalPrediction
set_option warningAsError true

structure Experiment (Outcome : Type) where
  outcomes : List Outcome
  distinct : outcomes.Nodup
  nullMass : Outcome → Fraction
  normalized : Equal (sum (outcomes.map nullMass)) one

variable {Outcome : Type} {experiment : Experiment Outcome}

def eventMass (experiment : Experiment Outcome) (event : Outcome → Bool) : Fraction :=
  sum (experiment.outcomes.map (fun x => if event x then experiment.nullMass x else zero))

structure Test (experiment : Experiment Outcome) where
  allocation : Fraction
  predictive : Outcome → Fraction
  parentMaximum : Outcome → Fraction
  proper : AtMost (sum (experiment.outcomes.map predictive)) one
  dominates : ∀ x ∈ experiment.outcomes, AtMost (experiment.nullMass x) (parentMaximum x)

def rejects (alpha : Fraction) (test : Test experiment) (x : Outcome) : Bool :=
  decide (statisticalGate alpha test.allocation (test.predictive x) (test.parentMaximum x))

theorem rejection_pointwise (experiment : Experiment Outcome) (alpha : Fraction)
    (test : Test experiment) (x : Outcome) (hx : x ∈ experiment.outcomes) :
    AtMost (if rejects alpha test x then experiment.nullMass x else zero)
      (mul (mul alpha test.allocation) (test.predictive x)) := by
  unfold rejects
  split
  · rename_i rejected
    have h : statisticalGate alpha test.allocation (test.predictive x) (test.parentMaximum x) :=
      of_decide_eq_true rejected
    exact le_trans (test.dominates x hx) (less_le h)
  · exact zero_le _

theorem single_test_error (experiment : Experiment Outcome) (alpha : Fraction) (test : Test experiment) :
    AtMost (eventMass experiment (rejects alpha test)) (mul alpha test.allocation) := by
  apply le_trans (sum_mono experiment.outcomes _ _ (rejection_pointwise experiment alpha test))
  apply le_trans (eq_le (sum_scale _ _ _))
  exact le_trans (mul_mono_left _ test.proper) (eq_le (mul_one _))

theorem event_empty (experiment : Experiment Outcome) :
    Equal (eventMass experiment (fun _ => false)) zero := by
  unfold eventMass
  induction experiment.outcomes with
  | nil => exact eq_refl _
  | cons x xs ih => exact eq_trans (add_congr_left _ ih) (add_zero zero)

theorem event_mono (experiment : Experiment Outcome) (a b : Outcome → Bool)
    (included : ∀ x ∈ experiment.outcomes, a x = true → b x = true) :
    AtMost (eventMass experiment a) (eventMass experiment b) := by
  apply sum_mono
  intro x hx
  cases ha : a x with
  | false => exact zero_le _
  | true => simp only [included x hx ha, ↓reduceIte]; exact le_refl _

theorem event_union (experiment : Experiment Outcome) (a b : Outcome → Bool) :
    AtMost (eventMass experiment (fun x => a x || b x))
      (add (eventMass experiment a) (eventMass experiment b)) := by
  apply le_trans _ (eq_le (sum_add experiment.outcomes _ _))
  apply sum_mono
  intro x _
  cases ha : a x <;> cases hb : b x <;> simp [ha, hb]
  · exact zero_le _
  · exact eq_le (eq_symm (eq_trans (add_comm _ _) (add_zero _)))
  · exact eq_le (eq_symm (add_zero _))
  · exact le_add _ _

def anyReject (alpha : Fraction) (tests : List (Test experiment)) (x : Outcome) : Bool :=
  tests.any (fun test => rejects alpha test x)

theorem finite_union_error (experiment : Experiment Outcome) (alpha : Fraction)
    (tests : List (Test experiment)) :
    AtMost (eventMass experiment (anyReject alpha tests))
      (mul alpha (sum (tests.map Test.allocation))) := by
  induction tests with
  | nil => exact le_trans (eq_le (event_empty _)) (eq_le (eq_symm (mul_zero _)))
  | cons test tests ih =>
    apply le_trans (event_union experiment (rejects alpha test) (anyReject alpha tests))
    apply le_trans (add_mono_right _ (single_test_error experiment alpha test))
    apply le_trans (add_mono_left _ ih)
    exact eq_le (eq_symm (mul_add _ _ _))

-- Every list entry is charged. Repeated names do not receive a free second look.
theorem paid_family_error (experiment : Experiment Outcome) (alpha : Fraction)
    (tests : List (Test experiment)) (paid : AtMost (sum (tests.map Test.allocation)) one) :
    AtMost (eventMass experiment (anyReject alpha tests)) alpha :=
  le_trans (finite_union_error experiment alpha tests)
    (le_trans (mul_mono_left _ paid) (eq_le (mul_one _)))

-- A data-dependent choice among the already paid finite family is covered when
-- every possible rejection implies a rejection by one of those same tests.
theorem selected_family_error (experiment : Experiment Outcome) (alpha : Fraction)
    (tests : List (Test experiment)) (paid : AtMost (sum (tests.map Test.allocation)) one)
    (selected : Outcome → Bool)
    (covered : ∀ x ∈ experiment.outcomes, selected x = true → anyReject alpha tests x = true) :
    AtMost (eventMass experiment selected) alpha :=
  le_trans (event_mono experiment selected (anyReject alpha tests) covered)
    (paid_family_error experiment alpha tests paid)

end FiniteAdmission
