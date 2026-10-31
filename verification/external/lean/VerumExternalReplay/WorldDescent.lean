import VerumExternalReplay.ActionQuotient

/-! Descent of an input-dependent observer to a world's action semantics.
The concrete state includes the actual source environment and observer state.
Every source binding, admitted action and query must be represented. This is a
semantic obligation on completed operations, not an atomicity, learning or
caller-correspondence proof. A closed admitted world can be supplied as a subtype.
-/
namespace WorldDescent
set_option warningAsError true
variable {C V E I J O S W Q A : Type}

def coupledStep (capture : C → A → Option (C × E)) (update : V → E → Option V)
    (state : C × V) (action : A) : Option (C × V) :=
  (capture state.1 action).bind fun event =>
    (update state.2 event.2).map fun value => (event.1, value)

def coupledRead (query : C → I → J) (read : V → J → Option O)
    (state : C × V) (input : I) : Option O := read state.2 (query state.1 input)

/-- Exact projection of completed source actions, a factor of the actual
input-dependent program, and coverage of the declared world carrier. Keeping
only one representative trace per world state is insufficient. -/
structure Witness (step : S → A → Option S) (read : S → I → Option O)
    (worldStep : W → A → Option W) (project : S → W)
    (next : Q → A → Option Q) (view : Q → I → Option O) (encode : W → Q) : Prop where
  cover : ∀ world, ∃ state, project state = world
  dispatch : ∀ state action, (step state action).map project = worldStep (project state) action
  factor : ActionQuotient.Factor step read next view (encode ∘ project)

variable {step : S → A → Option S} {read : S → I → Option O}
  {worldStep : W → A → Option W} {project : S → W}
  {next : Q → A → Option Q} {view : Q → I → Option O} {encode : W → Q}

theorem Witness.worldFactor (witness : Witness step read worldStep project next view encode) :
    ActionQuotient.Factor worldStep (fun w query => view (encode w) query) next view encode := by
  constructor
  · intro world action
    obtain ⟨state, rfl⟩ := witness.cover world
    calc
      next (encode (project state)) action = (step state action).map (encode ∘ project) :=
        witness.factor.update state action
      _ = ((step state action).map project).map encode := by
        cases step state action <;> rfl
      _ = (worldStep (project state) action).map encode := by rw [witness.dispatch]
  · intros; rfl

theorem Witness.world_relations (witness : Witness step read worldStep project next view encode)
    (world : W) (left right : List A)
    (relation : PartialObservers.fold worldStep left world = PartialObservers.fold worldStep right world) :
    PartialObservers.fold next left (encode world) = PartialObservers.fold next right (encode world) := by
  rw [witness.worldFactor.fold, witness.worldFactor.fold, relation]

theorem Witness.identity_action (witness : Witness step read worldStep project next view encode)
    (world : W) (action : A) (identity : worldStep world action = some world) :
    next (encode world) action = some (encode world) := by
  rw [witness.worldFactor.update, identity]
  rfl

theorem Witness.same_world_reads (witness : Witness step read worldStep project next view encode)
    (left right : S) (same : project left = project right) (query : I) :
    read left query = read right query := by
  rw [← witness.factor.observe, ← witness.factor.observe]
  simp only [Function.comp_apply, same]

theorem different_reads_obstruct_descent (left right : S)
    (same : project left = project right) (query : I) (different : read left query ≠ read right query) :
    ¬ Witness step read worldStep project next view encode := by
  intro witness
  exact different (witness.same_world_reads left right same query)

/-- A designated total world action gives divisibility only with actual world
closure and a proved phase increment. This does not identify an arbitrary
world action with a generator of its unit group or prove a census. -/
theorem Witness.clock_divides (witness : Witness step read worldStep project next view encode)
    (action : A) (advance : W → W) (phase : Q → Nat) (period clock : Nat)
    (total : ∀ world, worldStep world action = some (advance world))
    (increments : ∀ q q', next q action = some q' → phase q' = (phase q + 1) % period)
    (world : W) (origin : phase (encode world) = 0)
    (closed : ActionQuotient.iterate advance clock world = world) : period ∣ clock := by
  have commutes : ∀ w, phase (encode (advance w)) = (phase (encode w) + 1) % period := by
    intro w
    apply increments
    rw [witness.worldFactor.update, total]
    rfl
  exact ActionQuotient.cyclic_factor_divides_closed_clock advance (phase ∘ encode)
    period clock commutes world origin closed

end WorldDescent
