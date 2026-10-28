"""Checked action/outcome separation laws; native correspondence is separate."""
from pathlib import Path
import re
import tempfile
import unittest

from check_typed_programs import ROOT, check, require_success, run

FALSE = 'Tactic `rfl` failed:'
HEADER = '''import VerumExternalReplay.ActionCapture
open AcquiredObserver ActionCapture
set_option warningAsError true
def before : Event := ⟨1, 2, none, 42, 42, 3, 4, 5, none, some ⟨[1], [3]⟩⟩
def request : Request UInt8 Nat := ⟨71, before, 7⟩
def outcome : Sensory := ⟨[1], [7, 0, 255]⟩
def birth : Sensory := ⟨[1], [9, 0, 255]⟩
def owned : Failure UInt8 String := ⟨.act 7, some .ended, .action, "restart failed"⟩
def failed : ActionCapture.Result UInt8 String := .error owned
def restarted : ActionCapture.Result UInt8 String := .ok (.terminal .ended .resumed)
def captured : Capture UInt8 String := ⟨7, failed, some outcome, none⟩
def terminalCapture : Capture UInt8 String := ⟨7, restarted, some outcome, some birth⟩
'''


class ActionCaptureTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.ActionCapture'))

    def prove(self, source, expected=None):
        with tempfile.TemporaryDirectory(prefix='verum-action-capture-') as directory:
            path = Path(directory)/'check.lean'
            path.write_text(HEADER + source)
            return check(path, expected)

    def test_universal_axiom_roster(self):
        names = re.findall(r'^theorem (\w+)', (ROOT/'VerumExternalReplay/ActionCapture.lean').read_text(), re.M)
        for required in ('check_success_iff', 'error_target_requires_exact_action',
                         'resumed_requires_successful_restart', 'no_record_no_target'):
            self.assertIn(required, names)
        result = self.prove('\n'.join('#print axioms ActionCapture.'+name for name in names))
        rosters = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
        self.assertEqual(len(rosters), len(names))
        for row in rosters:
            self.assertTrue(row.endswith(('does not depend on any axioms',
                'depends on axioms: [propext]', 'depends on axioms: [propext, Quot.sound]')), row)

    def test_independent_presence_and_exact_outcome_bytes(self):
        self.prove('''
example : check terminalCapture = .ok terminalCapture := by rfl
example : targetObservation terminalCapture = some outcome := by rfl
example : terminalCapture.resumed = some birth := by rfl
example : check {terminalCapture with observation := none} =
    .ok {terminalCapture with observation := none} := by rfl
example : targetObservation {terminalCapture with observation := none} = none := by rfl
example : check {terminalCapture with resumed := none} =
    .ok {terminalCapture with resumed := none} := by rfl
example : check {terminalCapture with observation := some ⟨[], []⟩} =
    .ok {terminalCapture with observation := some ⟨[], []⟩} := by rfl
example : targetObservation {terminalCapture with observation := some ⟨[], []⟩} = some ⟨[], []⟩ := by rfl
example : check (unavailable (7 : UInt8) restarted) = .ok (unavailable 7 restarted) := by rfl
''')

    def test_failure_requires_own_action_terminal_and_keeps_original_error(self):
        self.prove('''
example : check captured = .ok captured := by rfl
example : captured.result = .error owned := by rfl
example : targetObservation captured = some outcome := by rfl
example : check {captured with resumed := some birth} = .error .unexpectedResumption := by rfl
example : check {captured with result := .error {owned with scope := .episode}} =
    .error .unownedObservation := by rfl
example : check {captured with result := .error {owned with operation := .act 8}} =
    .error .unownedObservation := by rfl
example : check {captured with result := .error {owned with terminal := none}} =
    .error .unownedObservation := by rfl
example : check {captured with result := .error {owned with scope := .episode}, resumed := some birth} =
    .error .unownedObservation := by rfl
''')
        for operation in ('begin', 'checkpoint', 'restore'):
            self.prove('example : check {captured with result := .error {owned with operation := .' +
                       operation + '}} = .error .unownedObservation := by rfl')

    def test_resumption_requires_actual_successful_terminal(self):
        for result in ('.ok .step', '.ok .refused', '.ok (.terminal .ended .stopped)', 'failed'):
            self.prove(f'''example : check (Detail := String) {{terminalCapture with result := {result}}} =
                .error .unexpectedResumption := by rfl''')

    def test_legacy_terminal_and_failure_never_reconstruct_targets(self):
        self.prove('''
example : targetObservation (legacy (7 : UInt8) restarted (some birth)) = none := by rfl
example : (legacy (7 : UInt8) restarted (some birth)).resumed = some birth := by rfl
example : legacy (7 : UInt8) failed (some birth) = unavailable 7 failed := by rfl
example : targetObservation (legacy (Detail := String) (7 : UInt8) (.ok .step) (some outcome)) = some outcome := by rfl
example : targetObservation (legacy (Detail := String) (7 : UInt8) (.ok .refused) (some outcome)) = some outcome := by rfl
example : targetObservation (legacy (Detail := String) (7 : UInt8) (.ok (.terminal .ended .stopped)) (some birth)) = none := by rfl
example : (legacy (Detail := String) (7 : UInt8) (.ok (.terminal .ended .stopped)) (some birth)).resumed = none := by rfl
''')

    def test_dispatch_cut_and_prequery_remain_frozen(self):
        self.prove('''
example : attach request captured = .ok ⟨request, captured⟩ := by rfl
example : attach request {terminalCapture with action := 8} = .error .foreignAction := by rfl
example : (preActionQuery id request).action = some 7 := by rfl
example : (preActionQuery id request).sensory = some ⟨[1], [3]⟩ := by rfl
example : (Retained.mk request captured).request.cut = 71 := by rfl
example : preActionQuery id (Retained.mk request captured).request =
    preActionQuery id (Retained.mk request terminalCapture).request := by rfl
''')

    def test_new_invocation_clears_sticky_bytes_even_for_same_action(self):
        self.prove('''
example : finish (begin request (some outcome)) failed none = .ok (unavailable 7 failed) := by rfl
example : finish (begin request (some outcome)) restarted (some birth) =
    .ok ⟨7, restarted, none, some birth⟩ := by rfl
example : finish (record (begin request (some birth)) (some outcome)) failed none = .ok captured := by rfl
example : (record (begin request (some outcome)) none).observation = none := by rfl
-- Without the begin transition, the constructor cannot establish freshness.
example : finish (⟨request, some outcome⟩ : Slot UInt8 Nat) failed none = .ok captured := by rfl
''')

    def test_semantic_target_keeps_observed_kind_and_never_uses_birth(self):
        self.prove('''
example : semanticTarget terminalCapture = .ok ⟨.terminal .ended, outcome⟩ := by rfl
example : semanticTarget captured = .ok ⟨.terminal .ended, outcome⟩ := by rfl
example : semanticTarget {terminalCapture with observation := none} = .error .missing := by rfl
example : semanticTarget (unavailable (7 : UInt8) failed) = .error .missing := by rfl
example : semanticTarget (unavailable (8 : UInt8) failed) = .error .unobserved := by rfl
example : semanticTarget (Detail := String) {terminalCapture with result := .ok .refused, resumed := none} =
    .ok ⟨.refused, outcome⟩ := by rfl
''')

    def test_successor_guard_does_not_fill_unknown_data(self):
        self.prove('''
example : successor terminalCapture = some birth := by rfl
example : successor captured = none := by rfl
example : checkSuccessor terminalCapture (some birth) = .ok () := by rfl
example : checkSuccessor terminalCapture (some outcome) = .error .contradictorySuccessor := by rfl
example : checkSuccessor terminalCapture none = .ok () := by rfl
example : checkSuccessor captured (some birth) = .ok () := by rfl
example : successor (Detail := String) {terminalCapture with result := .ok .step, resumed := none} = some outcome := by rfl
''')

    def test_false_substitution_ownership_and_reset_claims_are_rejected(self):
        claims = [
            'targetObservation terminalCapture = some birth',
            'targetObservation {terminalCapture with observation := none} = some birth',
            'targetObservation {terminalCapture with observation := some ⟨[], []⟩} = none',
            'captured.result = restarted',
            'check {captured with result := .error {owned with scope := .episode}} = .ok captured',
            'check {captured with result := .error {owned with operation := .act 8}} = .ok captured',
            'check {captured with result := .error {owned with terminal := none}} = .ok captured',
            'check {captured with resumed := some birth} = .ok {captured with resumed := some birth}',
            'check {captured with result := .error {owned with scope := .episode}, resumed := some birth} = .error .unexpectedResumption',
            'targetObservation (legacy (7 : UInt8) restarted (some birth)) = some birth',
            'targetObservation (legacy (7 : UInt8) failed (some outcome)) = some outcome',
            'attach request {terminalCapture with action := 8} = .ok ⟨request, {terminalCapture with action := 8}⟩',
            '(preActionQuery id request).sensory = some outcome',
            '(Retained.mk request terminalCapture).request.cut = 72',
            'finish (begin request (some outcome)) failed none = .ok captured',
            'semanticTarget terminalCapture = .ok ⟨.terminal .ended, birth⟩',
            'semanticTarget {terminalCapture with observation := none} = .ok ⟨.terminal .ended, birth⟩',
            'semanticTarget captured = .ok ⟨.refused, outcome⟩',
            'successor captured = some outcome',
            'checkSuccessor terminalCapture (some outcome) = .ok ()',
            'successor captured = some birth',
        ]
        for claim in claims:
            with self.subTest(claim=claim):
                self.prove(f'example : {claim} := by rfl', FALSE)


if __name__ == '__main__':
    unittest.main()
