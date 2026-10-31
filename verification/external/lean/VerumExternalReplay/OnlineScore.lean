import VerumExternalReplay.CategoricalScore
import VerumExternalReplay.CategoricalPrediction
import VerumExternalReplay.SharedUpdates

/-! Exact online updates of the existing finite categorical replay scorer.
The alphabet and old counts are fixed before each row. The two numerators
remain child KT; only the parent denominator differs. Natural arithmetic has
no overflow, and these laws do not authorize skipping invalid rows, changing
support, bypassing resource admission or replacing native ownership checks.
-/
namespace OnlineScore
open CategoricalScore
set_option warningAsError true

def Positive (q : Ratio) : Prop := 0 < q.numerator ∧ 0 < q.denominator

/-- Cross multiplication is used only with positive ratio operands below. -/
def Equal (a b : Ratio) : Prop := a.numerator * b.denominator = b.numerator * a.denominator

theorem equivalent_of_equal {a b : Ratio} (ha : Positive a) (hb : Positive b)
    (same : Equal a b) : equivalent a b = true := by
  simp [equivalent, Nat.ne_of_gt ha.2, Nat.ne_of_gt hb.2, Equal] at *
  exact same

theorem positive_mul {a b : Ratio} (ha : Positive a) (hb : Positive b) :
    Positive (mul a b) := ⟨Nat.mul_pos ha.1 hb.1, Nat.mul_pos ha.2 hb.2⟩

theorem positive_div {a b : Ratio} (ha : Positive a) (hb : Positive b) :
    Positive (div a b) := ⟨Nat.mul_pos ha.1 hb.2, Nat.mul_pos ha.2 hb.1⟩

theorem equal_mul {a b c d : Ratio} (h : Equal a b) (k : Equal c d) :
    Equal (mul a c) (mul b d) := by
  change (a.numerator*c.numerator)*(b.denominator*d.denominator) =
    (b.numerator*d.numerator)*(a.denominator*c.denominator)
  calc
    _ = (a.numerator*b.denominator)*(c.numerator*d.denominator) := by ac_rfl
    _ = (b.numerator*a.denominator)*(d.numerator*c.denominator) := by rw [h,k]
    _ = _ := by ac_rfl

theorem equal_div {a b c d : Ratio} (h : Equal a b) (k : Equal c d) :
    Equal (div a c) (div b d) := by
  change (a.numerator*c.denominator)*(b.denominator*d.numerator) =
    (b.numerator*d.denominator)*(a.denominator*c.numerator)
  calc
    _ = (a.numerator*b.denominator)*(d.numerator*c.denominator) := by ac_rfl
    _ = (b.numerator*a.denominator)*(c.numerator*d.denominator) := by rw [h,← k]
    _ = _ := by ac_rfl

theorem equal_trans {a b c : Ratio} (hb : Positive b) (h : Equal a b)
    (k : Equal b c) : Equal a c := by
  apply Nat.eq_of_mul_eq_mul_right hb.2
  change (a.numerator*c.denominator)*b.denominator = (c.numerator*a.denominator)*b.denominator
  calc
    _ = (a.numerator*b.denominator)*c.denominator := by ac_rfl
    _ = (b.numerator*a.denominator)*c.denominator := by rw [h]
    _ = (b.numerator*c.denominator)*a.denominator := by ac_rfl
    _ = (c.numerator*b.denominator)*a.denominator := by rw [k]
    _ = _ := by ac_rfl

private theorem product_positive (xs : List Nat) (h : ∀ x ∈ xs, 0 < x) :
    0 < product xs := by
  induction xs with
  | nil => decide
  | cons x xs ih => exact Nat.mul_pos (h x (by simp)) (ih (by intro y hy; exact h y (by simp [hy])))

private theorem rising_positive (base n : Nat) (h : 0 < base) : 0 < rising base n := by
  induction n with
  | zero => exact Nat.zero_lt_one
  | succ n ih => exact Nat.mul_pos ih (by omega)

private theorem self_power_positive (n : Nat) : 0 < n^n := by
  cases n with
  | zero => decide
  | succ n => exact Nat.pow_pos (by omega)

theorem kt_positive (K : Nat) (hK : 0 < K) (xs : List Nat) : Positive (kt K xs) := by
  constructor
  · apply product_positive
    intro x hx
    rcases List.mem_map.mp hx with ⟨n,_,rfl⟩
    exact rising_positive 1 n (by decide)
  · exact rising_positive K xs.sum hK

theorem ml_positive (xs : List Nat) : Positive (parentML xs) := by
  constructor
  · apply product_positive
    intro x hx
    rcases List.mem_map.mp hx with ⟨n,_,rfl⟩
    exact self_power_positive n
  · exact self_power_positive xs.sum

/-- Predict using the old count of the observed symbol and the old row total. -/
def ktStep (K total selected : Nat) : Ratio := ⟨2*selected+1, 2*total+K⟩

/-- Empirical parent likelihood ratio, including 0^0=1 at the first row. -/
def mlStep (total selected : Nat) : Ratio :=
  ⟨(selected+1)^(selected+1)*total^total,
    selected^selected*(total+1)^(total+1)⟩

theorem ktStep_positive (K total selected : Nat) (hK : 0 < K) :
    Positive (ktStep K total selected) := by constructor <;> simp [ktStep] <;> omega

theorem mlStep_positive (total selected : Nat) : Positive (mlStep total selected) :=
  ⟨Nat.mul_pos (self_power_positive _) (self_power_positive _),
    Nat.mul_pos (self_power_positive _) (self_power_positive _)⟩

private theorem product_modify (f : α → Nat) (xs : List α) (i : Nat) (hi : i < xs.length)
    (update : α → α) (factor : Nat) (step : f (update xs[i]) = f xs[i] * factor) :
    product ((xs.modify i update).map f) = product (xs.map f) * factor := by
  induction xs generalizing i with
  | nil => simp at hi
  | cons x xs ih =>
    cases i with
    | zero => simpa [product, Nat.mul_assoc, Nat.mul_comm, Nat.mul_left_comm] using
        congrArg (· * product (xs.map f)) step
    | succ i =>
      simpa [product, Nat.mul_assoc] using congrArg (f x * ·)
        (ih i (Nat.lt_of_succ_lt_succ hi) step)

private theorem product_modify_cross (f : α → Nat) (xs : List α) (i : Nat)
    (hi : i < xs.length) (update : α → α) :
    product ((xs.modify i update).map f) * f xs[i] =
      product (xs.map f) * f (update xs[i]) := by
  induction xs generalizing i with
  | nil => simp at hi
  | cons x xs ih =>
    cases i with
    | zero => simp only [List.modify, List.map, product, List.foldr_cons]; ac_rfl
    | succ i =>
      simpa [product, Nat.mul_assoc] using congrArg (f x * ·)
        (ih i (Nat.lt_of_succ_lt_succ hi))

theorem kt_step (K : Nat) (xs : List Nat) (i : Nat) (hi : i < xs.length) :
    kt K (xs.modify i (·+1)) = mul (kt K xs) (ktStep K xs.sum xs[i]) := by
  have hp := product_modify (rising 1) xs i hi (·+1) (2*xs[i]+1)
    (by simp [rising, Nat.add_comm])
  simp only [kt, ktStep, mul, hp, CategoricalPrediction.increment_sum xs i hi, rising,
    Nat.add_comm]

theorem ml_step (xs : List Nat) (i : Nat) (hi : i < xs.length) :
    Equal (parentML (xs.modify i (·+1))) (mul (parentML xs) (mlStep xs.sum xs[i])) := by
  have hp := product_modify_cross (fun n : Nat => n^n) xs i hi (·+1)
  simp only [Equal, parentML, mlStep, mul, CategoricalPrediction.increment_sum xs i hi]
  calc
    _ = (product ((xs.modify i (·+1)).map (fun n => n^n))*xs[i]^xs[i]) *
      (xs.sum^xs.sum*(xs.sum+1)^(xs.sum+1)) := by ac_rfl
    _ = (product (xs.map (fun n => n^n))*(xs[i]+1)^(xs[i]+1)) *
      (xs.sum^xs.sum*(xs.sum+1)^(xs.sum+1)) := by rw [hp]
    _ = _ := by ac_rfl

theorem kt_zeros (K width : Nat) : kt K (List.replicate width 0) = one := by
  have hp : product ((List.replicate width 0).map (rising 1)) = 1 := by
    induction width with
    | zero => rfl
    | succ n ih =>
        change 1 * product ((List.replicate n 0).map (rising 1)) = 1
        exact (Nat.one_mul _).trans ih
  unfold kt
  rw [hp]
  simp [rising, one]

theorem ml_zeros (width : Nat) : parentML (List.replicate width 0) = one := by
  have hp : product ((List.replicate width 0).map (fun n => n^n)) = 1 := by
    induction width with
    | zero => rfl
    | succ n ih =>
        change 1 * product ((List.replicate n 0).map (fun n => n^n)) = 1
        exact (Nat.one_mul _).trans ih
  unfold parentML
  rw [hp]
  simp [one]

/-- Allocating an unseen child contributes exactly one before its first row.
It does not restart the retained parent counts or alter the alphabet. -/
theorem new_child_neutral (K : Nat) (parent : List Nat) (children : List (List Nat)) (samples : Nat) :
    fromCounts K parent (children ++ [List.replicate K 0]) samples =
      fromCounts K parent children samples := by
  simp only [fromCounts, List.map_append, List.map_cons, List.map_nil, kt_zeros,
    List.foldr_append, List.foldr_cons, List.foldr_nil]
  rfl

/-- Product replacement for an arbitrary selected cell or context. All other
factors, including unseen and zero-count cells, remain in the same product. -/
theorem product_step (f : α → Ratio) (xs : List α) (i : Nat) (hi : i < xs.length)
    (update : α → α) (factor : Ratio)
    (step : Equal (f (update xs[i])) (mul (f xs[i]) factor)) :
    Equal (((xs.modify i update).map f).foldr mul one)
      (mul ((xs.map f).foldr mul one) factor) := by
  induction xs generalizing i with
  | nil => simp at hi
  | cons x xs ih =>
    cases i with
    | zero =>
      have h := equal_mul step (show Equal ((xs.map f).foldr mul one) _ from rfl)
      simpa [Equal, mul,
        Nat.mul_assoc, Nat.mul_comm, Nat.mul_left_comm] using h
    | succ i =>
      have h := equal_mul (show Equal (f x) (f x) from rfl)
        (ih i (Nat.lt_of_succ_lt_succ hi) step)
      simpa [Equal, mul,
        Nat.mul_assoc, Nat.mul_comm, Nat.mul_left_comm] using h

def scoreStep (K parentTotal parentSelected childTotal childSelected : Nat) : Score :=
  let child := ktStep K childTotal childSelected
  ⟨div child (ktStep K parentTotal parentSelected), div child (mlStep parentTotal parentSelected), 1⟩

def accumulate (score delta : Score) : Score :=
  ⟨mul score.compression delta.compression, mul score.predictive delta.predictive,
    score.samples + delta.samples⟩

def Agrees (a b : Score) : Prop :=
  Equal a.compression b.compression ∧ Equal a.predictive b.predictive ∧ a.samples = b.samples

def ScorePositive (s : Score) : Prop := Positive s.compression ∧ Positive s.predictive

theorem checked_agreement {a b : Score} (ha : ScorePositive a) (hb : ScorePositive b)
    (same : Agrees a b) : equivalent a.compression b.compression = true ∧
      equivalent a.predictive b.predictive = true ∧ a.samples = b.samples :=
  ⟨equivalent_of_equal ha.1 hb.1 same.1,
    equivalent_of_equal ha.2 hb.2 same.2.1, same.2.2⟩

theorem scoreStep_positive (K pTotal pSelected cTotal cSelected : Nat) (hK : 0 < K) :
    ScorePositive (scoreStep K pTotal pSelected cTotal cSelected) :=
  ⟨positive_div (ktStep_positive _ _ _ hK) (ktStep_positive _ _ _ hK),
    positive_div (ktStep_positive _ _ _ hK) (mlStep_positive _ _)⟩

theorem quotient_step (a b c d : Ratio) :
    Equal (div (mul a c) (mul b d)) (mul (div a b) (div c d)) := by
  unfold Equal div mul
  dsimp
  ac_rfl

/-- A shared global child-KT numerator receives only the selected child factor.
Parent-KT and parent-MLE receive their distinct factors, never child MLE. -/
theorem score_step (child parentKT parentMLE : Ratio) (samples : Nat)
    (K pTotal pSelected cTotal cSelected : Nat) :
    Agrees
      ⟨div (mul child (ktStep K cTotal cSelected)) (mul parentKT (ktStep K pTotal pSelected)),
       div (mul child (ktStep K cTotal cSelected)) (mul parentMLE (mlStep pTotal pSelected)),
       samples+1⟩
      (accumulate ⟨div child parentKT, div child parentMLE, samples⟩
        (scoreStep K pTotal pSelected cTotal cSelected)) :=
  ⟨quotient_step _ _ _ _, quotient_step _ _ _ _, rfl⟩

theorem product_ratios_positive (qs : List Ratio) (positive : ∀ q ∈ qs, Positive q) :
    Positive (qs.foldr mul one) := by
  induction qs with
  | nil => exact ⟨Nat.zero_lt_one, Nat.zero_lt_one⟩
  | cons q qs ih =>
      exact positive_mul (positive q (by simp)) (ih (by intro p hp; exact positive p (by simp [hp])))

theorem fromCounts_positive (K : Nat) (hK : 0 < K) (parent : List Nat)
    (children : List (List Nat)) (samples : Nat) :
    ScorePositive (fromCounts K parent children samples) := by
  have hc : Positive ((children.map (kt K)).foldr mul one) := by
    apply product_ratios_positive
    intro q hq
    rcases List.mem_map.mp hq with ⟨xs,_,rfl⟩
    exact kt_positive K hK xs
  exact ⟨positive_div hc (kt_positive K hK parent), positive_div hc (ml_positive parent)⟩

/-- One real parent and one selected child are incremented at the same symbol.
The full finite replay score equals its old value times the online factors.
This is algebra over an admitted refinement; native intake still owns the
child-to-parent association and refusal precedence. -/
theorem fromCounts_step (K : Nat) (hK : 0 < K) (parent : List Nat)
    (children : List (List Nat)) (child label samples : Nat)
    (hc : child < children.length) (hp : label < parent.length)
    (hl : label < children[child].length) :
    Agrees (fromCounts K (parent.modify label (·+1))
      (children.modify child (fun xs => xs.modify label (·+1))) (samples+1))
      (accumulate (fromCounts K parent children samples)
        (scoreStep K parent.sum parent[label] children[child].sum children[child][label])) := by
  have childStep := product_step (kt K) children child hc
    (fun xs => xs.modify label (·+1)) (ktStep K children[child].sum children[child][label])
    (by rw [kt_step K _ label hl]; rfl)
  have parentStep : Equal (kt K (parent.modify label (·+1)))
      (mul (kt K parent) (ktStep K parent.sum parent[label])) := by
    rw [kt_step K parent label hp]; rfl
  have childPositive : Positive ((children.map (kt K)).foldr mul one) := by
    apply product_ratios_positive
    intro q hq
    rcases List.mem_map.mp hq with ⟨xs,_,rfl⟩
    exact kt_positive K hK xs
  have hnew := positive_mul childPositive
    (ktStep_positive K children[child].sum children[child][label] hK)
  constructor
  · exact equal_trans (positive_div hnew (positive_mul (kt_positive K hK parent)
        (ktStep_positive _ _ _ hK))) (equal_div childStep parentStep) (quotient_step _ _ _ _)
  constructor
  · exact equal_trans (positive_div hnew (positive_mul (ml_positive parent)
        (mlStep_positive _ _))) (equal_div childStep (ml_step parent label hp)) (quotient_step _ _ _ _)
  · rfl

theorem agrees_trans {a b c : Score} (positive : ScorePositive b)
    (ab : Agrees a b) (bc : Agrees b c) : Agrees a c :=
  ⟨equal_trans positive.1 ab.1 bc.1,
    equal_trans positive.2 ab.2.1 bc.2.1, ab.2.2.trans bc.2.2⟩

theorem accumulate_positive {s d : Score} (hs : ScorePositive s) (hd : ScorePositive d) :
    ScorePositive (accumulate s d) := ⟨positive_mul hs.1 hd.1, positive_mul hs.2 hd.2⟩

theorem accumulate_agrees {a b : Score} (same : Agrees a b) (delta : Score) :
    Agrees (accumulate a delta) (accumulate b delta) :=
  ⟨equal_mul same.1 rfl, equal_mul same.2.1 rfl, congrArg (· + delta.samples) same.2.2⟩

/-- Attach exact ratios to an existing admitted update. A refusal does not
evaluate a score delta or publish the uncommitted row. -/
def track (update : S → E → Except F S) (delta : S → E → Score)
    (current : S × Score) (event : E) : Except F (S × Score) :=
  (update current.1 event).map (fun next => (next, accumulate current.2 (delta current.1 event)))

/-- Multirow preservation follows from the proved one-row law. The state
carrier and intake are supplied by the adapter, not reconstructed from scores.
`SharedUpdates.refusal_retained` preserves any first error across continuation. -/
theorem tracked_preservation (update : S → E → Except F S) (delta : S → E → Score)
    (closed : S → Score) (positive : ∀ state event, ScorePositive (delta state event))
    (closedPositive : ∀ state, ScorePositive (closed state))
    (step : ∀ state event next, update state event = .ok next →
      Agrees (closed next) (accumulate (closed state) (delta state event)))
    (events : List E) (position : Nat) (initial final : S × Score)
    (initialPositive : ScorePositive initial.2)
    (initialSame : Agrees (closed initial.1) initial.2)
    (accepted : SharedUpdates.runFrom (track update delta) position events initial = .complete final) :
    ScorePositive final.2 ∧ Agrees (closed final.1) final.2 := by
  induction events generalizing position initial with
  | nil => cases accepted; exact ⟨initialPositive, initialSame⟩
  | cons event events ih =>
      simp only [SharedUpdates.runFrom, track] at accepted
      cases hu : update initial.1 event with
      | error reason => simp [hu, Except.map] at accepted
      | ok next =>
          simp only [hu, Except.map] at accepted
          exact ih (position+1) (next, accumulate initial.2 (delta initial.1 event))
            (accumulate_positive initialPositive (positive _ _))
            (agrees_trans (accumulate_positive (closedPositive _) (positive _ _))
              (step _ _ _ hu) (accumulate_agrees initialSame _)) accepted

end OnlineScore
