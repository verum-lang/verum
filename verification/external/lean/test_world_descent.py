"""World factors require exact source coupling and all action relations."""
import re
import unittest
from lean_test_fixture import IsolatedLeanTest

HEADER = '''import VerumExternalReplay.WorldDescent
open WorldDescent
set_option warningAsError true
def worldStep (world action : Bool) : Option Bool := some (if action then !world else world)
def capture (context : Bool × Nat) (action : Bool) : Option ((Bool × Nat) × Bool) :=
  some ((if action then !context.1 else context.1, context.2 + 1), action)
def update (_ : Unit) (_ : Bool) : Option Unit := some ()
def nativeStep := coupledStep capture update
def nativeRead := coupledRead (fun (context : Bool × Nat) (_ : Unit) => context.1)
  (fun (_ : Unit) (query : Bool) => some query)
def project (state : (Bool × Nat) × Unit) : Bool := state.1.1
def view (state : Bool) (_ : Unit) : Option Bool := some state
theorem witnessed : Witness nativeStep nativeRead worldStep project worldStep view id := by
  constructor
  · intro world; exact ⟨((world,0),()),rfl⟩
  · intros; rfl
  · constructor <;> intros <;> rfl
'''


class WorldDescentTests(IsolatedLeanTest):
    MODULES = ('WorldDescent',)
    MEMORY_MIB = 1536

    def test_constructive_axioms(self):
        names = ['WorldDescent.' + n for n in re.findall(r'^theorem ([\w.]+)', self.sources['WorldDescent'], re.M)]
        self.assertEqual(len(names), 6)
        self.assert_axioms(self.prove(HEADER + '\n'.join('#print axioms ' + n for n in names)), names)

    def test_input_dependent_observer_preserves_all_world_relations(self):
        self.prove(HEADER + '''
example (world : Bool) : PartialObservers.fold worldStep [true,true] world = some world :=
  witnessed.world_relations world [true,true] [] (by cases world <;> rfl)
example (world : Bool) : worldStep world false = some world :=
  witnessed.identity_action world false rfl
example (left right : (Bool × Nat) × Unit) (same : project left = project right) :
    nativeRead left () = nativeRead right () := witnessed.same_world_reads left right same ()
def phase (world : Bool) : Nat := if world then 1 else 0
example : (2 : Nat) ∣ 6 := witnessed.clock_divides true Bool.not phase 2 6
  (fun _ => rfl)
  (by intro q q' h; cases q <;> cases q' <;> simp_all [worldStep,phase])
  false rfl (by decide)
''')

    def test_query_age_is_an_obstruction_even_with_zero_update(self):
        self.prove(HEADER + '''
def ageCapture (age : Nat) (_ : Unit) : Option (Nat × Unit) := some (age+1,())
def zeroUpdate (_ : Unit) (_ : Unit) : Option Unit := some ()
def ageStep := coupledStep ageCapture zeroUpdate
def ageRead := coupledRead (fun (age : Nat) (_ : Unit) => age)
  (fun (_ : Unit) (query : Nat) => some query)
def erase (_ : Nat × Unit) : Unit := ()
def stay (_ : Unit) (_ : Unit) : Option Unit := some ()
example (next : Q → Unit → Option Q) (view : Q → Unit → Option Nat) (encode : Unit → Q) :
    ¬ Witness ageStep ageRead stay erase next view encode :=
  different_reads_obstruct_descent (0,()) (1,()) rfl () (by decide)
''')

    def test_wrong_identity_reset_and_missing_generator_are_rejected(self):
        for claim in (
            'worldStep false false = some true',
            'worldStep true false = some false',
            'worldStep false true = some false',
            'PartialObservers.fold worldStep [true] false = PartialObservers.fold worldStep [] false',
        ):
            with self.subTest(claim=claim):
                self.prove(HEADER + 'example : ' + claim + ' := by decide_cbv', failure=True)


if __name__ == '__main__':
    unittest.main()
