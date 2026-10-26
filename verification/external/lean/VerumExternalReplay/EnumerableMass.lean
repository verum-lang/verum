import VerumExternalReplay.ExactMass

/- Countable allocation contract with exact positive natural denominators.
   Connecting a concrete prefix grammar to the remaining-mass ledger requires
   its own Kraft/decoder proof. Positive mass does not establish search fairness:
   the separate enumeration contract requires an exact index witness and finite
   cumulative work. No native scheduler, overflow, statistical admission or
   floating-point/logarithm correspondence follows from these model laws. -/
namespace EnumerableMass
open ExactMass
set_option warningAsError true

def partialSum (f : Nat → Fraction) : Nat → Fraction
  | 0 => zero
  | n + 1 => add (partialSum f n) (f n)

structure Ledger where
  weights : Nat → Fraction
  remaining : Nat → Fraction
  initial : Equal (remaining 0) one
  positive : ∀ n, 0 < (weights n).numerator
  conserves : ∀ n, Equal (add (weights n) (remaining (n + 1))) (remaining n)

def mass (ledger : Ledger) : Nat → Fraction := ledger.weights

theorem mass_positive (ledger : Ledger) (index : Nat) : 0 < (mass ledger index).numerator :=
  ledger.positive index

theorem prefix_exact (ledger : Ledger) (n : Nat) :
    Equal (add (partialSum (mass ledger) n) (ledger.remaining n)) one := by
  induction n with
  | zero => exact eq_trans (add_comm _ _) (eq_trans (add_zero _) ledger.initial)
  | succ n ih =>
    exact eq_trans (add_assoc _ _ _) (eq_trans (add_congr_left _ (ledger.conserves n)) ih)

theorem prefix_bounded (ledger : Ledger) (n : Nat) : AtMost (partialSum (mass ledger) n) one :=
  le_trans (le_add _ _) (eq_le (prefix_exact ledger n))

theorem prefix_mono (f g : Nat → Fraction) (pointwise : ∀ i, AtMost (f i) (g i)) (n : Nat) :
    AtMost (partialSum f n) (partialSum g n) := by
  induction n with
  | zero => exact le_refl _
  | succ n ih => exact le_trans (add_mono_right _ ih) (add_mono_left _ (pointwise n))

theorem prefix_congr (f g : Nat → Fraction) (pointwise : ∀ i, Equal (f i) (g i)) (n : Nat) :
    Equal (partialSum f n) (partialSum g n) :=
  le_antisymm (prefix_mono f g (fun i => eq_le (pointwise i)) n)
    (prefix_mono g f (fun i => eq_le (eq_symm (pointwise i))) n)

theorem prefix_scale (f : Nat → Fraction) (scale : Fraction) (n : Nat) :
    Equal (partialSum (fun i => mul scale (f i)) n) (mul scale (partialSum f n)) := by
  induction n with
  | zero => exact eq_symm (mul_zero _)
  | succ n ih => exact eq_trans (add_congr_right _ ih) (eq_symm (mul_add _ _ _))

def allocation (programs looks : Ledger) (program look : Nat) : Fraction :=
  mul (mass programs program) (mass looks look)

theorem allocation_positive (programs looks : Ledger) (program look : Nat) :
    0 < (allocation programs looks program look).numerator :=
  Nat.mul_pos (mass_positive programs program) (mass_positive looks look)

theorem rectangle_exact (programs looks : Ledger) (p l : Nat) :
    Equal (partialSum (fun i => partialSum (allocation programs looks i) l) p)
      (mul (partialSum (mass programs) p) (partialSum (mass looks) l)) := by
  apply eq_trans (prefix_congr _ _ (fun i => prefix_scale (mass looks) (mass programs i) l) p)
  apply eq_trans (prefix_congr _ _ (fun i => mul_comm (mass programs i) _) p)
  exact eq_trans (prefix_scale _ _ _) (mul_comm _ _)

theorem rectangle_bounded (programs looks : Ledger) (p l : Nat) :
    AtMost (partialSum (fun i => partialSum (allocation programs looks i) l) p) one :=
  le_trans (eq_le (rectangle_exact programs looks p l))
    (le_trans (mul_mono_right _ (prefix_bounded programs p))
      (le_trans (eq_le (eq_trans (mul_comm _ _) (mul_one _))) (prefix_bounded looks l)))

-- Each exact (program, look) name occurs at most once in this selection.
-- Invalid/unused words retain their unspent mass; no renormalization occurs.
theorem selected_bounded (programs looks : Ledger) (keep : Nat → Nat → Bool) (p l : Nat) :
    AtMost (partialSum (fun i => partialSum (fun j =>
      if keep i j then allocation programs looks i j else zero) l) p) one := by
  apply le_trans _ (rectangle_bounded programs looks p l)
  apply prefix_mono
  intro i
  apply prefix_mono
  intro j
  split
  · exact le_refl _
  · exact zero_le _

def half (bits : Nat) : Fraction := unit (2 ^ bits) (Nat.two_pow_pos bits)

def dyadic : Ledger where
  weights n := half (n + 1)
  remaining := half
  initial := rfl
  positive _ := by change 0 < 1; decide
  conserves n := by
    simp [Equal, add, half, unit, Nat.pow_succ, Nat.mul_add, Nat.mul_comm, Nat.mul_left_comm]
    omega

def lookReserve (look : Nat) : Fraction := unit (look + 1) (by omega)
def lookWeight (look : Nat) : Fraction := unit ((look + 1) * (look + 2)) (Nat.mul_pos (by omega) (by omega))

theorem look_reserve_balance (look : Nat) :
    Equal (add (lookWeight look) (lookReserve (look + 1))) (lookReserve look) := by
  simp only [Equal, add, lookReserve, lookWeight, unit, Nat.one_mul]
  change ((look + 2) + (look + 1) * (look + 2)) * (look + 1) =
    (look + 1) * (look + 2) * (look + 2)
  rw [Nat.add_comm (look + 2), ← Nat.succ_mul]
  change ((look + 2) * (look + 2)) * (look + 1) = (look + 1) * (look + 2) * (look + 2)
  ac_rfl

def paidLooks : Ledger where
  weights := lookWeight
  remaining := lookReserve
  initial := rfl
  positive _ := by change 0 < 1; decide
  conserves := look_reserve_balance

theorem paid_look_exact (look : Nat) : mass paidLooks look = lookWeight look := rfl

theorem paid_looks_partial_sum (looks : Nat) :
    Equal (add (partialSum (mass paidLooks) looks) (lookReserve looks)) one :=
  prefix_exact paidLooks looks

theorem half_add (first second : Nat) : Equal (half (first + second)) (mul (half first) (half second)) := by
  simp [Equal, half, unit, mul, Nat.pow_add]

theorem triple_mass (initial update read : Nat) :
    Equal (half (initial + update + read)) (mul (mul (half initial) (half update)) (half read)) :=
  eq_trans (half_add _ _) (mul_congr_right _ (half_add _ _))

-- A concrete grammar must prove its assigned length matches its ledger mass.
-- Invalid words may remain unselected, without reallocating their budget.
theorem priced_name (programs : Ledger) (program look bits : Nat)
    (encoding : Equal (mass programs program) (half bits)) :
    Equal (allocation programs paidLooks program look)
      (unit (2 ^ bits * ((look + 1) * (look + 2)))
        (Nat.mul_pos (Nat.two_pow_pos bits) (Nat.mul_pos (by omega) (by omega)))) := by
  exact eq_trans (mul_congr_right (lookWeight look) encoding) (by rfl)

-- The native gamma/ordinal/operand/root decoder must establish this law.
-- It is stronger than success on an isolated word: every suffix is preserved.
structure PrefixCodec (Code : Type) where
  encode : Code → List Bool
  decode : List Bool → Option (Code × List Bool)
  exact : ∀ code tail, decode (encode code ++ tail) = some (code, tail)

theorem prefix_unambiguous (codec : PrefixCodec C) (a b : C) (extension : List Bool)
    (same : codec.encode a ++ extension = codec.encode b) : a = b ∧ extension = [] := by
  have h := congrArg codec.decode same
  rw [codec.exact, ← List.append_nil (codec.encode b), codec.exact] at h
  exact Prod.mk.inj (Option.some.inj h)

def PrefixCodec.product (first : PrefixCodec A) (second : PrefixCodec B) : PrefixCodec (A × B) where
  encode pair := first.encode pair.1 ++ second.encode pair.2
  decode bits := do
    let (a, rest) ← first.decode bits
    let (b, tail) ← second.decode rest
    pure ((a, b), tail)
  exact pair tail := by
    change (first.decode ((first.encode pair.1 ++ second.encode pair.2) ++ tail)).bind _ = _
    rw [List.append_assoc, first.exact]
    change (second.decode (second.encode pair.2 ++ tail)).bind _ = _
    rw [second.exact]
    rfl

theorem encoded_product_mass (first : PrefixCodec A) (second : PrefixCodec B) (a : A) (b : B) :
    Equal (half ((first.product second).encode (a, b)).length)
      (mul (half (first.encode a).length) (half (second.encode b).length)) := by
  simp only [PrefixCodec.product, List.length_append]
  exact half_add _ _

structure Enumeration (Code : Type) where
  decode : Nat → Option Code
  index : Code → Nat
  exact : ∀ code, decode (index code) = some code

theorem index_injective (family : Enumeration C) (a b : C)
    (same : family.index a = family.index b) : a = b := by
  have h := congrArg family.decode same
  rw [family.exact, family.exact] at h
  exact Option.some.inj h

-- One unit accounts for visiting the name; cost includes all finite decoding,
-- preparation and evaluation work assigned to that visit. No mass-to-work or
-- native scheduler correspondence is inferred. C may itself be Code × Look.
def spent (cost : Nat → Nat) : Nat → Nat
  | 0 => 0
  | n + 1 => spent cost n + cost n + 1

def reached (family : Enumeration C) (cost : Nat → Nat) (budget : Nat) (code : C) : Bool :=
  decide (spent cost (family.index code + 1) ≤ budget)

theorem visits_bounded (cost : Nat → Nat) (n : Nat) : n ≤ spent cost n := by
  induction n with
  | zero => simp [spent]
  | succ n ih => simp only [spent]; omega

theorem no_free_infinite_visits (cost : Nat → Nat) (n budget : Nat)
    (paid : spent cost n ≤ budget) : n ≤ budget :=
  Nat.le_trans (visits_bounded cost n) paid

theorem every_code_reachable (family : Enumeration C) (cost : Nat → Nat) (code : C) :
    ∃ threshold, ∀ budget, threshold ≤ budget →
      reached family cost budget code = true ∧ family.decode (family.index code) = some code := by
  refine ⟨spent cost (family.index code + 1), ?_⟩
  intro budget enough
  exact ⟨by simp [reached, enough], family.exact code⟩

theorem reachability_monotone (family : Enumeration C) (cost : Nat → Nat) (code : C)
    (small large : Nat) (order : small ≤ large) (visited : reached family cost small code = true) :
    reached family cost large code = true := by
  simp only [reached, decide_eq_true_eq] at visited ⊢
  exact Nat.le_trans visited order

end EnumerableMass
