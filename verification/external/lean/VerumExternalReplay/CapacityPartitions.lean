import VerumExternalReplay.CapacityFactors

/-! Exact split/merge inequalities for fixed-alphabet KT cells. Coordinates in
different rows denote the same outcome; missing trailing coordinates are zero.
The inequalities themselves do not require the number of coordinates to be K.
Using a K-outcome capacity bound additionally requires that dimension premise.
-/
namespace RefinementFeasibility
set_option warningAsError true

def factorProduct {α : Type} (f : α → Rat) : List α → Rat
  | [] => 1
  | x :: xs => f x * factorProduct f xs

def pure (K n : Nat) : Rat := rising (1/2) n / rising ((K : Rat)/2) n

def cell (K : Nat) (counts : List Nat) : Rat :=
  factorProduct (rising (1/2)) counts / rising ((K : Rat)/2) counts.sum

private theorem mul_left_comm (a b c : Rat) : a*(b*c) = b*(a*c) := by
  rw [← Rat.mul_assoc, ← Rat.mul_assoc, Rat.mul_comm a b]

private theorem mul_mono {a b c d : Rat} (ha : 0 ≤ a) (hd : 0 ≤ d)
    (hab : a ≤ b) (hcd : c ≤ d) : a*c ≤ b*d :=
  Rat.le_trans (Rat.mul_le_mul_of_nonneg_left hcd ha)
    (Rat.mul_le_mul_of_nonneg_right hab hd)

private theorem div_le_div_of_cross {a b c d : Rat} (hb : 0 < b) (hd : 0 < d)
    (h : a*d ≤ c*b) : a/b ≤ c/d := by
  apply Rat.le_of_mul_le_mul_right (c := b*d) ?_ (Rat.mul_pos hb hd)
  have hl : a/b*(b*d) = a*d := by
    rw [← Rat.mul_assoc, Rat.div_mul_cancel (Rat.ne_of_gt hb)]
  have hr : c/d*(b*d) = c*b := by
    rw [Rat.mul_comm b d, ← Rat.mul_assoc, Rat.div_mul_cancel (Rat.ne_of_gt hd)]
  simpa only [hl, hr] using h

private theorem quotient_mul (a b c d : Rat) (hb : b ≠ 0) (hd : d ≠ 0) :
    a/b*(c/d) = (a*c)/(b*d) := by grind

theorem rising_supermultiplicative {a : Rat} (ha : 0 < a) (u v : Nat) :
    rising a u * rising a v ≤ rising a (u+v) := by
  induction v with
  | zero => simp only [rising_zero, Nat.add_zero, Rat.mul_one]; exact Rat.le_refl
  | succ v ih =>
    have hu := Rat.natCast_nonneg (a := u)
    have hv := Rat.natCast_nonneg (a := v)
    have h := mul_mono
      (Rat.le_of_lt (Rat.mul_pos (rising_pos ha u) (rising_pos ha v)))
      (show 0 ≤ a + ((u+v : Nat) : Rat) by grind)
      ih (show a+(v : Rat) ≤ a+((u+v : Nat) : Rat) by grind)
    simpa only [← Nat.add_assoc, rising_succ, Rat.mul_assoc] using h

theorem factorProduct_pos {α : Type} (f : α → Rat) (xs : List α)
    (h : ∀ x ∈ xs, 0 < f x) : 0 < factorProduct f xs := by
  induction xs with
  | nil => change (0 : Rat) < 1; decide
  | cons x xs ih =>
    exact Rat.mul_pos (h x (by simp)) (ih (by intro y hy; exact h y (by simp [hy])))

@[simp] theorem pure_zero (K : Nat) : pure K 0 = 1 := by
  simp only [pure, rising_zero]
  grind

theorem pure_pos {K : Nat} (hK : 0 < K) (n : Nat) : 0 < pure K n := by
  have hk : 0 < (K : Rat) := Rat.natCast_pos.mpr hK
  have ha : (0 : Rat) < 1/2 := by grind
  have hb : 0 < (K : Rat)/2 := by grind
  exact Rat.mul_pos (rising_pos ha n) (Rat.inv_pos.mpr (rising_pos hb n))

private def pureStep (K n : Nat) : Rat :=
  (1/2+(n : Rat))/((K : Rat)/2+(n : Rat))

private theorem pureStep_pos {K : Nat} (hK : 0 < K) (n : Nat) : 0 < pureStep K n := by
  have hk : 0 < (K : Rat) := Rat.natCast_pos.mpr hK
  have hn := Rat.natCast_nonneg (a := n)
  exact Rat.mul_pos (by grind) (Rat.inv_pos.mpr (by grind))

private theorem pure_succ {K : Nat} (hK : 0 < K) (n : Nat) :
    pure K (n+1) = pure K n * pureStep K n := by
  have hk : 0 < (K : Rat) := Rat.natCast_pos.mpr hK
  have hn := Rat.natCast_nonneg (a := n)
  have hb : 0 < (K : Rat)/2 := by grind
  have hd : 0 < (K : Rat)/2+(n : Rat) := by grind
  unfold pure pureStep
  rw [rising_succ, rising_succ,
    quotient_mul _ _ _ _ (Rat.ne_of_gt (rising_pos hb n)) (Rat.ne_of_gt hd)]

private theorem pureStep_monotone {K u v : Nat} (hK : 0 < K) (h : u ≤ v) :
    pureStep K u ≤ pureStep K v := by
  have hk : ((1 : Nat) : Rat) ≤ (K : Rat) := Rat.natCast_le_natCast.mpr (by omega)
  have hu := Rat.natCast_nonneg (a := u)
  have hv := Rat.natCast_nonneg (a := v)
  have huv : (u : Rat) ≤ (v : Rat) := Rat.natCast_le_natCast.mpr h
  unfold pureStep
  apply div_le_div_of_cross (by grind) (by grind)
  have hp := Rat.mul_nonneg
    (show 0 ≤ (K : Rat)/2-1/2 by grind)
    (show 0 ≤ (v : Rat)-(u : Rat) by grind)
  grind

theorem pure_merge {K : Nat} (hK : 0 < K) (u v : Nat) :
    pure K u * pure K v ≤ pure K (u+v) := by
  induction v with
  | zero => simp only [pure_zero, Nat.add_zero, Rat.mul_one]; exact Rat.le_refl
  | succ v ih =>
    have h := mul_mono
      (Rat.le_of_lt (Rat.mul_pos (pure_pos hK u) (pure_pos hK v)))
      (Rat.le_of_lt (pureStep_pos hK (u+v))) ih
      (pureStep_monotone hK (show v ≤ u+v by omega))
    simpa only [← Nat.add_assoc, pure_succ hK, Rat.mul_assoc] using h

theorem cell_pos {K : Nat} (hK : 0 < K) (counts : List Nat) : 0 < cell K counts := by
  have hk : 0 < (K : Rat) := Rat.natCast_pos.mpr hK
  have hn := factorProduct_pos (rising (1/2)) counts
    (by intro n _; exact rising_pos (by grind) n)
  exact Rat.mul_pos hn (Rat.inv_pos.mpr (rising_pos (by grind) counts.sum))

theorem cell_split {K : Nat} (hK : 0 < K) (counts : List Nat) :
    cell K counts ≤ factorProduct (pure K) counts := by
  have hk : 0 < (K : Rat) := Rat.natCast_pos.mpr hK
  have hb : 0 < (K : Rat)/2 := by grind
  induction counts with
  | nil => simp only [cell, factorProduct, List.sum_nil, rising_zero]; grind
  | cons n ns ih =>
    have hnum := Rat.mul_pos (rising_pos (show (0 : Rat) < 1/2 by grind) n)
      (factorProduct_pos (rising (1/2)) ns (by intro j _; exact rising_pos (by grind) j))
    have hden := rising_supermultiplicative hb n ns.sum
    have hsplit : cell K (n :: ns) ≤ pure K n * cell K ns := by
      unfold cell pure
      rw [quotient_mul _ _ _ _ (Rat.ne_of_gt (rising_pos hb n))
        (Rat.ne_of_gt (rising_pos hb ns.sum))]
      simp only [factorProduct, List.sum_cons]
      apply div_le_div_of_cross (rising_pos hb _) (Rat.mul_pos (rising_pos hb _) (rising_pos hb _))
      exact Rat.mul_le_mul_of_nonneg_left hden (Rat.le_of_lt hnum)
    exact Rat.le_trans hsplit (Rat.mul_le_mul_of_nonneg_left ih (Rat.le_of_lt (pure_pos hK n)))

/-- Coordinate-wise counts; a missing trailing coordinate contributes zero. -/
def mergeCounts : List Nat → List Nat → List Nat
  | [], ys => ys
  | x :: xs, [] => x :: xs
  | x :: xs, y :: ys => (x+y) :: mergeCounts xs ys

theorem mergeCounts_sum (xs ys : List Nat) :
    (mergeCounts xs ys).sum = xs.sum + ys.sum := by
  induction xs generalizing ys with
  | nil => simp [mergeCounts]
  | cons x xs ih =>
    cases ys with
    | nil => simp [mergeCounts]
    | cons y ys =>
      simp only [mergeCounts, List.sum_cons, ih]
      omega

theorem mergeCounts_length (xs ys : List Nat) (h : xs.length = ys.length) :
    (mergeCounts xs ys).length = xs.length := by
  induction xs generalizing ys with
  | nil => simpa only [mergeCounts, List.length_nil] using h.symm
  | cons x xs ih =>
    cases ys with
    | nil => simp at h
    | cons y ys =>
      simp only [List.length_cons, Nat.succ.injEq] at h
      simpa only [mergeCounts, List.length_cons] using congrArg Nat.succ (ih ys h)

theorem pureProduct_merge {K : Nat} (hK : 0 < K) (xs ys : List Nat) :
    factorProduct (pure K) xs * factorProduct (pure K) ys ≤
      factorProduct (pure K) (mergeCounts xs ys) := by
  induction xs generalizing ys with
  | nil => simp only [factorProduct, mergeCounts, Rat.one_mul]; exact Rat.le_refl
  | cons x xs ih =>
    cases ys with
    | nil => simp only [factorProduct, mergeCounts, Rat.mul_one]; exact Rat.le_refl
    | cons y ys =>
      have htail := ih ys
      have hhead := pure_merge hK x y
      have h := mul_mono
        (Rat.le_of_lt (Rat.mul_pos (pure_pos hK x) (pure_pos hK y)))
        (Rat.le_of_lt (factorProduct_pos (pure K) (mergeCounts xs ys)
          (by intro n _; exact pure_pos hK n))) hhead htail
      simpa only [factorProduct, mergeCounts, Rat.mul_assoc, Rat.mul_comm, mul_left_comm] using h

/-- Aggregate aligned outcome coordinates, retaining all K empty coordinates. -/
def histogram (K : Nat) : List (List Nat) → List Nat
  | [] => List.replicate K 0
  | counts :: rows => mergeCounts counts (histogram K rows)

theorem pureProduct_zeros (K n : Nat) :
    factorProduct (pure K) (List.replicate n 0) = 1 := by
  induction n with
  | zero => rfl
  | succ n ih => simp only [List.replicate_succ, factorProduct, pure_zero, ih, Rat.one_mul]

theorem histogram_sum (K : Nat) (rows : List (List Nat)) :
    (histogram K rows).sum = (rows.map List.sum).sum := by
  induction rows with
  | nil => simp [histogram]
  | cons row rows ih => simp only [histogram, mergeCounts_sum, ih, List.map_cons, List.sum_cons]

theorem histogram_length (K : Nat) (rows : List (List Nat))
    (h : ∀ row ∈ rows, row.length = K) : (histogram K rows).length = K := by
  induction rows with
  | nil => simp only [histogram, List.length_replicate]
  | cons row rows ih =>
    have hr := h row (by simp)
    have ht := ih (by intro r hm; exact h r (by simp [hm]))
    exact (mergeCounts_length row (histogram K rows) (hr.trans ht.symm)).trans hr

/-- Every finite partition is bounded by one pure cell per total outcome count. -/
theorem partition_bound {K : Nat} (hK : 0 < K) (rows : List (List Nat)) :
    factorProduct (cell K) rows ≤ factorProduct (pure K) (histogram K rows) := by
  induction rows with
  | nil => simp only [factorProduct, histogram, pureProduct_zeros]; exact Rat.le_refl
  | cons row rows ih =>
    have h := mul_mono (Rat.le_of_lt (cell_pos hK row))
      (Rat.le_of_lt (factorProduct_pos (pure K) (histogram K rows)
        (by intro n _; exact pure_pos hK n)))
      (cell_split hK row) ih
    exact Rat.le_trans h (pureProduct_merge hK row (histogram K rows))

theorem factorProduct_ne_zero {α : Type} (f : α → Rat) (xs : List α)
    (h : ∀ x ∈ xs, f x ≠ 0) : factorProduct f xs ≠ 0 := by
  induction xs with
  | nil => change (1 : Rat) ≠ 0; decide
  | cons x xs ih =>
    have hx := h x (by simp)
    have ht := ih (by intro y hy; exact h y (by simp [hy]))
    intro hz
    rcases Rat.mul_eq_zero.mp hz with he | he
    · exact hx he
    · exact ht he

theorem factorProduct_div {α : Type} (f g : α → Rat) (xs : List α)
    (hg : ∀ x ∈ xs, g x ≠ 0) :
    factorProduct (fun x => f x / g x) xs = factorProduct f xs / factorProduct g xs := by
  induction xs with
  | nil => simp only [factorProduct]; grind
  | cons x xs ih =>
    have hx := hg x (by simp)
    have ht : ∀ y ∈ xs, g y ≠ 0 := by intro y hy; exact hg y (by simp [hy])
    simp only [factorProduct, ih ht]
    exact quotient_mul _ _ _ _ hx (factorProduct_ne_zero g xs ht)

theorem weight_eq_pure_div {K : Nat} (hK : 0 < K) (n : Nat) :
    weight K n = pure K n / (n : Rat)^n := by
  have hk : 0 < (K : Rat) := Rat.natCast_pos.mpr hK
  have hb : 0 < (K : Rat)/2 := by grind
  have he := quotient_mul (rising (1/2) n) (rising ((K : Rat)/2) n) 1 ((n : Rat)^n)
    (Rat.ne_of_gt (rising_pos hb n)) (Rat.ne_of_gt (self_power_pos n))
  simpa only [weight, pure, Rat.div_def, Rat.one_mul, Rat.mul_one] using he.symm

/-- A concrete diagonal partition: one row for each outcome, including neutral zero rows. -/
def pureRows : List Nat → List (List Nat)
  | [] => []
  | n :: ns => (n :: List.replicate ns.length 0) :: (pureRows ns).map (0 :: ·)

theorem pureRows_width (counts : List Nat) :
    ∀ row ∈ pureRows counts, row.length = counts.length := by
  induction counts with
  | nil => simp only [pureRows, List.not_mem_nil]; intro _ h; exact h.elim
  | cons n ns ih =>
    intro row hr
    simp only [pureRows, List.mem_cons, List.mem_map] at hr
    rcases hr with he | ⟨tail, ht, he⟩
    · subst row
      simp only [List.length_cons, List.length_replicate]
    · subst row
      simp only [List.length_cons, ih tail ht]

private theorem mergeCounts_zeros_left (ns : List Nat) :
    mergeCounts (List.replicate ns.length 0) ns = ns := by
  induction ns with
  | nil => rfl
  | cons n ns ih => simp only [List.length_cons, List.replicate_succ, mergeCounts, Nat.zero_add, ih]

private theorem histogram_zero_cons (K : Nat) (rows : List (List Nat)) :
    histogram (K+1) (rows.map (0 :: ·)) = 0 :: histogram K rows := by
  induction rows with
  | nil => simp only [List.map_nil, histogram, List.replicate_succ]
  | cons row rows ih => simp only [List.map_cons, histogram, ih, mergeCounts, Nat.zero_add]

theorem histogram_pureRows (counts : List Nat) :
    histogram counts.length (pureRows counts) = counts := by
  induction counts with
  | nil => rfl
  | cons n ns ih =>
    simp only [pureRows, List.length_cons, histogram, histogram_zero_cons, mergeCounts,
      Nat.add_zero, ih, mergeCounts_zeros_left]

private theorem risingProduct_zeros (a : Rat) (n : Nat) :
    factorProduct (rising a) (List.replicate n 0) = 1 := by
  induction n with
  | zero => rfl
  | succ n ih => simp only [List.replicate_succ, factorProduct, rising_zero, ih, Rat.one_mul]

theorem cell_zero_cons (K : Nat) (row : List Nat) : cell K (0 :: row) = cell K row := by
  simp only [cell, factorProduct, rising_zero, Rat.one_mul, List.sum_cons, Nat.zero_add]

theorem cell_pure_row (K n width : Nat) : cell K (n :: List.replicate width 0) = pure K n := by
  simp only [cell, factorProduct, risingProduct_zeros, Rat.mul_one, List.sum_cons,
    List.sum_replicate_nat, Nat.mul_zero, Nat.add_zero, pure]

private theorem cellProduct_zero_cons (K : Nat) (rows : List (List Nat)) :
    factorProduct (cell K) (rows.map (0 :: ·)) = factorProduct (cell K) rows := by
  induction rows with
  | nil => rfl
  | cons row rows ih => simp only [List.map_cons, factorProduct, cell_zero_cons, ih]

theorem pureRows_attains (K : Nat) (counts : List Nat) :
    factorProduct (cell K) (pureRows counts) = factorProduct (pure K) counts := by
  induction counts with
  | nil => rfl
  | cons n ns ih =>
    simp only [pureRows, factorProduct, cell_pure_row, cellProduct_zero_cons, ih]

theorem cell_zero (K width : Nat) : cell K (List.replicate width 0) = 1 := by
  simp only [cell, risingProduct_zeros, List.sum_replicate_nat, Nat.mul_zero, rising_zero]
  grind

theorem cellProduct_delete_zero (K width : Nat) (before after : List (List Nat)) :
    factorProduct (cell K) (before ++ (List.replicate width 0 :: after)) =
      factorProduct (cell K) (before ++ after) := by
  induction before with
  | nil => simp only [List.nil_append, factorProduct, cell_zero, Rat.one_mul]
  | cons row before ih => simp only [List.cons_append, factorProduct, ih]

theorem histogram_zero_row (K : Nat) (rows : List (List Nat))
    (h : ∀ row ∈ rows, row.length = K) :
    histogram K (List.replicate K 0 :: rows) = histogram K rows := by
  change mergeCounts (List.replicate K 0) (histogram K rows) = histogram K rows
  have hlen := histogram_length K rows h
  simpa only [hlen] using mergeCounts_zeros_left (histogram K rows)

theorem histogram_delete_zero (K : Nat) (before after : List (List Nat))
    (h : ∀ row ∈ before ++ after, row.length = K) :
    histogram K (before ++ (List.replicate K 0 :: after)) = histogram K (before ++ after) := by
  have hz := histogram_zero_row K after (by
    intro row hr
    exact h row (List.mem_append_right before hr))
  clear h
  induction before with
  | nil => exact hz
  | cons row before ih => simp only [List.cons_append, histogram, ih]

end RefinementFeasibility
