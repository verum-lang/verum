import Lean
import VerumExternalReplay.ConservativeRefinement
import VerumExternalReplay.ExactTransports
import VerumExternalReplay.TypedPrograms

/-! Pure partial observers for one owned event history. `none` is semantic
undefinedness, never checker exhaustion. No theorem here certifies acquisition,
resource bounds, an ownership implementation, or a Rust/compiler adapter. -/
namespace PartialObservers
set_option warningAsError true
universe u v w u' v' w' u'' v'' w''

structure Observer (E : Type u) (Z : Type v) (R : Type w) where
  initial : Z
  update : Z → E → Option Z
  read : Z → Option R

def fold (step : Z → E → Option Z) : List E → Z → Option Z
  | [], z => some z
  | e :: es, z => (step z e).bind (fold step es)

-- Compose one checked transition without expanding the remaining history.
theorem fold_cons_success (step : Z → E → Option Z) (event : E) (events : List E)
    (state next : Z) (result : Option Z) (transition : step state event = some next)
    (tail : fold step events next = result) : fold step (event :: events) state = result := by
  rw [fold, transition]
  exact tail

/-- Normalize each transition with proof-producing `cbv`, then compose its
kernel-checked equality. An iterative tactic avoids both recursive tactic
expansion and accumulating unevaluated prior states. The final equality still
checks the exact remaining history and result; the count is not an admission.
No native evaluation axiom or increased proof limit is used. -/
elab "reduce_observer_fold" count:num : tactic => do
  for _ in [:count.getNat] do
    Lean.Elab.Tactic.evalTactic (← `(tactic| apply PartialObservers.fold_cons_success))
    Lean.Elab.Tactic.evalTactic (← `(tactic| (solve | (conv => lhs; cbv) <;> rfl)))
  Lean.Elab.Tactic.evalTactic (← `(tactic| rfl))

def online (step : Z → E → Option Z) (events : List E) (current : Option Z) : Option Z :=
  events.foldl (fun state event => state.bind (fun z => step z event)) current

theorem fold_online (step : Z → E → Option Z) (events : List E) (current : Option Z) :
    current.bind (fold step events) = online step events current := by
  induction events generalizing current with
  | nil => cases current <;> rfl
  | cons e es ih =>
      change _ = online step es (current.bind (fun z => step z e))
      rw [← ih]
      cases current <;> simp [fold]

theorem fold_append (step : Z → E → Option Z) (before after : List E) (z : Z) :
    fold step (before ++ after) z = (fold step before z).bind (fold step after) := by
  induction before generalizing z with
  | nil => rfl
  | cons e es ih =>
      simp only [List.cons_append, fold]
      cases step z e <;> simp [ih]

def Observer.replay (o : Observer E Z R) (events : List E) : Option Z :=
  fold o.update events o.initial

def Observer.observe (o : Observer E Z R) (events : List E) : Option R :=
  (o.replay events).bind o.read

structure Transport (source : Observer E Z R) (target : Observer E' Z' R')
    (state : Z → Z') (event : E → E') (result : R → R') : Prop where
  initial : target.initial = state source.initial
  update : ∀ z e, target.update (state z) (event e) = (source.update z e).map state
  read : ∀ z, target.read (state z) = (source.read z).map result

variable {E : Type u} {Z : Type v} {R : Type w}
  {E' : Type u'} {Z' : Type v'} {R' : Type w'}
  {source : Observer E Z R} {target : Observer E' Z' R'}
  {state : Z → Z'} {event : E → E'} {result : R → R'}

theorem fold_transport (source : Z → E → Option Z) (target : Z' → E' → Option Z')
    (state : Z → Z') (event : E → E')
    (commutes : ∀ z e, target (state z) (event e) = (source z e).map state)
    (events : List E) (z : Z) :
    fold target (events.map event) (state z) = (fold source events z).map state := by
  induction events generalizing z with
  | nil => rfl
  | cons e es ih =>
      simp only [List.map_cons, fold, commutes]
      cases source z e <;> simp [ih]

theorem Transport.replay (h : Transport source target state event result) (events : List E) :
    target.replay (events.map event) = (source.replay events).map state := by
  unfold Observer.replay
  rw [h.initial]
  exact fold_transport _ _ _ _ h.update events _

theorem Transport.observe (h : Transport source target state event result) (events : List E) :
    target.observe (events.map event) = (source.observe events).map result := by
  unfold Observer.observe
  rw [h.replay]
  cases source.replay events <;> simp [h.read]

theorem Transport.definedness (h : Transport source target state event result) (events : List E) :
    (target.replay (events.map event)).isSome = (source.replay events).isSome ∧
    (target.observe (events.map event)).isSome = (source.observe events).isSome := by
  rw [h.replay, h.observe]
  simp

theorem Transport.compose
    (first : Transport source middle state₁ event₁ result₁)
    (second : Transport middle target state₂ event₂ result₂) :
    Transport source target (state₂ ∘ state₁) (event₂ ∘ event₁) (result₂ ∘ result₁) := by
  refine ⟨?_, ?_, ?_⟩
  · rw [second.initial, first.initial]; rfl
  · intro z e
    exact ExactTransports.compose
      (fun z => source.update z e) (fun z => middle.update z (event₁ e))
      (fun z => target.update z (event₂ (event₁ e))) state₁ state₂
      (fun z => first.update z e) (fun z => second.update z (event₁ e)) z
  · intro z
    change target.read (state₂ (state₁ z)) = _
    rw [second.read, first.read, Option.map_map]

/-- Expose every visited state through the existing conservative-refinement model. -/
def outcome (value : Option Z) : ConservativeRefinement.Outcome Z Unit Unit :=
  match value with
  | none => .refuse ()
  | some z => .next z

theorem all_admissible (step : Z → E → Option Z) (events : List E) (z : Z) :
    ConservativeRefinement.Admissible (fun z e => outcome (step z e))
      (fun _ _ => True) events z := by
  induction events generalizing z with
  | nil => trivial
  | cons e es ih =>
      constructor
      · trivial
      · cases h : step z e <;> simp only [h, outcome]
        exact ih _

theorem Transport.visited_states (h : Transport source target state event result)
    (events : List E) (z : Z) :
    ConservativeRefinement.run (fun z e => outcome (target.update z (event e))) events (state z) =
      (ConservativeRefinement.run (fun z e => outcome (source.update z e)) events z).map state := by
  apply ConservativeRefinement.finite_execution_preserved state (fun _ e => e)
    (fun z e => outcome (target.update z (event e)))
    (fun z e => outcome (source.update z e)) (fun _ _ => True)
  · intro z e _
    rw [h.update]
    cases source.update z e <;> rfl
  · exact all_admissible source.update events z

def zip (a : Option A) (b : Option B) : Option (A × B) :=
  a.bind (fun x => b.map (fun y => (x, y)))

def Observer.product (a : Observer E A R) (b : Observer E B S) : Observer E (A × B) (R × S) where
  initial := (a.initial, b.initial)
  update z e := zip (a.update z.1 e) (b.update z.2 e)
  read z := zip (a.read z.1) (b.read z.2)

theorem fold_product (a : Observer E A R) (b : Observer E B S) (events : List E) (x : A) (y : B) :
    fold (a.product b).update events (x, y) = zip (fold a.update events x) (fold b.update events y) := by
  induction events generalizing x y with
  | nil => rfl
  | cons e es ih =>
      change (zip (a.update x e) (b.update y e)).bind (fold (a.product b).update es) =
        zip ((a.update x e).bind (fold a.update es)) ((b.update y e).bind (fold b.update es))
      cases a.update x e <;> cases b.update y e <;> simp [zip]
      exact ih _ _

theorem observe_product (a : Observer E A R) (b : Observer E B S) (events : List E) :
    (a.product b).observe events = zip (a.observe events) (b.observe events) := by
  unfold Observer.observe Observer.replay
  rw [fold_product]
  change (zip (fold a.update events a.initial) (fold b.update events b.initial)).bind
      (fun z => zip (a.read z.1) (b.read z.2)) =
    zip ((fold a.update events a.initial).bind a.read) ((fold b.update events b.initial).bind b.read)
  cases fold a.update events a.initial <;> cases fold b.update events b.initial <;>
    simp [zip]

theorem product_domain (a : Observer E A R) (b : Observer E B S) (events : List E) :
    ((a.product b).observe events).isSome =
      ((a.observe events).isSome && (b.observe events).isSome) := by
  rw [observe_product]
  cases a.observe events <;> cases b.observe events <;> rfl

section Programs
open TypedPrograms
variable {Ty : Type u} {Prim : List Ty → Ty → Type v} {Lit : Ty → Type v} {V : Ty → Type w}

/-- A read has its own external query row. Update and read receive the state
followed by their independently typed input contexts. A concrete input binding
can first be expressed by `Program.subst`, whose exact law is already checked. -/
def fromPrograms (sem : Semantics Prim Lit V) (initial : V z)
    (update : Program Prim Lit (z :: events) z)
    (read : Program Prim Lit (z :: queries) r) (query : Env V queries) :
    Observer (Env V events) (V z) (V r) where
  initial := initial
  update state event := update.run sem (Env.cons state event)
  read state := read.run sem (Env.cons state query)

variable {TargetTy : Type u'} {Target : List TargetTy → TargetTy → Type v'}
  {TLit : TargetTy → Type v'} {W : TargetTy → Type w'}
  (ρ : Ty → TargetTy) (literal : ∀ {t}, Lit t → TLit (ρ t))
  (expand : ∀ {params t}, Prim params t → Program Target TLit (params.map ρ) (ρ t))
  (sourceSem : Semantics Prim Lit V) (targetSem : Semantics Target TLit W)
  (j : ∀ {t}, V t → W (ρ t))
  (literal_exact : ∀ {t} (l : Lit t), targetSem.literal (literal l) = j (sourceSem.literal l))
  (primitive_exact : ∀ {params t} (op : Prim params t) (xs : HList V params),
    (expand op).run targetSem (xs.relabel ρ j).get = (sourceSem.primitive op xs).map j)

include literal_exact primitive_exact

theorem program_transport (initial : V z)
    (update : Program Prim Lit (z :: events) z)
    (read : Program Prim Lit (z :: queries) r) (query : Env V queries) :
    Transport (fromPrograms sourceSem initial update read query)
      (fromPrograms targetSem (j initial) (update.translate ρ literal expand)
        (read.translate ρ literal expand) (Env.relabel ρ j query))
      (@j z) (fun event => Env.relabel ρ j event) (@j r) := by
  refine ⟨rfl, ?_, ?_⟩
  · intro state event
    simpa only [fromPrograms, Env.relabel_cons] using
      update.run_translate ρ literal expand sourceSem targetSem j literal_exact primitive_exact
        (Env.cons state event)
  · intro state
    simpa only [fromPrograms, Env.relabel_cons] using
      read.run_translate ρ literal expand sourceSem targetSem j literal_exact primitive_exact
        (Env.cons state query)

end Programs

/-- info: 'PartialObservers.fold_online' depends on axioms: [propext] -/
#guard_msgs in
#print axioms fold_online

/-- info: 'PartialObservers.fold_append' depends on axioms: [propext] -/
#guard_msgs in
#print axioms fold_append

/-- info: 'PartialObservers.Transport.observe' depends on axioms: [propext] -/
#guard_msgs in
#print axioms Transport.observe

/-- info: 'PartialObservers.Transport.definedness' depends on axioms: [propext] -/
#guard_msgs in
#print axioms Transport.definedness

/-- info: 'PartialObservers.Transport.compose' depends on axioms: [propext] -/
#guard_msgs in
#print axioms Transport.compose

/-- info: 'PartialObservers.Transport.visited_states' depends on axioms: [propext] -/
#guard_msgs in
#print axioms Transport.visited_states

/-- info: 'PartialObservers.product_domain' depends on axioms: [propext] -/
#guard_msgs in
#print axioms product_domain

/-- info: 'PartialObservers.program_transport' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms program_transport

end PartialObservers
