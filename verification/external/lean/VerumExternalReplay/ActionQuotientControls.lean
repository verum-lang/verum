import VerumExternalReplay.ActionQuotient

namespace ActionQuotientControls
open ActionQuotient
set_option warningAsError true

/-- Infinite concrete states and infinitely many action values; the quotient
law is universal, not inferred by enumerating a prefix of the counter. -/
def counterStep (state action : Nat) : Option Nat := some (state + action)
def counterRead (period state : Nat) (_ : Unit) : Option Nat := some (state % period)
def residue (period : Nat) (positive : 0 < period) (state : Nat) : Fin period :=
  ⟨state % period, Nat.mod_lt _ positive⟩
def residueStep (period : Nat) (positive : 0 < period) (state : Fin period)
    (action : Nat) : Option (Fin period) := some (residue period positive (state.val + action))
def residueRead (state : Fin period) (_ : Unit) : Option Nat := some state.val

theorem counter_factor (period : Nat) (positive : 0 < period) :
    Factor counterStep (counterRead period) (residueStep period positive)
      residueRead (residue period positive) := by
  constructor
  · intro state action
    apply congrArg some
    apply Fin.ext
    exact Nat.mod_add_mod _ _ _
  · intro state query
    rfl

theorem residue_separating (period : Nat) (positive : 0 < period) :
    Separating (residueStep period positive) residueRead := by
  apply separating_of_reads
  intro q r reads
  exact Fin.ext (Option.some.inj (reads ()))

theorem counter_behavioral_classes (period : Nat) (positive : 0 < period) (s t : Nat) :
    Equivalent counterStep (counterRead period) s t ↔
      residue period positive s = residue period positive t :=
  (counter_factor period positive).exact_behavioral_classes (residue_separating period positive)

example (period : Nat) (positive : 0 < period) (actions : List Nat) (state : Nat) :
    observation (residueStep period positive) residueRead actions (residue period positive state) () =
      observation counterStep (counterRead period) actions state () :=
  (counter_factor period positive).observations actions state ()

example : Equivalent counterStep (counterRead 5) 2 7 :=
  (counter_factor 5 (by decide)).kernel_equivalent (by decide)

-- Distinct current readings alone refute the proposed quotient.
example : ¬ Equivalent counterStep (counterRead 5) 2 3 := by
  intro same
  have impossible := (same []).2 ()
  contradiction

def toggle (state action : Bool) : Option Bool := some (if action then !state else state)
def echoState (state : Bool) (_ : Unit) : Option Bool := some state

theorem checked_boolean_factor : Factor toggle echoState toggle echoState id := by
  apply check_sound [false,true] [false,true] [()]
  · intro state; cases state <;> simp
  · intro action; cases action <;> simp
  · intro query; cases query; simp
  · decide_cbv

example : Factor toggle echoState
    (quotientStep toggle id id) (quotientRead echoState id) id :=
  factor_of_congruence id (fun _ => rfl)
    (fun _ _ same _ => by cases same; rfl) (fun _ _ same _ => by cases same; rfl)

example : Factor toggle echoState toggle echoState (id ∘ id) :=
  checked_boolean_factor.compose checked_boolean_factor

-- Checking every STATE but just one action does not cover the action domain.
def ignoreAction (state : Bool) (_ : Bool) : Option Bool := some state
example : check [false,true] [false] [()] toggle echoState ignoreAction echoState id = true := by
  decide_cbv
example : check [false,true] [false,true] [()] toggle echoState ignoreAction echoState id = false := by
  decide_cbv

-- Query coverage is an independent obligation.
def stay (_ : Unit) (_ : Unit) : Option Unit := some ()
def echoQuery (_ : Unit) (query : Bool) : Option Bool := some query
def alwaysFalse (_ : Unit) (_ : Bool) : Option Bool := some false
example : check [()] [()] [false] stay echoQuery stay alwaysFalse id = true := by decide_cbv
example : check [()] [()] [false,true] stay echoQuery stay alwaysFalse id = false := by decide_cbv

-- An external-age read has a finite STATE factor, with an unbounded query and
-- output alphabet. That certificate is not an autonomous finite clock.
def externalAge (_ : Nat) (query : Nat) : Option Nat := some query
def externalAgeView (_ : Unit) (query : Nat) : Option Nat := some query
def collapsedStep (_ : Unit) (_ : Nat) : Option Unit := some ()
theorem external_age_state_factor :
    Factor counterStep externalAge collapsedStep externalAgeView (fun _ => ()) := by
  constructor <;> intros <;> rfl

theorem external_age_not_autonomous :
    ¬ ∃ stateRead : Nat → Option Nat, ∀ state query, stateRead state = externalAge state query :=
  query_difference_refuses_autonomous_read 0 0 1 (by decide)

theorem external_age_unbounded (state bound : Nat) :
    ∃ query value, externalAge state query = some value ∧ bound < value :=
  ⟨bound + 1, bound + 1, rfl, Nat.lt_succ_self _⟩

-- No finite prefix, however long, establishes universal future agreement.
def delayed (horizon time : Nat) : Nat := if time ≤ horizon then 0 else 1
theorem finite_prefix_is_insufficient (horizon : Nat) :
    ∃ f g : Nat → Nat, (∀ time, time ≤ horizon → f time = g time) ∧
      f (horizon + 1) ≠ g (horizon + 1) := by
  refine ⟨fun _ => 0, delayed horizon, ?_, ?_⟩
  · intro time within
    simp [delayed, within]
  · simp [delayed]

def delayedRead (horizon state : Nat) (_ : Unit) : Option Nat := some (delayed horizon state)
def zeroRead (_ : Unit) (_ : Unit) : Option Nat := some 0
example : check [0,1,2,3] [1] [()] counterStep (delayedRead 3)
    collapsedStep zeroRead (fun _ => ()) = true := by decide_cbv
example : check [0,1,2,3,4] [1] [()] counterStep (delayedRead 3)
    collapsedStep zeroRead (fun _ => ()) = false := by decide_cbv

-- Read equality cannot erase the action domain (some versus none).
def partialStep (state : Bool) (_ : Unit) : Option Bool := if state then some state else none
def noReading (_ : Bool) (_ : Unit) : Option Bool := none
example : observation partialStep noReading [()] false () =
    observation partialStep noReading [()] true () := by decide_cbv
example : ¬ Equivalent partialStep noReading false true := by
  intro same
  have impossible := (same [()]).1
  contradiction

-- A concrete clock closing and the exact phase action are both needed.
def rotation (period state : Nat) : Nat := (state + 1) % period
example (period clock : Nat) (closed : iterate (rotation period) clock 0 = 0) : period ∣ clock :=
  cyclic_factor_divides_closed_clock (rotation period) id period clock (fun _ => rfl) 0 rfl closed
example : iterate Nat.succ 4 0 = 4 := by decide_cbv
example : ¬ (3 : Nat) ∣ 2 := by decide
example : ¬ (2 : Nat) ∣ 1 := by decide

-- Even a fully observed finite world can have several group-action orbits.
-- Its state count is not the order of a cyclic factor or of its unit group.
def twoOrbitClock : Option Bool → Option Bool
  | none => none
  | some state => some (!state)
example (state : Option Bool) : iterate twoOrbitClock 2 state = state := by
  cases state with
  | none => rfl
  | some state => cases state <;> rfl
example : twoOrbitClock (some false) ≠ some false := by decide_cbv
example : [none, some false, some true].length = 3 ∧ ¬ (3 : Nat) ∣ 2 := by decide_cbv

-- Required refusal controls: no favorable metadata label makes these true.
/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : check [false,true] [false,true] [()] toggle echoState ignoreAction echoState id = true := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : check [()] [()] [false,true] stay echoQuery stay alwaysFalse id = true := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : check [0,1,2,3,4] [1] [()] counterStep (delayedRead 3)
    collapsedStep zeroRead (fun _ => ()) = true := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : externalAge 0 0 = externalAge 0 1 := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : (partialStep false ()).isSome = (partialStep true ()).isSome := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : iterate Nat.succ 4 0 = 0 := by decide_cbv

/-- error: `decide_cbv` failed: the proposition evaluates to `false` -/
#guard_msgs in
example : (3 : Nat) ∣ 2 := by decide_cbv

-- Reject hidden oracle, admission or project-specific axioms in every named law.
/-- info: 'ActionQuotient.Factor.fold' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ActionQuotient.Factor.fold

/-- info: 'ActionQuotient.Factor.definedness' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ActionQuotient.Factor.definedness

/-- info: 'ActionQuotient.Factor.observations' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ActionQuotient.Factor.observations

/-- info: 'ActionQuotient.Factor.kernel_congruence' does not depend on any axioms -/
#guard_msgs in
#print axioms ActionQuotient.Factor.kernel_congruence

/-- info: 'ActionQuotient.Factor.kernel_equivalent' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ActionQuotient.Factor.kernel_equivalent

/-- info: 'ActionQuotient.Factor.equivalent_iff' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ActionQuotient.Factor.equivalent_iff

/-- info: 'ActionQuotient.Factor.exact_behavioral_classes' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ActionQuotient.Factor.exact_behavioral_classes

/-- info: 'ActionQuotient.separating_of_reads' does not depend on any axioms -/
#guard_msgs in
#print axioms ActionQuotient.separating_of_reads

/-- info: 'ActionQuotient.query_difference_refuses_autonomous_read' does not depend on any axioms -/
#guard_msgs in
#print axioms ActionQuotient.query_difference_refuses_autonomous_read

/-- info: 'ActionQuotient.Factor.compose' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ActionQuotient.Factor.compose

/-- info: 'ActionQuotient.factor_of_congruence' does not depend on any axioms -/
#guard_msgs in
#print axioms ActionQuotient.factor_of_congruence

/-- info: 'ActionQuotient.check_sound' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ActionQuotient.check_sound

/-- info: 'ActionQuotient.check_complete' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ActionQuotient.check_complete

/-- info: 'ActionQuotient.cyclic_factor_iterate' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ActionQuotient.cyclic_factor_iterate

/-- info: 'ActionQuotient.cyclic_factor_reaches_phases' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ActionQuotient.cyclic_factor_reaches_phases

/-- info: 'ActionQuotient.cyclic_factor_divides_closed_clock' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ActionQuotient.cyclic_factor_divides_closed_clock

/-- info: 'ActionQuotient.cyclic_factor_divides_multiple' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ActionQuotient.cyclic_factor_divides_multiple

/-- info: 'ActionQuotientControls.counter_factor' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ActionQuotientControls.counter_factor

/-- info: 'ActionQuotientControls.residue_separating' does not depend on any axioms -/
#guard_msgs in
#print axioms ActionQuotientControls.residue_separating

/-- info: 'ActionQuotientControls.counter_behavioral_classes' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ActionQuotientControls.counter_behavioral_classes

/-- info: 'ActionQuotientControls.checked_boolean_factor' depends on axioms: [propext, Quot.sound] -/
#guard_msgs in
#print axioms ActionQuotientControls.checked_boolean_factor

/-- info: 'ActionQuotientControls.external_age_state_factor' does not depend on any axioms -/
#guard_msgs in
#print axioms ActionQuotientControls.external_age_state_factor

/-- info: 'ActionQuotientControls.external_age_not_autonomous' does not depend on any axioms -/
#guard_msgs in
#print axioms ActionQuotientControls.external_age_not_autonomous

/-- info: 'ActionQuotientControls.external_age_unbounded' does not depend on any axioms -/
#guard_msgs in
#print axioms ActionQuotientControls.external_age_unbounded

/-- info: 'ActionQuotientControls.finite_prefix_is_insufficient' depends on axioms: [propext] -/
#guard_msgs in
#print axioms ActionQuotientControls.finite_prefix_is_insufficient

end ActionQuotientControls
