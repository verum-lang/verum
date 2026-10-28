import VerumExternalReplay.AdaptiveLength
import VerumExternalReplay.ResidualPrediction

/-! Shared learned lengths composed with the existing retained KT banks.
The same causal kernel and mask apply at every final length: this is the
prefix-consistency premise, not a consequence of per-length normalization. -/
namespace AdaptiveResidual
open ExactMass CausalCoding AdaptiveLength ResidualPrediction
set_option warningAsError true

def lengths : List Row → Counts
  | [] => []
  | r :: rows => commit (lengths rows) r.target.length

theorem lengths_total (rows : List Row) : total (lengths rows) = rows.length := by
  induction rows with
  | nil => rfl
  | cons r rows ih => simp only [lengths, commit_total, List.length_cons, ih]

/-- Compare the complete finite measures, independently of native map order. -/
def histogramAgrees (rows : List Row) (provided : Counts) : Bool :=
  let expected := lengths rows
  total provided == rows.length &&
  expected.all (fun (n, _) => countAt n expected == countAt n provided) &&
  provided.all (fun (n, _) => countAt n expected == countAt n provided)

def forecastAgrees (s : ResidualPrediction.State)
    (residual : List ((List Nat × Nat) × List Nat)) (reference : List (Nat × List Nat)) : Bool :=
  s.residual.map (fun (key, count) => (key, count.val.counts)) == residual &&
  s.reference.map (fun (key, count) => (key, count.val.counts)) == reference

def lengthMass (counts : Counts) (bytes : Nat) (isPrefix : Bool) : Fraction :=
  if isPrefix then remaining counts bytes else weight counts bytes

def candidate (counts : Counts) (s : ResidualPrediction.State)
    (context mask target : List Nat) (isPrefix : Bool) : Fraction :=
  mul (lengthMass counts target.length isPrefix)
    (PositionPrediction.frozen (fun p => (context, p)) s.residual 0
      (CausalCoding.residual (wireBits mask) (wireBits target)))

def reference (counts : Counts) (s : ResidualPrediction.State)
    (target : List Nat) (isPrefix : Bool) : Fraction :=
  mul (lengthMass counts target.length isPrefix)
    (PositionPrediction.frozen id s.reference 0 (wireBits target))

/-- A prefix partitions into stopping now and all 256 possible next bytes.
For a byte prefix of length n, `past` is its 8n-bit encoding. The theorem is
universal in the retained causal state and the fixed residual mask. -/
theorem prefix_partition (counts : Counts) (n : Nat) (kernel : Kernel State)
    (state : State) (prediction past : List Bool) :
    Equal (add
      (mul (weight counts n) (likelihood kernel state (residual prediction past)))
      (sum ((words 8).map (fun nextByte => mul (remaining counts (n+1))
        (likelihood kernel state (residual prediction (past ++ nextByte)))))))
      (mul (remaining counts n) (likelihood kernel state (residual prediction past))) := by
  apply eq_trans (add_congr_left _ (sum_scale _ _ _))
  apply eq_trans (add_congr_left _ (mul_congr_left _ (prefix_marginal kernel state prediction past 8)))
  apply eq_trans (add_congr_right _ (mul_comm _ _))
  apply eq_trans (add_congr_left _ (mul_comm _ _))
  apply eq_trans (eq_symm (mul_add _ _ _))
  apply eq_trans (mul_congr_left _ (point_tail counts n))
  exact mul_comm _ _

end AdaptiveResidual
