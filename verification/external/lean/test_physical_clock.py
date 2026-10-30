"""Clock model laws and false-claim controls; not a native execution receipt."""
import re
import unittest
from lean_test_fixture import IsolatedLeanTest

HEADER = '''import VerumExternalReplay.PhysicalClock
open PhysicalClock
set_option warningAsError true
deriving instance DecidableEq for Except
def maximum : Nat := 18446744073709551615
def initial : Clock Nat := ⟨7, some 0, 0, .ready⟩
def selected : Point Nat := ⟨7, ⟨0, 0⟩⟩
def pending := (start maximum initial selected).1
def tick : Receipt Nat := ⟨selected, some 11, .tick, some ⟨0, 1⟩⟩
def terminal : Receipt Nat := ⟨selected, some 11, .terminal .completed, some ⟨1, 0⟩⟩
def afterTick : Clock Nat := ⟨7, some 0, 1, .ready⟩
def afterTerminal : Clock Nat := ⟨7, some 0, 0, .terminalPending⟩
'''


class PhysicalClockTests(IsolatedLeanTest):
    MODULES = ('PhysicalClock',)

    def test_all_universal_laws_have_constructive_axioms(self):
        names = ['PhysicalClock.' + name for name in
                 re.findall(r'^theorem (\w+)', self.sources['PhysicalClock'], re.M)]
        self.assertEqual(len(names), 27)
        self.assert_axioms(self.prove(HEADER + '\n'.join('#print axioms ' + n for n in names)), names)

    def test_actual_clock_phase_and_receipt_contract(self):
        self.prove(HEADER + '''
example : point maximum initial = .ok selected := by decide_cbv
example : pending = ⟨7, some 0, 0, .inFlight⟩ := by decide_cbv
example : observe maximum selected (some 11) .tick = some tick := by decide_cbv
example : observe maximum selected (some 11) (.terminal .completed) = some terminal := by decide_cbv
example : commit pending tick = some (afterTick, .ok ()) := by decide_cbv
example : commit afterTick tick = some (afterTick, .error (.foreignPoint, tick)) := by decide_cbv
example : commit (start maximum afterTick ⟨7, ⟨0, 1⟩⟩).1 tick =
    some (⟨7, some 0, 1, .inFlight⟩, .error (.foreignPoint, tick)) := by decide_cbv
example : commit pending terminal = some (afterTerminal, .ok ()) := by decide_cbv
example : closeTerminal maximum afterTerminal = (⟨7, some 1, 0, .ready⟩, .ok ()) := by decide_cbv
example : restart maximum 8 pending = (pending, .error .pendingOutcome) := by decide_cbv
example : restart maximum 8 afterTerminal = (afterTerminal, .error .pendingTerminal) := by decide_cbv
example : restart maximum 8 afterTick = (⟨8, some 1, 0, .ready⟩, .ok ()) := by decide_cbv
example : commit { pending with origin := 8 } tick =
    some ({ pending with origin := 8 }, .error (.foreignPoint, tick)) := by decide_cbv
example : commit pending { tick with successor := none } = none := by decide_cbv
''')

    def test_full_word_boundaries_without_wrapping(self):
        self.prove(HEADER + '''
example : checkedNext maximum (maximum - 1) = some maximum := by rfl
example : checkedNext maximum maximum = none := by rfl
example : point maximum { initial with age := maximum } = .error .ageOverflow := by rfl
example : point maximum { initial with episode := none } = .error .missingEpisode := by rfl
example : point maximum { pending with age := maximum } = .error .pendingOutcome := by rfl
example : (closeTerminal maximum { afterTerminal with episode := some maximum }).1.episode = none := by rfl
example : (restart maximum 8 { initial with episode := some maximum }).1.episode = none := by rfl
example : (restart maximum 8 { initial with episode := none }).1.episode = none := by rfl
example : observe maximum ⟨7, ⟨maximum, 0⟩⟩ none (.terminal .ended) =
    some ⟨⟨7, ⟨maximum, 0⟩⟩, none, .terminal .ended, none⟩ := by rfl
''')

    def test_journal_precedence_and_untracked_guard(self):
        self.prove(HEADER + '''
example : lifecycleGuard none pending = .error .pendingPhysicalTime := by decide_cbv
example : lifecycleGuard (some ⟨false, true⟩) pending = .error .pendingPhysicalTime := by decide_cbv
example : lifecycleGuard (some ⟨false, false⟩) pending = .error .unknownDelivery := by decide_cbv
example : lifecycleGuard (some ⟨true, false⟩) pending = .error .pendingBoundary := by decide_cbv
example : lifecycleGuard none afterTerminal = .error .pendingTerminalClosure := by decide_cbv
example : lifecycleGuard (some ⟨false, true⟩) afterTick = .ok () := by decide_cbv
example : beginRun maximum 8 none pending ([7, 11] : List Nat) (fun _ => []) =
    ((pending, [7, 11]), .error .pendingPhysicalTime) := by decide_cbv
example : beginRun maximum 8 none afterTick ([7, 11] : List Nat) (fun _ => []) =
    ((⟨8, some 1, 0, .ready⟩, []), .ok ()) := by decide_cbv
''')

    def test_false_claims_rejected_semantically(self):
        claims = {
            'restart_erases_pending': '(restart maximum 8 pending).2 = .ok ()',
            'restart_erases_terminal': '(restart maximum 8 afterTerminal).2 = .ok ()',
            'journal_proves_ready': 'lifecycleGuard (some ⟨false, true⟩) pending = .ok ()',
            'untracked_proves_ready': 'lifecycleGuard none pending = .ok ()',
            'duplicate_tick': 'commit afterTick tick = some (⟨7,some 0,2,.ready⟩, .ok ())',
            'foreign_receipt_accepted': 'commit {pending with origin := 8} tick = some (afterTick,.ok ())',
            'terminal_closes_early': 'commit pending terminal = some (⟨7,some 1,0,.ready⟩,.ok ())',
            'word_bound_wraps': 'checkedNext maximum maximum = some 0',
            'restart_resurrects': '(restart maximum 8 {initial with episode := none}).1.episode = some 0',
            'malformed_tick_is_refusal': 'commit pending {tick with successor := none} = some (pending,.error (.ageOverflow,tick))',
        }
        for name, claim in claims.items():
            with self.subTest(claim=name):
                proof = 'rw [checked_next_at_limit]; decide_cbv' if name == 'word_bound_wraps' else 'decide_cbv'
                self.prove(HEADER + 'example : ' + claim + ' := by ' + proof, failure=True)


if __name__ == '__main__':
    unittest.main()
