import Init

/-!
Small total and partial readout contracts. This file proves no claim about
quantum recovery, SLT, statistical validity, physical learning, or Rust.
Resource costs and measurable risk require additional contracts.
-/
set_option autoImplicit false

namespace AdaptiveObservation
universe u v w x y

theorem xor_cancel (b c : Bool) : Bool.xor (Bool.xor b c) c = b := by
  cases b <;> cases c <;> rfl

def lift {X : Type u} (f : X → Bool) (s : X × Bool) : X × Bool :=
  (s.1, Bool.xor s.2 (f s.1))

theorem lift_preserves_source {X : Type u} (f : X → Bool) (s : X × Bool) :
    (lift f s).1 = s.1 := rfl

theorem lift_initial_readout {X : Type u} (f : X → Bool) (a : X) :
    (lift f (a, false)).2 = f a := by
  cases h : f a <;> simp [lift, h]

theorem lift_involutive {X : Type u} (f : X → Bool) (s : X × Bool) :
    lift f (lift f s) = s := by
  cases s with
  | mk a b =>
    show (a, Bool.xor (Bool.xor b (f a)) (f a)) = (a, b)
    rw [xor_cancel]

theorem lift_injective {X : Type u} (f : X → Bool) (s t : X × Bool)
    (h : lift f s = lift f t) : s = t := by
  have q := congrArg (lift f) h
  rw [lift_involutive, lift_involutive] at q
  exact q

def liftBits {X : Type u} {I : Type v}
    (f : X → I → Bool) (s : X × (I → Bool)) : X × (I → Bool) :=
  (s.1, fun i => Bool.xor (s.2 i) (f s.1 i))

theorem liftBits_involutive {X : Type u} {I : Type v}
    (f : X → I → Bool) (s : X × (I → Bool)) :
    liftBits f (liftBits f s) = s := by
  cases s with
  | mk a b =>
    have h : (fun i => Bool.xor (Bool.xor (b i) (f a i)) (f a i)) = b := by
      funext i
      exact xor_cancel (b i) (f a i)
    exact congrArg (fun q => (a, q)) h

theorem liftBits_initial_readout {X : Type u} {I : Type v}
    (f : X → I → Bool) (a : X) :
    (liftBits f (a, fun _ => false)).2 = f a := by
  funext i
  cases h : f a i <;> simp [liftBits, h]

def transportDecision {Z0 : Type u} {Z1 : Type v} {A : Type w}
    (projection : Z1 → Z0) (old : Z0 → A) : Z1 → A :=
  fun z => old (projection z)

theorem decision_preserved {H : Type u} {Z0 : Type v} {Z1 : Type w} {A : Type x}
    (r0 : H → Z0) (r1 : H → Z1) (projection : Z1 → Z0)
    (refines : ∀ h, r0 h = projection (r1 h)) (old : Z0 → A) (h : H) :
    old (r0 h) = transportDecision projection old (r1 h) :=
  congrArg old (refines h)

theorem pointwise_loss_preserved
    {H : Type u} {Z0 : Type v} {Z1 : Type w} {A : Type x} {L : Type y}
    (r0 : H → Z0) (r1 : H → Z1) (projection : Z1 → Z0)
    (refines : ∀ h, r0 h = projection (r1 h)) (old : Z0 → A)
    (loss : H → A → L) (h : H) :
    loss h (old (r0 h)) = loss h (transportDecision projection old (r1 h)) :=
  congrArg (loss h) (decision_preserved r0 r1 projection refines old h)

theorem partial_decision_preserved
    {H : Type u} {Z0 : Type v} {Z1 : Type w} {A : Type x}
    (r0 : H → Z0) (r1 : H → Z1) (projection : Z1 → Z0)
    (refines : ∀ h, r0 h = projection (r1 h)) (old : Z0 → Option A) (h : H) :
    old (r0 h) = transportDecision projection old (r1 h) :=
  decision_preserved r0 r1 projection refines old h

theorem undefined_reflected
    {H : Type u} {Z0 : Type v} {Z1 : Type w} {A : Type x}
    (r0 : H → Z0) (r1 : H → Z1) (projection : Z1 → Z0)
    (refines : ∀ h, r0 h = projection (r1 h)) (old : Z0 → Option A) (h : H) :
    old (r0 h) = none ↔ transportDecision projection old (r1 h) = none := by
  rw [partial_decision_preserved r0 r1 projection refines old h]

theorem no_factor_of_separated_fiber {H : Type u} {Z : Type v} {Y : Type w}
    (r : H → Z) (target : H → Y) (a b : H)
    (same : r a = r b) (different : target a ≠ target b) :
    ¬ ∃ readout : Z → Y, ∀ h, readout (r h) = target h := by
  intro factors
  obtain ⟨readout, valid⟩ := factors
  apply different
  exact (valid a).symm.trans ((congrArg readout same).trans (valid b))

theorem blind_bit_impossible :
    ¬ ∃ readout : Unit → Bool, ∀ b : Bool, readout () = b := by
  apply no_factor_of_separated_fiber (fun _ : Bool => ()) (fun b => b) false true rfl
  decide

theorem richer_bit_suffices :
    ∃ readout : Bool → Bool, ∀ b, readout b = b :=
  ⟨fun b => b, fun _ => rfl⟩

#print axioms xor_cancel
#print axioms lift_preserves_source
#print axioms lift_initial_readout
#print axioms lift_involutive
#print axioms lift_injective
#print axioms liftBits_involutive
#print axioms liftBits_initial_readout
#print axioms decision_preserved
#print axioms pointwise_loss_preserved
#print axioms partial_decision_preserved
#print axioms undefined_reflected
#print axioms no_factor_of_separated_fiber
#print axioms blind_bit_impossible
#print axioms richer_bit_suffices

end AdaptiveObservation
