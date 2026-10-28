import VerumExternalReplay.BinaryPrediction

/-! Exact finite KT merger comparisons for ordered binary words. Each separate
word starts with zero counts; the merged word retains the first word's counts.
These are predictive code preferences, not statistical error guarantees or
asymptotic first-purchase thresholds. Counts-event probabilities would also
need their multinomial factors. -/
namespace KTMerge
open ExactMass CausalCoding BinaryPrediction CategoricalPrediction
set_option warningAsError true

def wordMass (word : List Bool) : Fraction :=
  likelihood kernel (fromCounts 0 0) word

theorem wordMass_positive (word : List Bool) : 0 < (wordMass word).numerator :=
  likelihood_positive _ _

def mergerRatio (left right : List Bool) : Fraction :=
  divide (wordMass (left ++ right)) (mul (wordMass left) (wordMass right))
    (Nat.mul_pos (wordMass_positive left) (wordMass_positive right))

/-- A fixed naming weight pays its price precisely at the strict ratio
comparison. Equality gives no gain. Assigning weights to a changing census or
an adaptively selected pair requires a separate coding/policy argument. -/
theorem merger_threshold (left right : List Bool) (weight : Fraction)
    (positiveWeight : 0 < weight.numerator) :
    codePreference weight (wordMass (left ++ right))
        (mul (wordMass left) (wordMass right)) ↔
      Less (divide one weight positiveWeight) (mergerRatio left right) :=
  code_preference_threshold _ _ _ positiveWeight _

def zeros16 : List Bool := List.replicate 16 false
def ones16 : List Bool := List.replicate 16 true

/-- Reduced rational witnesses are identified with the actual sequential
predictor, rather than introduced as assumptions about its score. -/
theorem same_ratio : Equal (mergerRatio zeros16 zeros16)
    ⟨315208830571481, 62142696120150, by decide⟩ := by decide_cbv

theorem opposite_ratio : Equal (mergerRatio zeros16 ones16)
    ⟨1, 601080390, by decide⟩ := by decide_cbv

theorem same_ratio_exceeds_three :
    Less ⟨3, 1, by decide⟩ (mergerRatio zeros16 zeros16) := by decide_cbv

theorem opposite_ratio_below_one :
    Less (mergerRatio zeros16 ones16) one := by decide_cbv

/-- The fixed pair weight 1/3 corresponds to price log2(3). No logarithm or
floating-point approximation is needed to decide the purchase. -/
theorem same_histories_pay_pair_price :
    codePreference (unit 3 (by decide)) (wordMass (zeros16 ++ zeros16))
      (mul (wordMass zeros16) (wordMass zeros16)) := by decide_cbv

theorem opposite_histories_do_not_pay_pair_price :
    ¬ codePreference (unit 3 (by decide)) (wordMass (zeros16 ++ ones16))
      (mul (wordMass zeros16) (wordMass ones16)) := by decide_cbv

/-- Both observed pairs have positive probability under the same fair IID
source. Sharing a source law does not imply matching empirical histories. -/
def fairIID : Kernel Unit where
  probability _ _ := unit 2 (by decide)
  next _ _ := ()
  normalized _ := by decide

theorem same_histories_possible_under_fair_iid :
    Equal (likelihood fairIID () (zeros16 ++ zeros16))
      (unit 4294967296 (by decide)) := by decide_cbv

theorem opposite_histories_possible_under_fair_iid :
    Equal (likelihood fairIID () (zeros16 ++ ones16))
      (unit 4294967296 (by decide)) := by decide_cbv

end KTMerge
