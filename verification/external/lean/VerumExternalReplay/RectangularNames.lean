import Lean

/-! Original max-shell names for finite rectangular products. Arithmetic is over
unbounded naturals. Service order, retained runtime ownership, allocation and
work costs are separate obligations; an emission count is not a name. -/
namespace RectangularNames
set_option warningAsError true

structure Triple where
  i : Nat
  u : Nat
  r : Nat
  deriving DecidableEq, Repr

def Valid (n p : Triple) : Prop := p.i < n.i ∧ p.u < n.u ∧ p.r < n.r
instance (n p : Triple) : Decidable (Valid n p) :=
  inferInstanceAs (Decidable (p.i < n.i ∧ p.u < n.u ∧ p.r < n.r))
def shell (p : Triple) : Nat := max p.i (max p.u p.r)
def volume (n : Triple) (q : Nat) : Nat := min q n.i * min q n.u * min q n.r
def readCount (n : Triple) (q : Nat) : Nat :=
  if q < n.r then min q n.i * min q n.u else 0
def updateCount (n : Triple) (q : Nat) : Nat :=
  if q < n.u then min q n.i * min (q + 1) n.r else 0
def initialCount (n : Triple) (q : Nat) : Nat :=
  if q < n.i then min (q + 1) n.u * min (q + 1) n.r else 0
def offset (n p : Triple) (q : Nat) : Nat :=
  if p.i = q then readCount n q + updateCount n q + p.u * min (q + 1) n.r + p.r
  else if p.u = q then readCount n q + p.i * min (q + 1) n.r + p.r
  else p.i * min q n.u + p.u
def rank (n p : Triple) : Nat := volume n (shell p) + offset n p (shell p)

def decodeOffset (n : Triple) (q k : Nat) : Option Triple :=
  if k < readCount n q then
    some ⟨k / min q n.u, k % min q n.u, q⟩
  else if k < readCount n q + updateCount n q then
    let j := k - readCount n q
    some ⟨j / min (q + 1) n.r, q, j % min (q + 1) n.r⟩
  else if k < readCount n q + updateCount n q + initialCount n q then
    let j := k - (readCount n q + updateCount n q)
    some ⟨q, j / min (q + 1) n.r, j % min (q + 1) n.r⟩
  else none

private theorem rectangle_bound (i j height width : Nat) (hi : i < height) (hj : j < width) :
    i * width + j < height * width := by
  have row := Nat.mul_le_mul_right width (Nat.succ_le_of_lt hi)
  simp only [Nat.succ_mul] at row
  omega

private theorem rectangle_inverse (i j width : Nat) (hj : j < width) :
    (i * width + j) / width = i ∧ (i * width + j) % width = j := by
  have positive : 0 < width := by omega
  constructor
  · rw [Nat.add_comm, Nat.add_mul_div_right _ _ positive, Nat.div_eq_of_lt hj]
    omega
  · simp [Nat.add_mod, Nat.mod_eq_of_lt hj]

theorem shell_volume (n : Triple) (q : Nat) :
    volume n (q + 1) = volume n q + readCount n q + updateCount n q + initialCount n q := by
  rcases n with ⟨a, b, c⟩
  unfold volume readCount updateCount initialCount
  dsimp
  by_cases ha : q < a <;> by_cases hb : q < b <;> by_cases hc : q < c <;>
    simp only [ha, hb, hc, ↓reduceIte] <;>
    (repeat first | rw [Nat.min_eq_left (by omega)] | rw [Nat.min_eq_right (by omega)]) <;>
    (try simp only [Nat.mul_add, Nat.add_mul, Nat.mul_one, Nat.one_mul]) <;> omega

theorem volume_monotone (n : Triple) (a b : Nat) (h : a ≤ b) : volume n a ≤ volume n b := by
  unfold volume
  exact Nat.mul_le_mul (Nat.mul_le_mul (by omega) (by omega)) (by omega)

theorem offset_inverse (n p : Triple) (valid : Valid n p) :
    offset n p (shell p) < readCount n (shell p) + updateCount n (shell p) + initialCount n (shell p) ∧
    decodeOffset n (shell p) (offset n p (shell p)) = some p := by
  rcases valid with ⟨hi, hu, hr⟩
  let q := shell p
  have qi : p.i ≤ q := Nat.le_max_left _ _
  have qu : p.u ≤ q := Nat.le_trans (Nat.le_max_left _ _) (Nat.le_max_right _ _)
  have qr : p.r ≤ q := Nat.le_trans (Nat.le_max_right _ _) (Nat.le_max_right _ _)
  have maximum : p.i = q ∨ p.u = q ∨ p.r = q := by dsimp [q, shell]; omega
  change offset n p q < readCount n q + updateCount n q + initialCount n q ∧
    decodeOffset n q (offset n p q) = some p
  by_cases ei : p.i = q
  · have ub : p.u < min (q + 1) n.u := by omega
    have rb : p.r < min (q + 1) n.r := by omega
    have count : initialCount n q = min (q + 1) n.u * min (q + 1) n.r := by
      simp only [initialCount, ite_eq_left (show q < n.i by omega)]
    have bound := rectangle_bound p.u p.r _ _ ub rb
    have inverse := rectangle_inverse p.u p.r _ rb
    have off : offset n p q = readCount n q + updateCount n q + (p.u * min (q + 1) n.r + p.r) := by
      simp only [offset, ite_eq_left ei]; omega
    constructor
    · rw [off, count]; omega
    · rw [off]
      simp only [decodeOffset, ite_eq_right (by omega : ¬ readCount n q + updateCount n q +
        (p.u * min (q + 1) n.r + p.r) < readCount n q),
        ite_eq_right (by omega : ¬ readCount n q + updateCount n q +
        (p.u * min (q + 1) n.r + p.r) < readCount n q + updateCount n q),
        ite_eq_left (by omega : readCount n q + updateCount n q +
        (p.u * min (q + 1) n.r + p.r) < readCount n q + updateCount n q + initialCount n q),
        Nat.add_sub_cancel_left, inverse.1, inverse.2]
      cases p; simp_all
  · by_cases eu : p.u = q
    · have ib : p.i < min q n.i := by omega
      have rb : p.r < min (q + 1) n.r := by omega
      have count : updateCount n q = min q n.i * min (q + 1) n.r := by
        simp only [updateCount, ite_eq_left (show q < n.u by omega)]
      have bound := rectangle_bound p.i p.r _ _ ib rb
      have inverse := rectangle_inverse p.i p.r _ rb
      have off : offset n p q = readCount n q + (p.i * min (q + 1) n.r + p.r) := by
        simp only [offset, ite_eq_right ei, ite_eq_left eu]; omega
      constructor
      · rw [off]; omega
      · rw [off]
        simp only [decodeOffset, ite_eq_right (by omega : ¬ readCount n q +
          (p.i * min (q + 1) n.r + p.r) < readCount n q),
          ite_eq_left (by omega : readCount n q + (p.i * min (q + 1) n.r + p.r) <
          readCount n q + updateCount n q), Nat.add_sub_cancel_left, inverse.1, inverse.2]
        cases p; simp_all
    · have er : p.r = q := by omega
      have ib : p.i < min q n.i := by omega
      have ub : p.u < min q n.u := by omega
      have count : readCount n q = min q n.i * min q n.u := by
        simp only [readCount, ite_eq_left (show q < n.r by omega)]
      have bound := rectangle_bound p.i p.u _ _ ib ub
      have inverse := rectangle_inverse p.i p.u _ ub
      have off : offset n p q = p.i * min q n.u + p.u := by
        simp only [offset, ite_eq_right ei, ite_eq_right eu]
      constructor
      · rw [off]; omega
      · rw [off]
        simp only [decodeOffset, ite_eq_left (by omega : p.i * min q n.u + p.u < readCount n q),
          inverse.1, inverse.2]
        cases p; simp_all

theorem rank_interval (n p : Triple) (valid : Valid n p) :
    volume n (shell p) ≤ rank n p ∧ rank n p < volume n (shell p + 1) := by
  have bound := (offset_inverse n p valid).1
  unfold rank
  constructor
  · exact Nat.le_add_right _ _
  · rw [shell_volume]
    simpa only [Nat.add_assoc] using Nat.add_lt_add_left bound (volume n (shell p))

/-- The original decoder's binary search over monotonically growing boxes.
    Its natural-valued model cannot overflow; a machine-word implementation
    additionally needs representable sizes, products and successor arithmetic. -/
def locate (n : Triple) (index lo hi : Nat) : Nat :=
  if lo < hi then
    let mid := lo + (hi - lo) / 2
    if index < volume n (mid + 1) then locate n index lo mid
    else locate n index (mid + 1) hi
  else lo
termination_by hi - lo
decreasing_by all_goals omega

def decode (n : Triple) (index : Nat) : Option Triple :=
  let q := locate n index 0 (shell n)
  decodeOffset n q (index - volume n q)

theorem interval_unique (n : Triple) (index a b : Nat)
    (ha : volume n a ≤ index ∧ index < volume n (a + 1))
    (hb : volume n b ≤ index ∧ index < volume n (b + 1)) : a = b := by
  by_cases ab : a < b
  · have := volume_monotone n (a + 1) b (by omega); omega
  · by_cases ba : b < a
    · have := volume_monotone n (b + 1) a (by omega); omega
    · omega

theorem locate_interval (n : Triple) (index q lo hi : Nat)
    (interval : volume n q ≤ index ∧ index < volume n (q + 1))
    (lower : lo ≤ q) (upper : q ≤ hi) : locate n index lo hi = q := by
  rw [locate]
  split
  · rename_i growing
    let mid := lo + (hi - lo) / 2
    change (if index < volume n (mid + 1) then locate n index lo mid
      else locate n index (mid + 1) hi) = q
    split
    · rename_i left
      have bound : q ≤ mid := by
        by_cases inside : q ≤ mid
        · exact inside
        · have := volume_monotone n (mid + 1) q (by omega)
          omega
      exact locate_interval n index q lo mid interval lower bound
    · rename_i right
      have bound : mid + 1 ≤ q := by
        by_cases inside : mid + 1 ≤ q
        · exact inside
        · have := volume_monotone n (q + 1) (mid + 1) (by omega)
          omega
      exact locate_interval n index q (mid + 1) hi interval bound upper
  · omega
termination_by hi - lo
decreasing_by all_goals omega

theorem decode_rank (n p : Triple) (valid : Valid n p) : decode n (rank n p) = some p := by
  have enough : shell p ≤ shell n := by
    rcases valid with ⟨hi, hu, hr⟩
    dsimp [shell]
    omega
  unfold decode
  rw [locate_interval n (rank n p) (shell p) 0 (shell n)
    (rank_interval n p valid) (Nat.zero_le _) enough]
  simp only [rank, Nat.add_sub_cancel_left]
  exact (offset_inverse n p valid).2

theorem rank_injective (n p t : Triple) (hp : Valid n p) (ht : Valid n t)
    (equal : rank n p = rank n t) : p = t := by
  have h := (decode_rank n p hp).symm.trans ((congrArg (decode n) equal).trans (decode_rank n t ht))
  exact Option.some.inj h

private theorem rectangle_decode (k height width : Nat) (bound : k < height * width) :
    k / width < height ∧ k % width < width ∧ k / width * width + k % width = k := by
  have positive : 0 < width := by
    by_cases zero : width = 0
    · simp only [zero, Nat.mul_zero] at bound; omega
    · omega
  exact ⟨(Nat.div_lt_iff_lt_mul positive).mpr bound, Nat.mod_lt _ positive,
    Nat.div_add_mod' k width⟩

private theorem offset_witness (n p : Triple) (q k : Nat) (valid : Valid n p)
    (sq : shell p = q) (off : offset n p q = k) :
    ∃ p, decodeOffset n q k = some p ∧ Valid n p ∧ rank n p = volume n q + k := by
  have inverse := (offset_inverse n p valid).2
  rw [sq, off] at inverse
  exact ⟨p, inverse, valid, by simp only [rank, sq, off]⟩

theorem decode_offset_sound (n : Triple) (q k : Nat)
    (bound : k < readCount n q + updateCount n q + initialCount n q) :
    ∃ p, decodeOffset n q k = some p ∧ Valid n p ∧ rank n p = volume n q + k := by
  by_cases read : k < readCount n q
  · have qr : q < n.r := by
      by_cases yes : q < n.r
      · exact yes
      · simp only [readCount, ite_eq_right yes] at read; omega
    have localBound : k < min q n.i * min q n.u := by
      simpa only [readCount, ite_eq_left qr] using read
    obtain ⟨ib, ub, restored⟩ := rectangle_decode k _ _ localBound
    let p : Triple := ⟨k / min q n.u, k % min q n.u, q⟩
    have valid : Valid n p := by dsimp [Valid, p]; exact ⟨by omega, by omega, by omega⟩
    have sq : shell p = q := by dsimp [shell, p]; omega
    apply offset_witness n p q k valid sq
    have ei : p.i ≠ q := by dsimp [p]; omega
    have eu : p.u ≠ q := by dsimp [p]; omega
    simpa only [offset, ite_eq_right ei, ite_eq_right eu] using restored
  · by_cases update : k < readCount n q + updateCount n q
    · have qu : q < n.u := by
        by_cases yes : q < n.u
        · exact yes
        · simp only [updateCount, ite_eq_right yes] at update; omega
      let j := k - readCount n q
      have localBound : j < min q n.i * min (q + 1) n.r := by
        have count : updateCount n q = min q n.i * min (q + 1) n.r :=
          ite_eq_left qu
        dsimp [j]; omega
      obtain ⟨ib, rb, restored⟩ := rectangle_decode j _ _ localBound
      let p : Triple := ⟨j / min (q + 1) n.r, q, j % min (q + 1) n.r⟩
      have valid : Valid n p := by dsimp [Valid, p]; exact ⟨by omega, by omega, by omega⟩
      have sq : shell p = q := by dsimp [shell, p]; omega
      apply offset_witness n p q k valid sq
      have ei : p.i ≠ q := by dsimp [p]; omega
      simp only [offset, ite_eq_right ei, show p.u = q from rfl, ↓reduceIte]
      change readCount n q + j / min (q + 1) n.r * min (q + 1) n.r + j % min (q + 1) n.r = k
      dsimp [j] at *; omega
    · have qi : q < n.i := by
        by_cases yes : q < n.i
        · exact yes
        · simp only [initialCount, ite_eq_right yes] at bound; omega
      let j := k - (readCount n q + updateCount n q)
      have localBound : j < min (q + 1) n.u * min (q + 1) n.r := by
        have count : initialCount n q = min (q + 1) n.u * min (q + 1) n.r :=
          ite_eq_left qi
        dsimp [j]; omega
      obtain ⟨ub, rb, restored⟩ := rectangle_decode j _ _ localBound
      let p : Triple := ⟨q, j / min (q + 1) n.r, j % min (q + 1) n.r⟩
      have valid : Valid n p := by dsimp [Valid, p]; exact ⟨by omega, by omega, by omega⟩
      have sq : shell p = q := by dsimp [shell, p]; omega
      apply offset_witness n p q k valid sq
      simp only [offset, show p.i = q from rfl, ↓reduceIte]
      change readCount n q + updateCount n q + j / min (q + 1) n.r * min (q + 1) n.r + j % min (q + 1) n.r = k
      dsimp [j] at *; omega

theorem locate_bounds (n : Triple) (index lo hi : Nat) (ordered : lo ≤ hi)
    (lower : volume n lo ≤ index) (upper : index < volume n (hi + 1)) :
    volume n (locate n index lo hi) ≤ index ∧
      index < volume n (locate n index lo hi + 1) := by
  rw [locate]
  split
  · rename_i growing
    let mid := lo + (hi - lo) / 2
    change volume n (if index < volume n (mid + 1) then locate n index lo mid
      else locate n index (mid + 1) hi) ≤ index ∧ index < volume n
      ((if index < volume n (mid + 1) then locate n index lo mid else locate n index (mid + 1) hi) + 1)
    by_cases left : index < volume n (mid + 1)
    · simp only [ite_eq_left left]
      exact locate_bounds n index lo mid (by dsimp [mid]; omega) lower left
    · simp only [ite_eq_right left]
      exact locate_bounds n index (mid + 1) hi (by dsimp [mid]; omega) (by omega) upper
  · have same : lo = hi := by omega
    simpa only [same] using And.intro lower upper
termination_by hi - lo
decreasing_by all_goals omega

theorem full_volume (n : Triple) : volume n (shell n) = n.i * n.u * n.r := by
  unfold volume shell
  rw [Nat.min_eq_right (by omega), Nat.min_eq_right (by omega), Nat.min_eq_right (by omega)]

theorem decode_in_range (n : Triple) (index : Nat) (bound : index < n.i * n.u * n.r) :
    ∃ p, decode n index = some p ∧ Valid n p ∧ rank n p = index := by
  have upper : index < volume n (shell n + 1) := by
    have mono := volume_monotone n (shell n) (shell n + 1) (by omega)
    rw [full_volume] at mono
    omega
  have interval := locate_bounds n index 0 (shell n) (Nat.zero_le _)
    (by simp [volume]) upper
  let q := locate n index 0 (shell n)
  have localBound : index - volume n q < readCount n q + updateCount n q + initialCount n q := by
    change volume n q ≤ index ∧ index < volume n (q + 1) at interval
    rw [shell_volume] at interval
    omega
  obtain ⟨p, decoded, valid, named⟩ := decode_offset_sound n q (index - volume n q) localBound
  exact ⟨p, decoded, valid, by have lower : volume n q ≤ index := interval.1; omega⟩

theorem rank_bound (n p : Triple) (valid : Valid n p) : rank n p < n.i * n.u * n.r := by
  have enough : shell p + 1 ≤ shell n := by
    rcases valid with ⟨hi, hu, hr⟩
    dsimp [shell]; omega
  have mono := volume_monotone n (shell p + 1) (shell n) enough
  rw [full_volume] at mono
  have bound := (rank_interval n p valid).2
  omega

theorem locate_above (n : Triple) (index lo hi : Nat) (ordered : lo ≤ hi)
    (outside : volume n hi ≤ index) : locate n index lo hi = hi := by
  rw [locate]
  split
  · rename_i growing
    let mid := lo + (hi - lo) / 2
    have bound : mid + 1 ≤ hi := by dsimp [mid]; omega
    have mono := volume_monotone n (mid + 1) hi bound
    change (if index < volume n (mid + 1) then locate n index lo mid
      else locate n index (mid + 1) hi) = hi
    rw [ite_eq_right (by omega : ¬ index < volume n (mid + 1))]
    exact locate_above n index (mid + 1) hi bound outside
  · omega
termination_by hi - lo
decreasing_by omega

theorem decode_out_of_range (n : Triple) (index : Nat) (outside : n.i * n.u * n.r ≤ index) :
    decode n index = none := by
  have high : volume n (shell n) ≤ index := by simpa only [full_volume] using outside
  unfold decode
  rw [locate_above n index 0 (shell n) (Nat.zero_le _) high]
  have hi : ¬ shell n < n.i := by dsimp [shell]; omega
  have hu : ¬ shell n < n.u := by dsimp [shell]; omega
  have hr : ¬ shell n < n.r := by dsimp [shell]; omega
  simp only [decodeOffset, readCount, updateCount, initialCount, ite_eq_right hi,
    ite_eq_right hu, ite_eq_right hr, Nat.add_zero, Nat.not_lt_zero, ↓reduceIte]

theorem empty_support (n p : Triple) (empty : n.i = 0 ∨ n.u = 0 ∨ n.r = 0) : ¬ Valid n p := by
  unfold Valid
  omega

end RectangularNames
