import VerumExternalReplay.EnumerableMass

/-! Exact finite-history risk bounds. States may encode whole histories;
children are ordered outcome branches, not a set of equal-valued states.
The horizon is arbitrary. Normalization, causal prediction, null domination
and complete native outcome coverage are premises, not consequences of an
export or of these arithmetic laws. Infinite-horizon probability semantics
and adaptive allocation at random declaration times need separate proofs. -/
namespace FiniteRisk
open ExactMass
set_option warningAsError true

variable {H : Type}

def Subflow (children : H → List H) (mass : H → Fraction) : Prop :=
  ∀ h, AtMost (sum ((children h).map mass)) (mass h)

def Flow (children : H → List H) (mass : H → Fraction) : Prop :=
  ∀ h, Equal (sum ((children h).map mass)) (mass h)

/-- First-hit cylinders form a cut: descendants of a hit are not counted. -/
def stopped (children : H → List H) (mass : H → Fraction) (hit : H → Bool) :
    Nat → H → Fraction
  | 0, h => if hit h then mass h else zero
  | n + 1, h => if hit h then mass h else sum ((children h).map (stopped children mass hit n))

/-- Full leaf mass of paths with a hit anywhere in their observed history. -/
def eventMass (children : H → List H) (mass : H → Fraction) (hit : H → Bool) :
    Nat → H → Fraction
  | 0, h => if hit h then mass h else zero
  | n + 1, h => sum ((children h).map
      (eventMass children mass (fun next => hit h || hit next) n))

theorem stopped_bounded (children : H → List H) (mass : H → Fraction)
    (hit : H → Bool) (flow : Subflow children mass) (n : Nat) (h : H) :
    AtMost (stopped children mass hit n h) (mass h) := by
  induction n generalizing h with
  | zero => cases hh : hit h <;> simp only [stopped, hh, Bool.false_eq_true, ite_false, ite_true]
            · exact zero_le _
            · exact le_refl _
  | succ n ih =>
    cases hh : hit h <;> simp only [stopped, hh, Bool.false_eq_true, ite_false, ite_true]
    · exact le_trans (sum_mono _ _ _ (fun child _ => ih child)) (flow h)
    · exact le_refl _

theorem event_always (children : H → List H) (mass : H → Fraction)
    (flow : Flow children mass) (n : Nat) (h : H) :
    Equal (eventMass children mass (fun _ => true) n h) (mass h) := by
  induction n generalizing h with
  | zero => exact eq_refl _
  | succ n ih =>
    exact eq_trans (sum_congr _ _ _ (fun child _ => ih child)) (flow h)

theorem event_is_first_hit (children : H → List H) (mass : H → Fraction)
    (flow : Flow children mass) (hit : H → Bool) (n : Nat) (h : H) :
    Equal (eventMass children mass hit n h) (stopped children mass hit n h) := by
  induction n generalizing hit h with
  | zero => exact eq_refl _
  | succ n ih =>
    cases hh : hit h
    · simp only [eventMass, stopped, hh, Bool.false_or, Bool.false_eq_true, ite_false]
      exact sum_congr _ _ _ (fun child _ => ih hit child)
    · simp only [eventMass, stopped, hh, Bool.true_or, ite_true]
      exact eq_trans (sum_congr _ _ _ (fun child _ => event_always children mass flow n child)) (flow h)

theorem stopped_domination (children : H → List H) (p q : H → Fraction)
    (hit : H → Bool) (risk : Fraction)
    (crossing : ∀ h, hit h = true → AtMost (p h) (mul risk (q h)))
    (n : Nat) (h : H) :
    AtMost (stopped children p hit n h) (mul risk (stopped children q hit n h)) := by
  induction n generalizing h with
  | zero =>
    cases hh : hit h <;> simp only [stopped, hh, Bool.false_eq_true, ite_false, ite_true]
    · exact zero_le _
    · exact crossing h hh
  | succ n ih =>
    cases hh : hit h <;> simp only [stopped, hh, Bool.false_eq_true, ite_false, ite_true]
    · exact le_trans (sum_mono _ _ _ (fun child _ => ih child)) (eq_le (sum_scale _ _ _))
    · exact crossing h hh

theorem finite_crossing_bound (children : H → List H) (p q : H → Fraction)
    (hit : H → Bool) (risk : Fraction) (pflow : Flow children p)
    (qflow : Subflow children q) (root : H) (initial : AtMost (q root) one)
    (crossing : ∀ h, hit h = true → AtMost (p h) (mul risk (q h))) (horizon : Nat) :
    AtMost (eventMass children p hit horizon root) risk :=
  le_trans (eq_le (event_is_first_hit children p pflow hit horizon root))
    (le_trans (stopped_domination children p q hit risk crossing horizon root)
      (le_trans (mul_mono_left risk (stopped_bounded children q hit qflow horizon root))
        (le_trans (mul_mono_left risk initial) (eq_le (mul_one risk)))))

/-- No division is needed, including on null-zero histories. The envelope need
not itself be a mass flow or the resulting ratio a supermartingale. -/
def crosses (risk : Fraction) (q envelope : H → Fraction) (h : H) : Bool :=
  decide (AtMost (envelope h) (mul risk (q h)))

theorem envelope_crossing_bound (children : H → List H) (p q envelope : H → Fraction)
    (risk : Fraction) (pflow : Flow children p) (qflow : Subflow children q)
    (root : H) (initial : AtMost (q root) one)
    (dominates : ∀ h, AtMost (p h) (envelope h)) (horizon : Nat) :
    AtMost (eventMass children p (crosses risk q envelope) horizon root) risk := by
  apply finite_crossing_bound children p q _ risk pflow qflow root initial _ horizon
  intro h hit
  exact le_trans (dominates h) (of_decide_eq_true hit)

theorem event_never (children : H → List H) (mass : H → Fraction) (n : Nat) (h : H) :
    Equal (eventMass children mass (fun _ => false) n h) zero := by
  have zeros (xs : List H) : sum (xs.map (fun _ => zero)) = zero := by
    induction xs with
    | nil => rfl
    | cons x xs ih => simp only [List.map_cons, sum, ih]; rfl
  induction n generalizing h with
  | zero => exact eq_refl _
  | succ n ih =>
    apply eq_trans (sum_congr _ _ _ (fun child _ => ih child))
    rw [zeros]; exact eq_refl _

/-- Events may overlap arbitrarily; no independence premise is used. -/
theorem event_or_bound (children : H → List H) (mass : H → Fraction)
    (a b : H → Bool) (n : Nat) (h : H) :
    AtMost (eventMass children mass (fun x => a x || b x) n h)
      (add (eventMass children mass a n h) (eventMass children mass b n h)) := by
  induction n generalizing a b h with
  | zero =>
    cases ha : a h <;> cases hb : b h <;>
      simp only [eventMass, ha, hb, Bool.false_or, Bool.true_or, Bool.false_eq_true, ite_false, ite_true]
    · exact zero_le _
    · exact le_trans (eq_le (eq_symm (add_zero _))) (eq_le (add_comm _ _))
    · exact le_add _ _
    · exact le_add _ _
  | succ n ih =>
    have rearrange : (fun x => (a h || b h) || (a x || b x)) =
        (fun x => (a h || a x) || (b h || b x)) := by
      funext x
      cases a h <;> cases b h <;> cases a x <;> cases b x <;> rfl
    simp only [eventMass]
    rw [rearrange]
    exact le_trans (sum_mono _ _ _ (fun child _ => ih _ _ child)) (eq_le (sum_add _ _ _))

def anyHit (tests : List (H → Bool)) (h : H) : Bool := tests.any (fun hit => hit h)

theorem event_union_bound (children : H → List H) (mass : H → Fraction)
    (tests : List (H → Bool)) (n : Nat) (h : H) :
    AtMost (eventMass children mass (anyHit tests) n h)
      (sum (tests.map (fun hit => eventMass children mass hit n h))) := by
  induction tests with
  | nil => exact eq_le (event_never children mass n h)
  | cons first rest ih =>
    exact le_trans (event_or_bound children mass first (anyHit rest) n h)
      (add_mono_left _ ih)

theorem envelope_family_bound {I : Type} (children : H → List H) (p : H → Fraction)
    (q envelope : I → H → Fraction) (weight : I → Fraction) (family : List I)
    (alpha : Fraction) (pflow : Flow children p) (root : H)
    (qflow : ∀ i ∈ family, Subflow children (q i))
    (initial : ∀ i ∈ family, AtMost (q i root) one)
    (dominates : ∀ i ∈ family, ∀ h, AtMost (p h) (envelope i h))
    (allocated : AtMost (sum (family.map weight)) one) (horizon : Nat) :
    AtMost (eventMass children p
      (anyHit (family.map (fun i => crosses (mul alpha (weight i)) (q i) (envelope i))))
      horizon root) alpha := by
  apply le_trans (event_union_bound children p _ horizon root)
  simp only [List.map_map]
  apply le_trans (sum_mono family _ _ (fun i hi =>
    envelope_crossing_bound children p (q i) (envelope i) (mul alpha (weight i))
      pflow (qflow i hi) root (initial i hi) (dominates i hi) horizon))
  exact le_trans (eq_le (sum_scale family weight alpha))
    (le_trans (mul_mono_left alpha allocated) (eq_le (mul_one alpha)))

end FiniteRisk
