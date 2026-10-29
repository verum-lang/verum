import VerumExternalReplay.StoppedProcess
import VerumExternalReplay.FrozenTraining

/-! A frozen contextual KT bank chooses STOP and BIT cells using only a revealed
prefix. Cut and query may be captured by the total selector; future bytes and
final length are absent from its arguments. Native selector totality, bounded
counter fidelity, refusal/pauses and source correspondence remain obligations. -/
namespace ContextualStop
open ExactMass CausalCoding StoppedProcess FrozenTraining
set_option warningAsError true

inductive Kind where
  | stop | bit
  deriving DecidableEq

structure Selection (Key : Type) where
  key : Key
  invert : Bool

abbrev Selector (Key : Type) := Kind → List Bool → Selection Key
abbrev Bank (Key : Type) := Counts (Kind × Key)

def probability (select : Selector Key) (bank : Bank Key) (kind : Kind)
    (past : List Bool) (raw : Bool) : Fraction :=
  let cell := select kind past
  BinaryPrediction.kernel.probability
    (BinaryPrediction.fromCounts (bank (kind,cell.key) false) (bank (kind,cell.key) true))
    (Bool.xor cell.invert raw)

theorem probabilities_normalized (select : Selector Key) (bank : Bank Key)
    (kind : Kind) (past : List Bool) :
    Equal (add (probability select bank kind past false) (probability select bank kind past true)) one := by
  have normalized := BinaryPrediction.probabilities_normalized
    (BinaryPrediction.fromCounts (bank (kind,(select kind past).key) false)
      (bank (kind,(select kind past).key) true))
  cases h : (select kind past).invert with
  | false => simpa only [probability,h,Bool.false_xor,BinaryPrediction.kernel] using normalized
  | true =>
    simp only [probability,h,Bool.true_xor,Bool.not_false,Bool.not_true]
    apply eq_trans (add_comm _ _)
    exact normalized

theorem probability_positive (select : Selector Key) (bank : Bank Key)
    (kind : Kind) (past : List Bool) (raw : Bool) :
    0 < (probability select bank kind past raw).numerator := BinaryPrediction.probability_positive _ _

/-- A common denominator ceiling bounds both outcomes, including inversion. -/
theorem fraction_bounds (p : Fraction) (c : Nat)
    (denominator : p.denominator ≤ c+1) (positive : 0 < p.numerator)
    (complement : p.numerator+1 ≤ p.denominator) :
    AtMost (unit (c+1) (by omega)) p ∧ AtMost p (decay c 1) := by
  constructor
  · simp only [AtMost,unit,Nat.one_mul]
    exact Nat.le_trans denominator (Nat.le_mul_of_pos_left _ positive)
  · have scaled := Nat.mul_le_mul_right (c+1) complement
    simp only [Nat.add_mul,Nat.mul_add,Nat.mul_one,Nat.one_mul] at scaled
    simp only [AtMost,decay,Nat.pow_one,Nat.mul_add,Nat.mul_one,Nat.mul_comm]
    simp only [Nat.mul_comm] at scaled
    omega

theorem kt_bounds (zeros ones cap : Nat) (hz : zeros ≤ cap) (ho : ones ≤ cap) (bit : Bool) :
    let p := BinaryPrediction.kernel.probability (BinaryPrediction.fromCounts zeros ones) bit
    AtMost (unit (4*cap+2) (by omega)) p ∧ AtMost p (decay (4*cap+1) 1) := by
  apply fraction_bounds
  · simp [BinaryPrediction.kernel,BinaryPrediction.fromCounts,CategoricalPrediction.predict,
      CategoricalPrediction.denominator]
    omega
  · exact BinaryPrediction.probability_positive _ _
  · cases bit <;> simp [BinaryPrediction.kernel,BinaryPrediction.fromCounts,
      BinaryPrediction.label,BinaryPrediction.symbol,CategoricalPrediction.predict,
      CategoricalPrediction.denominator]
    all_goals omega

theorem probability_bounds (select : Selector Key) (bank : Bank Key) (kind : Kind)
    (cap : Nat) (bounded : ∀ key bit, bank (kind,key) bit ≤ cap) (past : List Bool) (raw : Bool) :
    AtMost (unit (4*cap+2) (by omega)) (probability select bank kind past raw) ∧
      AtMost (probability select bank kind past raw) (decay (4*cap+1) 1) :=
  kt_bounds _ _ cap (bounded _ false) (bounded _ true) _

def bitKernel (select : Selector Key) (bank : Bank Key) : Kernel (List Bool) where
  probability := probability select bank .bit
  next past bit := bit :: past
  normalized := probabilities_normalized select bank .bit

def model (select : Selector Key) (bank : Bank Key) (width : Nat) (positive : 0 < width) :
    Model (List Bool) where
  unitBits := width
  positiveWidth := positive
  body := bitKernel select bank
  stop := probability select bank .stop
  normalized := probabilities_normalized select bank .stop

theorem revealed_prefix (select : Selector Key) (bank : Bank Key) (past bits : List Bool) :
    CausalCoding.advance (bitKernel select bank) past bits = bits.reverse ++ past := by
  induction bits generalizing past with
  | nil => rfl
  | cons bit rest ih => simpa [CausalCoding.advance,bitKernel,List.reverse_cons,List.append_assoc] using ih (bit::past)

/-- A finite frozen u64 bank supplies a uniform floor even with infinitely many
prefixes and context keys. Bounded STOP cells suffice; BIT contexts may vary. -/
def law (select : Selector Key) (bank : Bank Key) (width : Nat) (positive : 0 < width)
    (cap : Nat) (bounded : ∀ key bit, bank (.stop,key) bit ≤ cap) (past : List Bool) :
    FiniteWire.LengthLaw :=
  lengthLaw (model select bank width positive) past (4*cap+1)
    (fun p => (probability_bounds select bank .stop cap bounded p false).2)

/-- Training counts canonical outcomes, while causal state retains raw bits. -/
def visit (select : Selector Key) (kind : Kind) (past : List Bool) (raw : Bool) : (Kind × Key) × Bool :=
  ((kind,(select kind past).key),Bool.xor (select kind past).invert raw)

def bitVisits (select : Selector Key) (past : List Bool) : List Bool → List ((Kind × Key) × Bool)
  | [] => []
  | bit :: rest => visit select .bit past bit :: bitVisits select (bit::past) rest

/-- Only whole rows contain a final STOP. Empty rows contain that single visit. -/
def rowVisits (select : Selector Key) (past : List Bool) : List (List Bool) → List ((Kind × Key) × Bool)
  | [] => [visit select .stop past true]
  | bits :: rest => visit select .stop past false ::
      (bitVisits select past bits ++ rowVisits select (bits.reverse ++ past) rest)

def eventProbability (bank : Bank Key) (event : (Kind × Key) × Bool) : Fraction :=
  BinaryPrediction.kernel.probability
    (BinaryPrediction.fromCounts (bank event.1 false) (bank event.1 true)) event.2

/-- Every factor reads the same pre-row bank, even if a cell is visited again. -/
def eventMass (bank : Bank Key) : List ((Kind × Key) × Bool) → Fraction
  | [] => one
  | event :: rest => mul (eventProbability bank event) (eventMass bank rest)

theorem event_mass_append (bank : Bank Key) (first rest : List ((Kind × Key) × Bool)) :
    Equal (eventMass bank (first ++ rest)) (mul (eventMass bank first) (eventMass bank rest)) := by
  induction first with
  | nil => exact eq_symm (eq_trans (mul_comm _ _) (mul_one _))
  | cons event first ih =>
    simp only [List.cons_append,eventMass,mul_assoc_exact]
    exact mul_congr_left _ ih

theorem bit_visits_mass (select : Selector Key) (bank : Bank Key) (past bits : List Bool) :
    Equal (eventMass bank (bitVisits select past bits))
      (likelihood (bitKernel select bank) past bits) := by
  induction bits generalizing past with
  | nil => exact eq_refl _
  | cons bit rest ih => exact mul_congr_left _ (ih (bit::past))

theorem row_visits_mass (select : Selector Key) (bank : Bank Key) (width : Nat)
    (positive : 0 < width) (past : List Bool) (blocks : List (List Bool)) :
    Equal (eventMass bank (rowVisits select past blocks))
      (point (model select bank width positive) past blocks) := by
  induction blocks generalizing past with
  | nil => exact eq_trans (mul_one _) (eq_symm (eq_trans (mul_comm _ _) (mul_one _)))
  | cons bits rest ih =>
    change Equal (mul (probability select bank .stop past false)
      (eventMass bank (bitVisits select past bits ++ rowVisits select (bits.reverse ++ past) rest))) _
    apply eq_trans (mul_congr_left _ (event_mass_append bank _ _))
    apply eq_trans (mul_congr_left _ (mul_congr_right _ (bit_visits_mass select bank past bits)))
    apply eq_trans (mul_congr_left _ (mul_congr_left _ (ih (bits.reverse ++ past))))
    simp only [point,cylinder,endpoint,model,mul_assoc_exact,revealed_prefix]
    exact eq_refl _

def publish [DecidableEq Key] (select : Selector Key) (bank : Bank Key)
    (past : List Bool) (blocks : List (List Bool)) : Bank Key :=
  fun key bit => bank key bit + collect (rowVisits select past blocks) (fun _ _ => 0) key bit

theorem published_multiplicity [DecidableEq Key] (select : Selector Key) (bank : Bank Key)
    (past : List Bool) (blocks : List (List Bool)) (key : Kind × Key) (bit : Bool) :
    publish select bank past blocks key bit = bank key bit +
      ((rowVisits select past blocks).filter (fun e => decide (e = (key,bit)))).length :=
  merged_multiplicity _ bank key bit

end ContextualStop
