import Init

/-! Exact partial-program embeddings with explicit total-cost contracts.
None is a semantic outcome, never operational suspension. Cost includes every
translation, search, library-acquisition and execution charge being claimed;
these laws cannot establish an omitted charge or a runtime cost measurement.
Both bounds must use the same accumulated intermediate cost. Monotonicity
alone cannot recover already-incurred charges after a budget map shrinks them.
Risk transport uses one supplied aggregation on a fixed input domain.
It is not a Blackwell, probabilistic-TV or optimal-search theorem. -/
namespace ResourceTransfer
set_option autoImplicit false
set_option warningAsError true
universe p q r x y z a b c l k

/-- One explicit total-cost convention; none of its components is inferred. -/
def totalCost (execution translation search acquisition : Nat) : Nat :=
  execution + translation + search + acquisition

structure Language (P : Type p) (X : Type x) (Y : Type a) where
  denote : P → X → Option Y
  cost : P → X → Nat

structure Transfer {P : Type p} {Q : Type q} {X : Type x} {Z : Type y}
    {A : Type a} {B : Type b} (s : Language P X A) (t : Language Q Z B) where
  translate : P → Q
  input : X → Z
  output : A → B
  budget : Nat → Nat
  monotone : ∀ m n, m ≤ n → budget m ≤ budget n
  exact : ∀ p x, t.denote (translate p) (input x) = (s.denote p x).map output
  bounded : ∀ p x, t.cost (translate p) (input x) ≤ budget (s.cost p x)

theorem natural_order_transitive (a b c : Nat) (ab : a ≤ b) (bc : b ≤ c) : a ≤ c :=
  Nat.le_trans ab bc

theorem option_map_composition {A : Type a} {B : Type b} {C : Type c}
    (f : A → B) (g : B → C) (v : Option A) :
    (v.map f).map g = v.map (fun x => g (f x)) := by cases v <;> rfl

section Composition
variable {P : Type p} {Q : Type q} {R : Type r}
variable {X : Type x} {Y : Type y} {Z : Type z} {A : Type a} {B : Type b} {C : Type c}
variable {s : Language P X A} {t : Language Q Y B} {u : Language R Z C}
variable (st : Transfer s t) (tu : Transfer t u)

theorem semantic_composition (program : P) (input : X) :
    u.denote (tu.translate (st.translate program)) (tu.input (st.input input)) =
      (s.denote program input).map (fun y => tu.output (st.output y)) := by
  rw [tu.exact, st.exact, option_map_composition]

theorem cost_composition (program : P) (input : X) :
    u.cost (tu.translate (st.translate program)) (tu.input (st.input input)) ≤
      tu.budget (st.budget (s.cost program input)) :=
  Nat.le_trans (tu.bounded _ _) (tu.monotone _ _ (st.bounded _ _))

theorem monotone_composition (f g : Nat → Nat)
    (hf : ∀ m n, m ≤ n → f m ≤ f n) (hg : ∀ m n, m ≤ n → g m ≤ g n)
    (m n : Nat) (h : m ≤ n) : g (f m) ≤ g (f n) := hg _ _ (hf _ _ h)

def compose : Transfer s u where
  translate := fun p => tu.translate (st.translate p)
  input := fun x => tu.input (st.input x)
  output := fun y => tu.output (st.output y)
  budget := fun n => tu.budget (st.budget n)
  monotone := monotone_composition st.budget tu.budget st.monotone tu.monotone
  exact := semantic_composition st tu
  bounded := cost_composition st tu
end Composition

section OneTransfer
variable {P : Type p} {Q : Type q} {X : Type x} {Z : Type y} {A : Type a} {B : Type b}
variable {s : Language P X A} {t : Language Q Z B} (tr : Transfer s t) (program : P) (input : X)

theorem none_preserved (h : s.denote program input = none) :
    t.denote (tr.translate program) (tr.input input) = none := by rw [tr.exact, h]; rfl

theorem none_reflected (h : t.denote (tr.translate program) (tr.input input) = none) :
    s.denote program input = none := by
  rw [tr.exact] at h
  cases e : s.denote program input with
  | none => rfl
  | some y => simp [e] at h

theorem some_preserved (value : A) (h : s.denote program input = some value) :
    t.denote (tr.translate program) (tr.input input) = some (tr.output value) := by
  rw [tr.exact, h]; rfl

theorem some_reflected (injective : ∀ a b, tr.output a = tr.output b → a = b)
    (value : A) (h : t.denote (tr.translate program) (tr.input input) = some (tr.output value)) :
    s.denote program input = some value := by
  rw [tr.exact] at h
  cases e : s.denote program input with
  | none => simp [e] at h
  | some y => simp only [e, Option.map_some, Option.some.injEq] at h
              exact congrArg some (injective y value h)

theorem budget_transport (budget : Nat) (h : s.cost program input ≤ budget) :
    t.cost (tr.translate program) (tr.input input) ≤ tr.budget budget :=
  Nat.le_trans (tr.bounded _ _) (tr.monotone _ _ h)

theorem pointwise_loss_transport {L : Type l}
    (sourceLoss : X → Option A → L) (targetLoss : Z → Option B → L)
    (compatible : ∀ x result, targetLoss (tr.input x) (result.map tr.output) = sourceLoss x result) :
    targetLoss (tr.input input) (t.denote (tr.translate program) (tr.input input)) =
      sourceLoss input (s.denote program input) := by rw [tr.exact, compatible]

theorem fixed_aggregate_risk_transport {L : Type l} {R : Type k}
    (sourceLoss : X → Option A → L) (targetLoss : Z → Option B → L)
    (compatible : ∀ x result, targetLoss (tr.input x) (result.map tr.output) = sourceLoss x result)
    (aggregate : (X → L) → R) :
    aggregate (fun x => targetLoss (tr.input x) (t.denote (tr.translate program) (tr.input x))) =
      aggregate (fun x => sourceLoss x (s.denote program x)) := by
  apply congrArg aggregate
  funext x
  exact pointwise_loss_transport tr program x sourceLoss targetLoss compatible
end OneTransfer

/-- Already-paid work is never passed through an execution-only budget map. -/
def advancePaid (overhead : Nat) (executionMap : Nat → Nat) (state : Nat × Nat) : Nat × Nat :=
  (state.1 + overhead, executionMap state.2)

theorem paid_budgets_compose (a b : Nat) (f g : Nat → Nat) (state : Nat × Nat) :
    advancePaid b g (advancePaid a f state) = (state.1 + a + b, g (f state.2)) := rfl

theorem paid_work_monotone (overhead : Nat) (executionMap : Nat → Nat) (state : Nat × Nat) :
    state.1 ≤ (advancePaid overhead executionMap state).1 := Nat.le_add_right _ _

theorem lifetime_budget_transport (a b : Nat) (f g : Nat → Nat) (state : Nat × Nat)
    (execution : Nat) (within : execution ≤ g (f state.2)) :
    (advancePaid b g (advancePaid a f state)).1 + execution ≤
      state.1 + a + b + g (f state.2) := Nat.add_le_add_left within _

end ResourceTransfer
