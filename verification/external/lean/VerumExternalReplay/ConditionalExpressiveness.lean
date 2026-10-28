import VerumExternalReplay.PositionPrediction

/-! Expressiveness of one fixed-context, fixed-mask positional product model.
Normalization does not imply representation of correlated targets. The positive
witness avoids confusing the independence obstruction with smoothing at zero
mass. Mixtures, different contexts and other model families are not restricted
by this theorem. Causal kernel transport states a semantic contract, not an
acquisition, implementation, resource or cost theorem. -/
namespace ConditionalExpressiveness
open ExactMass CausalCoding
set_option warningAsError true

abbrev PairLaw := Bool → Bool → Fraction

def product (first second : Bool → Fraction) : PairLaw := fun x y => mul (first x) (second y)

def RankOne (law : PairLaw) : Prop :=
  Equal (mul (law false false) (law true true)) (mul (law false true) (law true false))

instance (law : PairLaw) : Decidable (RankOne law) := inferInstanceAs (Decidable (Equal _ _))

theorem product_rank_one (first second : Bool → Fraction) : RankOne (product first second) := by
  simp [RankOne, product, Equal, mul, Nat.mul_comm, Nat.mul_left_comm]

/-- Shared length and unchanged remaining-bit factors do not remove the
constraint on four complete targets differing at two positions. -/
theorem scaled_product_rank_one (scale : Fraction) (first second : Bool → Fraction) :
    RankOne (fun x y => mul scale (product first second x y)) := by
  simp [RankOne, product, Equal, mul, Nat.mul_comm, Nat.mul_left_comm]

theorem rank_one_congr (a b : PairLaw) (same : ∀ x y, Equal (a x y) (b x y))
    (h : RankOne a) : RankOne b := by
  have left := eq_trans (mul_congr_right (a true true) (same false false))
    (mul_congr_left (b false false) (same true true))
  have right := eq_trans (mul_congr_right (a true false) (same false true))
    (mul_congr_left (b false true) (same true false))
  exact eq_trans (eq_symm left) (eq_trans h right)

def correlated (x y : Bool) : Fraction := if x = y then ⟨3,8,by decide⟩ else ⟨1,8,by decide⟩

theorem correlated_positive (x y : Bool) : 0 < (correlated x y).numerator := by
  cases x <;> cases y <;> decide

theorem correlated_normalized :
    Equal (sum [correlated false false, correlated false true,
      correlated true false, correlated true true]) one := by decide

theorem correlated_not_rank_one : ¬ RankOne correlated := by decide

theorem correlated_not_product (first second : Bool → Fraction) :
    ¬ ∀ x y, Equal (product first second x y) (correlated x y) := by
  intro h
  exact correlated_not_rank_one (rank_one_congr _ _ h (product_rank_one first second))

/-- Fixed mask bits can be absorbed into the corresponding unary factor. -/
theorem fixed_mask_rank_one (first second : Bool → Fraction) (mask₀ mask₁ : Bool) :
    RankOne (fun x y => product first second (Bool.xor mask₀ x) (Bool.xor mask₁ y)) :=
  product_rank_one (fun x => first (Bool.xor mask₀ x)) (fun y => second (Bool.xor mask₁ y))

structure PositionLaw where
  probability : Nat → Bool → Fraction
  normalized : ∀ i, Equal (add (probability i false) (probability i true)) one

def positionalKernel (law : PositionLaw) : Kernel Nat where
  probability := law.probability
  next position _ := position+1
  normalized := law.normalized

def positional (law : PositionLaw) (position : Nat) : List Bool → Fraction
  | [] => one
  | bit :: rest => mul (law.probability position bit) (positional law (position+1) rest)

theorem positional_embedding (law : PositionLaw) (position : Nat) (body : List Bool) :
    likelihood (positionalKernel law) position body = positional law position body := by
  induction body generalizing position with
  | nil => rfl
  | cons bit rest ih =>
      change mul (law.probability position bit) (likelihood (positionalKernel law) (position+1) rest) = _
      exact congrArg (mul (law.probability position bit)) (ih (position+1))

theorem positional_pair_rank_one (law : PositionLaw) (position : Nat) :
    RankOne (fun x y => positional law position [x,y]) := by
  apply rank_one_congr (product (law.probability position) (law.probability (position+1))) _ ?_
    (product_rank_one _ _)
  intro x y
  exact eq_symm (mul_congr_left _ (mul_one _))

theorem frozen_pair_rank_one {Key : Type} [DecidableEq Key]
    (key : Nat → Key) (bank : PositionPrediction.Bank Key) (position : Nat) :
    RankOne (fun x y => PositionPrediction.frozen key bank position [x,y]) := by
  let law : PositionLaw := ⟨fun i bit => BinaryPrediction.kernel.probability
    (PositionPrediction.lookup bank (key i)) bit,
    fun i => BinaryPrediction.probabilities_normalized _⟩
  exact positional_pair_rank_one law position

/-- A general extension must commute with state evolution and preserve each
conditional probability. Matching a current read alone is insufficient. -/
theorem kernel_transport {S T : Type} (source : Kernel S) (target : Kernel T) (embed : S → T)
    (probability : ∀ state bit, Equal (target.probability (embed state) bit) (source.probability state bit))
    (transition : ∀ state bit, target.next (embed state) bit = embed (source.next state bit))
    (state : S) (body : List Bool) :
    Equal (likelihood target (embed state) body) (likelihood source state body) := by
  induction body generalizing state with
  | nil => exact eq_refl _
  | cons bit rest ih =>
      simp only [likelihood,transition]
      exact eq_trans (mul_congr_right _ (probability state bit)) (mul_congr_left _ (ih _))

inductive Prefix where
  | start | first : Bool → Prefix | done

def autoregressive : Kernel Prefix where
  probability state bit := match state with
    | .start | .done => ⟨1,2,by decide⟩
    | .first previous => if previous = bit then ⟨3,4,by decide⟩ else ⟨1,4,by decide⟩
  next state bit := match state with | .start => .first bit | _ => .done
  normalized state := by cases state with
    | start => decide
    | done => decide
    | first previous => cases previous <;> decide

theorem autoregressive_realizes (x y : Bool) :
    Equal (likelihood autoregressive .start [x,y]) (correlated x y) := by
  cases x <;> cases y <;> decide

theorem autoregressive_normalized (n : Nat) :
    Equal (sum ((words n).map (likelihood autoregressive .start))) one :=
  body_normalized autoregressive .start n

theorem causal_strictly_exceeds_positional (law : PositionLaw) (position : Nat) :
    ¬ ∀ x y, Equal (positional law position [x,y]) (likelihood autoregressive .start [x,y]) := by
  intro h
  apply correlated_not_rank_one
  exact rank_one_congr _ correlated (fun x y => eq_trans (h x y) (autoregressive_realizes x y))
    (positional_pair_rank_one law position)

end ConditionalExpressiveness
