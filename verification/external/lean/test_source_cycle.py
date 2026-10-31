"""Physical dispatch pairing never turns a relative phase into world closure."""
import re
import unittest
from lean_test_fixture import IsolatedLeanTest

HEADER = '''import VerumExternalReplay.SourceCycle
open SourceCycle ObserverOwnership
set_option warningAsError true
def cycle : Cycle Nat Nat Nat Nat where
  period := 3
  positive := by decide
  inside := fun n => n < 3
  phase := id
  bounded := by intro n h; exact h
  update := fun n _ => some ((n+1)%3)
  read := fun n _ => some n
  view := some
  advances := by
    intro n event next _ updated
    cases updated
    exact ⟨Nat.mod_lt _ (by decide),rfl⟩
  observes := by intros; rfl
abbrev State := Nat × Nat
def value : State → Nat := Prod.snd
def cut : Cut Nat Nat State := ⟨11,7,(100,1)⟩
def nextCut : Cut Nat Nat State := ⟨11,7,(101,2)⟩
def point : PhysicalClock.Point Nat := ⟨5,9,20⟩
def clock : PhysicalClock.Clock Nat := ⟨5,some 9,20,.ready⟩
def afterTick : PhysicalClock.Clock Nat := ⟨5,some 9,21,.ready⟩
def afterTerminal : PhysicalClock.Clock Nat := ⟨5,some 9,0,.terminalPending⟩
def binding := anchor point cut
def tick : Pair Nat Nat Nat State Nat Nat :=
  ⟨18,18,0,⟨point,some 40,.tick,some ⟨9,21⟩⟩,⟨cut,nextCut⟩⟩
def terminal : Pair Nat Nat Nat State Nat Nat :=
  {tick with physical := ⟨point,some 40,.terminal .death,some ⟨10,0⟩⟩}
abbrev pairedTick := Paired cycle value 100 clock afterTick binding 18 0 tick
def queryAgeRead (_state query : Nat) : Option Nat := some query
'''


class SourceCycleTests(IsolatedLeanTest):
    MODULES = ('SourceCycle',)

    def test_constructive_axioms(self):
        names = ['SourceCycle.'+name for name in re.findall(r'^theorem (\w+)', self.sources['SourceCycle'], re.M)]
        self.assertEqual(len(names), 13)
        self.assert_axioms(self.prove(HEADER+'\n'.join('#print axioms '+n for n in names)),names)

    def test_paired_tick_and_relative_anchor(self):
        self.prove(HEADER+'''
example : pairedTick := by decide_cbv
example : Aligned cycle value binding := anchor_aligned cycle value point cut (by change 1 < 3; decide)
example : (follow binding tick).binding = some (tickBinding binding tick.observer) := by rfl
example : (tickBinding binding tick.observer).point.time.age = 21 ∧
    (tickBinding binding tick.observer).current.state = (101,2) := ⟨rfl,rfl⟩
example : Aligned cycle value (tickBinding binding tick.observer) ∧ afterTick.age = 21 := by
  have h := tick_preserves_binding cycle value 100 clock afterTick binding 18 0 tick
    (anchor_aligned cycle value point cut (by change 1 < 3; decide)) (by decide_cbv) rfl
  exact ⟨h.2.1,h.2.2.1⟩
''')

    def test_consecutive_paired_ticks_wrap_phase_without_resetting_age(self):
        self.prove(HEADER+'''
def secondCut : Cut Nat Nat State := ⟨11,7,(102,0)⟩
def second : Pair Nat Nat Nat State Nat Nat :=
  ⟨19,19,0,⟨⟨5,9,21⟩,some 41,.tick,some ⟨9,22⟩⟩,⟨nextCut,secondCut⟩⟩
def afterSecond : PhysicalClock.Clock Nat := ⟨5,some 9,22,.ready⟩
def secondBinding := tickBinding (tickBinding binding tick.observer) second.observer
example : Aligned cycle value secondBinding := by
  have first := tick_preserves_binding cycle value 100 clock afterTick binding 18 0 tick
    (anchor_aligned cycle value point cut (by change 1 < 3; decide)) (by decide_cbv) rfl
  exact (tick_preserves_binding cycle value 100 afterTick afterSecond
    (tickBinding binding tick.observer) 19 0 second first.2.1 (by decide_cbv) rfl).2.1
example : secondBinding.point.time.age = 22 ∧ secondBinding.ticks = 2 ∧
    cycle.phase (value secondBinding.current.state) = 0 := ⟨rfl,rfl,rfl⟩
example (query : Nat) : cycle.read (value secondBinding.current.state) query = some 0 := rfl
''')

    def test_terminal_and_restart_keep_real_observer_state(self):
        self.prove(HEADER+'''
example : Paired cycle value 100 clock afterTerminal binding 18 0 terminal := by decide_cbv
example : follow binding terminal = ⟨nextCut,none⟩ := terminal_keeps_update binding terminal .death rfl
example : afterTerminal.age = 0 ∧ afterTerminal.episode = clock.episode ∧
    afterTerminal.phase = .terminalPending :=
  terminal_clock_correspondence cycle value 100 clock afterTerminal binding 18 0 terminal
    (by decide_cbv) .death rfl
example : beginRun 100 6 none afterTick nextCut (some binding) =
    ((⟨6,some 10,0,.ready⟩,(nextCut,none)),.ok ()) := by rfl
example : beginRun 100 6 none afterTerminal nextCut none =
    ((afterTerminal,(nextCut,none)),.error .pendingTerminalClosure) := by rfl
example : cycle.phase (value (anchor ⟨6,10,0⟩ nextCut).current.state) = 2 := by rfl
''')

    def test_query_age_cannot_supply_autonomy(self):
        self.prove(HEADER+'''
example (candidate : Cycle Nat Nat Nat Nat) (inside : candidate.inside 0)
    (queryAge : candidate.read = fun _ query => some query) : False := by
  have same := query_independent candidate 0 inside 0 1
  rw [queryAge] at same
  cases same
example : PhysicalClock.point 100 {clock with phase := .inFlight} = .error .pendingOutcome := by rfl
example : anchorAt 20 clock cut = .error .ageOverflow := by rfl
''')

    def test_false_pairing_reset_and_autonomy_claims_rejected(self):
        claims = {
            'foreign_dispatch': 'Paired cycle value 100 clock afterTick binding 18 0 {tick with observerDispatch := 19}',
            'wrong_event': 'Paired cycle value 100 clock afterTick binding 18 1 tick',
            'forged_successor': 'Paired cycle value 100 clock afterTick binding 18 0 {tick with physical := {tick.physical with successor := some ⟨9,22⟩}}',
            'foreign_origin': 'Paired cycle value 100 {clock with origin := 6} afterTick binding 18 0 tick',
            'stale_state_same_value': 'Paired cycle value 100 clock afterTick binding 18 0 {tick with observer := {tick.observer with before := {cut with state := (99,1)}}}',
            'foreign_instance': 'Paired cycle value 100 clock afterTick binding 18 0 {tick with observer := {tick.observer with after := {nextCut with owner := 8}}}',
            'invented_update': 'Paired cycle value 100 clock afterTick binding 18 0 {tick with observer := {tick.observer with after := {nextCut with state := (101,0)}}}',
            'uncommitted_clock': 'Paired cycle value 100 clock clock binding 18 0 tick',
            'terminal_is_tick': 'Paired cycle value 100 clock afterTick binding 18 0 terminal',
            'terminal_resets_U': '(follow binding terminal).current.state = (100,0)',
            'terminal_retains_anchor': '(follow binding terminal).binding.isSome = true',
            'restart_resets_U': '(beginRun 100 6 none afterTick nextCut (some binding)).1.2.1.state = (100,0)',
            'reanchor_invents_phase_zero': 'cycle.phase (value (anchor ⟨6,10,0⟩ nextCut).current.state) = 0',
            'anchor_implies_age_phase': 'cycle.phase (value binding.current.state) = binding.point.time.age % cycle.period',
            'query_age_autonomous': 'queryAgeRead 0 0 = queryAgeRead 0 1',
        }
        for name,claim in claims.items():
            with self.subTest(claim=name):
                self.prove(HEADER+'example : '+claim+' := by decide_cbv',failure=True)


if __name__=='__main__':
    unittest.main()
