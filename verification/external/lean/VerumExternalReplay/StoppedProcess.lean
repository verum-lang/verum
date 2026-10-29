import VerumExternalReplay.FiniteWire

/-! Prefix-dependent stopping on finite blocks. The same causal body kernel is
used at every boundary; neither future contents nor a final length is an input.
Finite-cut conservation is separate from proper termination, which requires a
uniform bound on continuation. All arithmetic is exact and constructive. -/
namespace StoppedProcess
open ExactMass EnumerableMass CausalCoding
set_option warningAsError true

structure Model (State : Type) where
  unitBits : Nat
  positiveWidth : 0 < unitBits
  body : Kernel State
  stop : State → Bool → Fraction
  normalized : ∀ state, Equal (add (stop state false) (stop state true)) one

variable {State : Type}

/-- One continuation decision followed by exactly one complete block. -/
def extend (model : Model State) (state : State) (future : State → Fraction) : Fraction :=
  mul (model.stop state false) (sum ((words model.unitBits).map (fun bits =>
    mul (likelihood model.body state bits) (future (advance model.body state bits)))))

theorem extend_congr (model : Model State) (state : State) (f g : State → Fraction)
    (same : ∀ s, Equal (f s) (g s)) : Equal (extend model state f) (extend model state g) :=
  mul_congr_left _ (sum_congr _ _ _ (fun _ _ => mul_congr_left _ (same _)))

theorem extend_add (model : Model State) (state : State) (f g : State → Fraction) :
    Equal (extend model state (fun s => add (f s) (g s)))
      (add (extend model state f) (extend model state g)) := by
  apply eq_trans (mul_congr_left _ (sum_congr _ _ _ (fun _ _ => mul_add _ _ _)))
  exact eq_trans (mul_congr_left _ (sum_add _ _ _)) (mul_add _ _ _)

theorem extend_constant (model : Model State) (state : State) (value : Fraction) :
    Equal (extend model state (fun _ => value)) (mul (model.stop state false) value) := by
  apply mul_congr_left
  apply eq_trans (sum_congr _ _ _ (fun _ _ => mul_comm _ _))
  apply eq_trans (sum_scale _ _ _)
  exact eq_trans (mul_congr_left _ (body_normalized _ _ _)) (mul_one _)

theorem extend_bounded (model : Model State) (state : State) (future : State → Fraction)
    (bound : Fraction) (bounded : ∀ s, AtMost (future s) bound) :
    AtMost (extend model state future) (mul (model.stop state false) bound) :=
  le_trans (mul_mono_left _ (sum_mono _ _ _ (fun _ _ => mul_mono_left _ (bounded _))))
    (eq_le (extend_constant model state bound))

/-- Mass of all bodies terminating after exactly n further blocks. -/
def slice (model : Model State) : Nat → State → Fraction
  | 0, state => model.stop state true
  | n+1, state => extend model state (slice model n)

/-- Mass surviving n complete blocks; includes the boundary before its STOP. -/
def tail (model : Model State) : Nat → State → Fraction
  | 0, _ => one
  | n+1, state => extend model state (tail model n)

theorem point_tail (model : Model State) (n : Nat) (state : State) :
    Equal (add (slice model n state) (tail model (n+1) state)) (tail model n state) := by
  induction n generalizing state with
  | zero =>
    apply eq_trans (add_congr_left _ (eq_trans (extend_constant model state one) (mul_one _)))
    exact eq_trans (add_comm _ _) (model.normalized state)
  | succ n ih =>
    exact eq_trans (eq_symm (extend_add model state _ _))
      (extend_congr model state _ _ (ih))

theorem normalized_prefix (model : Model State) (state : State) (n : Nat) :
    Equal (add (partialSum (fun k => slice model k state) n) (tail model n state)) one := by
  induction n with
  | zero => exact eq_trans (add_comm _ _) (add_zero _)
  | succ n ih =>
    exact eq_trans (add_assoc _ _ _) (eq_trans (add_congr_left _ (point_tail model n state)) ih)

/-- Geometric envelope with continuation at most c/(c+1). -/
def decay (c n : Nat) : Fraction := ⟨c^n,(c+1)^n,Nat.pow_pos (by omega)⟩

theorem decay_succ (c n : Nat) : decay c (n+1) = mul (decay c 1) (decay c n) := by
  simp [decay,mul,Nat.pow_succ,Nat.mul_comm]

theorem tail_bounded (model : Model State) (c : Nat)
    (bounded : ∀ state, AtMost (model.stop state false) (decay c 1)) (n : Nat) (state : State) :
    AtMost (tail model n state) (decay c n) := by
  induction n generalizing state with
  | zero => exact le_refl _
  | succ n ih =>
    rw [decay_succ]
    exact le_trans (extend_bounded model state _ _ ih) (mul_mono_right _ (bounded state))

/-- A rational Bernoulli bound gives an explicit tail cutoff without real limits. -/
theorem decay_bound (c n : Nat) :
    (c+1+n) * c^n ≤ (c+1) * (c+1)^n := by
  induction n with
  | zero => simp
  | succ n ih =>
    have factor : (c+1+(n+1))*c ≤ (c+1+n)*(c+1) := by
      simp only [Nat.add_mul,Nat.mul_add,Nat.mul_one,Nat.one_mul]
      omega
    have first := Nat.mul_le_mul_right (c^n) factor
    have second := Nat.mul_le_mul_right (c+1) ih
    simp only [Nat.pow_succ,Nat.mul_assoc,Nat.mul_comm,Nat.mul_left_comm] at first second ⊢
    exact Nat.le_trans first second

theorem decay_vanishes (c : Nat) (epsilon : Fraction) (positive : 0 < epsilon.numerator)
    (n : Nat) (large : (c+1)*epsilon.denominator ≤ n) : Less (decay c n) epsilon := by
  have coarse : AtMost (decay c n) (⟨c+1,c+1+n,by omega⟩ : Fraction) := by
    simpa only [AtMost,decay,Nat.mul_comm] using decay_bound c n
  have scale : c+1+n ≤ epsilon.numerator*(c+1+n) := Nat.le_mul_of_pos_left _ positive
  have small : Less (⟨c+1,c+1+n,by omega⟩ : Fraction) epsilon := by
    change (c+1)*epsilon.denominator < epsilon.numerator*(c+1+n)
    omega
  exact le_less_trans coarse small

def lengthLaw (model : Model State) (state : State) (c : Nat)
    (bounded : ∀ s, AtMost (model.stop s false) (decay c 1)) : FiniteWire.LengthLaw where
  weight n := slice model n state
  remaining n := tail model n state
  conserved := normalized_prefix model state
  vanishes epsilon positive := by
    refine ⟨(c+1)*epsilon.denominator, ?_⟩
    intro n large
    exact le_less_trans (tail_bounded model c bounded n state)
      (decay_vanishes c epsilon positive n large)

/-- The prefix cylinder deliberately omits its final STOP factor. -/
def cylinder (model : Model State) (state : State) : List (List Bool) → Fraction
  | [] => one
  | bits :: rest => mul (mul (model.stop state false) (likelihood model.body state bits))
      (cylinder model (advance model.body state bits) rest)

def endpoint (model : Model State) (state : State) : List (List Bool) → State
  | [] => state
  | bits :: rest => endpoint model (advance model.body state bits) rest

def point (model : Model State) (state : State) (blocks : List (List Bool)) : Fraction :=
  mul (cylinder model state blocks) (model.stop (endpoint model state blocks) true)

/-- Partial or oversized blocks are not silently interpreted as complete units. -/
def score (model : Model State) (state : State) (blocks : List (List Bool)) : Option Fraction :=
  if blocks.all (fun bits => decide (bits.length = model.unitBits)) then
    some (point model state blocks)
  else none

theorem accepted_blocks (model : Model State) (state : State) (blocks : List (List Bool))
    (mass : Fraction) (accepted : score model state blocks = some mass) :
    ∀ bits ∈ blocks, bits.length = model.unitBits := by
  unfold score at accepted
  split at accepted
  · rename_i valid
    simpa only [decide_eq_true_eq] using List.all_eq_true.mp valid
  · contradiction

theorem endpoint_append (model : Model State) (state : State) (past future : List (List Bool)) :
    endpoint model state (past ++ future) = endpoint model (endpoint model state past) future := by
  induction past generalizing state with
  | nil => rfl
  | cons bits rest ih => exact ih _

theorem cylinder_append (model : Model State) (state : State) (past future : List (List Bool)) :
    Equal (cylinder model state (past ++ future))
      (mul (cylinder model state past) (cylinder model (endpoint model state past) future)) := by
  induction past generalizing state with
  | nil => exact eq_symm (eq_trans (mul_comm _ _) (mul_one _))
  | cons bits rest ih =>
    simp only [List.cons_append,cylinder,endpoint,mul_assoc_exact]
    exact mul_congr_left _ (mul_congr_left _ (ih _))

theorem point_extension (model : Model State) (state : State) (past future : List (List Bool)) :
    Equal (point model state (past ++ future))
      (mul (cylinder model state past) (point model (endpoint model state past) future)) := by
  unfold point
  rw [endpoint_append,← mul_assoc_exact]
  exact mul_congr_right _ (cylinder_append model state past future)

/-- Every finite cut of continuations plus its surviving remainder equals the
same cylinder. For well-sized blocks this is the complete prefix event. -/
theorem cylinder_partition (model : Model State) (state : State) (past : List (List Bool)) (n : Nat) :
    Equal (add
      (mul (cylinder model state past) (partialSum (fun k => slice model k (endpoint model state past)) n))
      (mul (cylinder model state past) (tail model n (endpoint model state past))))
      (cylinder model state past) :=
  eq_trans (eq_symm (mul_add _ _ _))
    (eq_trans (mul_congr_left _ (normalized_prefix model _ n)) (mul_one _))

end StoppedProcess
