import VerumExternalReplay.RowRatioFold

/-! Positive arithmetic fixtures use independent Python fractions.Fraction results.
False claims must reach Lean's semantic false-proposition diagnostic, not a tool failure.
These controls supplement the universal proofs; they do not bound their domain. -/
namespace RowRatioFoldControls
open ExactMass RowRatioFold
set_option warningAsError true

def raw (n d : Nat) : Fraction := ⟨n, d+1, Nat.zero_lt_succ d⟩
def positive (n d : Nat) : Positive :=
  ⟨raw (n+1) d, Nat.zero_lt_succ n⟩
def L := positive 2 1
-- 2/3 divided by 5/7, followed by 11/13 divided by 17/19.
def first : Factor := ⟨positive 1 2, positive 4 6⟩
def second : Factor := ⟨positive 10 12, positive 16 18⟩
def pairs := [first, second]
def publish (q b : Fraction) := mul q b
def opened := beginRow L (raw 5 1) (raw 7 10)
def accumulated := advanceMany opened pairs

example : Equal (fold (initial (positive 2 1)) []) (raw 2 2) := by decide_cbv

example : Equal (fold (initial (positive 2 1)) [⟨(positive 1 2), (positive 4 6)⟩]) (raw 28 44) := by decide_cbv

example : Equal (fold (initial (positive 2 1)) [⟨(positive 1 2), (positive 4 6)⟩, ⟨(positive 10 12), (positive 16 18)⟩]) (raw 5852 9944) := by decide_cbv

example : Equal (fold (initial (positive 6 10)) [⟨(positive 12 16), (positive 18 22)⟩, ⟨(positive 18 22), (positive 12 16)⟩]) (raw 11 6) := by decide_cbv

example : Equal (fold (initial (positive 340282366920938463463374607431768211456 2)) [⟨(positive 115792089237316195423570985008687907853269984665640564039457584007913129639936 6), (positive 340282366920938463463374607431768211458 10)⟩, ⟨(positive 6 115792089237316195423570985008687907853269984665640564039457584007913129639936), (positive 10 340282366920938463463374607431768211458)⟩]) (raw 3 340282366920938463463374607431768211456) := by decide_cbv


-- Equal fractions with different spellings cancel exactly.
example : Equal (step (raw 5 1) ⟨⟨raw 6 8, by decide⟩, positive 1 2⟩) (raw 5 1) :=
  neutral_step _ _ (by decide)
example : Equal accumulated.q (raw 5 1) := by decide_cbv
example : Equal (close publish accumulated).q (raw 2926 1988) := by decide_cbv
example : (close publish accumulated).row = none := by decide_cbv
example : (close publish (close publish accumulated)).q = (close publish accumulated).q := by decide_cbv
example : (close publish accumulated).absolute = raw 7 10 := by decide_cbv


/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : Equal (fold (initial L) []) one := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : Equal (step (initial L) first) (raw 5 6) := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : Equal (fold (initial L) pairs) (fold (initial L) [second]) := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : Equal (fold (initial L) pairs) (fold (initial L) [first]) := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : Equal accumulated.q (fold (initial L) pairs) := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : Equal (advance opened first).q (publish opened.q (step (initial L) first)) := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : Equal (close publish (close publish accumulated)).q (publish (close publish accumulated).q (fold (initial L) pairs)) := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : Equal (close publish accumulated).absolute (fold (initial L) pairs) := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : (initial ⟨zero, by decide_cbv⟩).numerator = 1 := by rfl

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : (step one ⟨positive 0 0, ⟨zero, by decide_cbv⟩⟩).numerator = 1 := by rfl

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : (step one ⟨⟨zero, by decide_cbv⟩, positive 0 0⟩).numerator = 0 := by rfl

end RowRatioFoldControls
