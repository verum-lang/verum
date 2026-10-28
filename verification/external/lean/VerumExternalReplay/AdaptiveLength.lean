import VerumExternalReplay.FiniteWire

/-! A proper finite-length law learned from completed rows. Counts are a finite
sparse measure; repeated keys add, so this model does not dictate native storage.
The shared candidate/reference bank uses unit prior strength and byte lengths.
No conditioning on well-typed encodings is performed. -/
namespace AdaptiveLength
open ExactMass EnumerableMass FiniteWire
set_option warningAsError true

abbrev Counts := List (Nat × Nat)

def total : Counts → Nat
  | [] => 0
  | (_, c) :: rest => c + total rest

def countAt (n : Nat) : Counts → Nat
  | [] => 0
  | (k, c) :: rest => (if k = n then c else 0) + countAt n rest

def tail (n : Nat) : Counts → Nat
  | [] => 0
  | (k, c) :: rest => (if n ≤ k then c else 0) + tail n rest

def cutoff : Counts → Nat
  | [] => 0
  | (k, _) :: rest => max (k+1) (cutoff rest)

theorem tail_zero (counts : Counts) : tail 0 counts = total counts := by
  induction counts with
  | nil => rfl
  | cons entry rest ih => simp [tail, total, ih]

theorem tail_split (counts : Counts) (n : Nat) :
    countAt n counts + tail (n+1) counts = tail n counts := by
  induction counts with
  | nil => rfl
  | cons entry rest ih =>
    rcases entry with ⟨k,c⟩
    simp only [countAt, tail]
    split <;> split <;> split <;> omega

theorem tail_finite (counts : Counts) (n : Nat) (large : cutoff counts ≤ n) :
    tail n counts = 0 := by
  induction counts with
  | nil => rfl
  | cons entry rest ih =>
    rcases entry with ⟨k,c⟩
    simp only [cutoff, Nat.max_le] at large
    have beyond : ¬ n ≤ k := by omega
    simp [tail, beyond, ih large.2]

/-- Add a unit-strength prior to an empirical count, with one shared denominator. -/
def smooth (samples count : Nat) (prior : Fraction) : Fraction :=
  ⟨count * prior.denominator + prior.numerator,
   (samples+1) * prior.denominator, Nat.mul_pos (by omega) prior.positive⟩

theorem smooth_positive (samples count : Nat) (prior : Fraction)
    (positive : 0 < prior.numerator) : 0 < (smooth samples count prior).numerator := by
  simp only [smooth]
  omega

theorem smooth_add (samples x y : Nat) (a b c : Fraction)
    (conserved : Equal (add a b) c) :
    Equal (add (smooth samples x a) (smooth samples y b))
      (smooth samples (x+y) c) := by
  have scaled := congrArg (fun n => n * (samples+1) * (samples+1)) conserved
  change _ = _ at scaled
  simp only [Equal, add, smooth]
  generalize samples+1 = d at scaled ⊢
  simp only [add, Nat.mul_add,
    Nat.mul_comm, Nat.mul_left_comm, Nat.add_assoc,
    Nat.add_comm, Nat.add_left_comm] at scaled ⊢
  omega

def weight (counts : Counts) (n : Nat) : Fraction :=
  smooth (total counts) (countAt n counts) (half (n+1))

def remaining (counts : Counts) (n : Nat) : Fraction :=
  smooth (total counts) (tail n counts) (half n)

theorem weight_positive (counts : Counts) (n : Nat) : 0 < (weight counts n).numerator :=
  smooth_positive _ _ _ Nat.zero_lt_one

theorem point_tail (counts : Counts) (n : Nat) :
    Equal (add (weight counts n) (remaining counts (n+1))) (remaining counts n) := by
  have h := smooth_add (total counts) (countAt n counts) (tail (n+1) counts)
    (half (n+1)) (half (n+1)) (half n) (dyadic.conserves n)
  simpa only [weight, remaining, tail_split] using h

def ledger (counts : Counts) : Ledger where
  weights := weight counts
  remaining := remaining counts
  initial := by simp [remaining, tail_zero, smooth, half, unit, Equal, one]
  positive := weight_positive counts
  conserves := point_tail counts

theorem normalized_prefix (counts : Counts) (n : Nat) :
    Equal (add (partialSum (weight counts) n) (remaining counts n)) one :=
  prefix_exact (ledger counts) n

theorem tail_vanishes (counts : Counts) (epsilon : Fraction) (positive : 0 < epsilon.numerator) :
    ∃ bound, ∀ n, bound ≤ n → Less (remaining counts n) epsilon := by
  refine ⟨max (cutoff counts) epsilon.denominator, ?_⟩
  intro n large
  have finite := tail_finite counts n (Nat.le_trans (Nat.le_max_left _ _) large)
  have small := geometric_tail epsilon positive n (Nat.le_trans (Nat.le_max_right _ _) large)
  simp only [remaining, smooth, finite, Nat.zero_mul, Nat.zero_add,
    half, unit, Less, Nat.one_mul] at small ⊢
  exact Nat.lt_of_lt_of_le small (Nat.mul_le_mul_left _
    (Nat.le_mul_of_pos_left _ (by omega)))

def law (counts : Counts) : LengthLaw where
  weight := weight counts
  remaining := remaining counts
  conserved := normalized_prefix counts
  vanishes := tail_vanishes counts

/-- A completed row contributes exactly one observation, even for zero bytes. -/
def commit (counts : Counts) (bytes : Nat) : Counts := (bytes,1) :: counts

theorem commit_total (counts : Counts) (bytes : Nat) :
    total (commit counts bytes) = total counts + 1 := by simp [commit, total, Nat.add_comm]

theorem commit_at (counts : Counts) (bytes n : Nat) :
    countAt n (commit counts bytes) = countAt n counts + if bytes = n then 1 else 0 := by
  simp [commit, countAt, Nat.add_comm]

theorem commit_tail (counts : Counts) (bytes n : Nat) :
    tail n (commit counts bytes) = tail n counts + if n ≤ bytes then 1 else 0 := by
  simp [commit, tail, Nat.add_comm]

theorem shared_ratio (counts : Counts) (bytes : Nat) (q p : Fraction)
    (positive : 0 < p.numerator) :
    Equal (divide (mul (weight counts bytes) q) (mul (weight counts bytes) p)
      (Nat.mul_pos (weight_positive counts bytes) positive)) (divide q p positive) :=
  divide_common_scale q p _ positive (weight_positive counts bytes)

end AdaptiveLength
