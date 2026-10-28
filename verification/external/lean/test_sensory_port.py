"""Immutable sensory laws and strict controls; native replay has its own adapter."""
from pathlib import Path
import re
import tempfile
import unittest

import check_lifecycle_export as lifecycle
from check_typed_programs import ROOT, check, require_success, run
from test_lifecycle_export import model_fixture, event_value, pose_read_fixture

FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'
HEADER = '''import VerumExternalReplay.SensoryPort
open AcquiredObserver SensoryPort
set_option warningAsError true
deriving instance DecidableEq for Except
def captured : Event := ⟨1, 2, none, 42, 42, 3, 4, 5, none, some ⟨[1, 0], [7, 255]⟩⟩
def later : Event := {captured with sensory := some ⟨[1, 0], [255, 7]⟩}
def absent : Event := {captured with sensory := none}
def reader : FiniteCode.Code Op Value :=
  ⟨[⟨.primitive .eventSensory, [.input 0], none⟩], some 0, []⟩
def binding : ObserverAdmission.Binding Source := ⟨eventSource captured, 1, [.external 0]⟩
def remember (state : List Value) (event : Event) : Option (List Value) :=
  ((apply 0 .eventSensory [.event event]).toOption).map (fun value => state ++ [value])
'''


class SensoryPortTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.SensoryPort'))

    def prove(self, source, expected=None):
        with tempfile.TemporaryDirectory(prefix='verum-sensory-port-') as directory:
            path = Path(directory)/'check.lean'; path.write_text(HEADER + source)
            return check(path, expected)

    def test_universal_axiom_roster(self):
        names = re.findall(r'^theorem (\w+)', (ROOT/'VerumExternalReplay/SensoryPort.lean').read_text(), re.M)
        result = self.prove('\n'.join('#print axioms SensoryPort.'+n for n in names))
        rosters = [r['data'] for r in result['diagnostics'] if r['severity'] == 'information']
        self.assertEqual(len(rosters), len(names))
        for row in rosters:
            self.assertTrue(row.endswith(('does not depend on any axioms',
                'depends on axioms: [propext]', 'depends on axioms: [propext, Quot.sound]')), row)

    def test_exact_bytes_and_opaque_schema(self):
        self.prove('''
example : apply 0 .eventSensory [.event captured] = .ok (.pair (.bytes [1, 0]) (.bytes [7, 255])) := by decide_cbv
example : apply 0 .querySensory [.query (queryAt captured (some 9))] = .ok (.pair (.bytes [1, 0]) (.bytes [7, 255])) := by decide_cbv
example : apply 0 .length [.bytes [7, 255]] = .ok (.nat 2) := by decide_cbv
example : apply 0 .at [.bytes [7, 255], .nat 1] = .ok (.nat 255) := by decide_cbv
example : apply 0 .at [.bytes [7, 255], .nat 2] = .error .missing := by decide_cbv
example : apply 0 .eventSensory [] = .error .arity := by decide_cbv
example : apply 0 .eventSensory [.bytes []] = .error .sort := by decide_cbv
example : eventSource captured = eventSource later := by decide_cbv
example : apply 0 .equal [.event captured, .event later] = .ok (.bool false) := by decide_cbv
''')

    def test_binding_cannot_create_a_missing_payload(self):
        self.prove('''
example : run 0 reader binding .unit (eventSource captured) [.event captured] =
    some (.pair (.bytes [1, 0]) (.bytes [7, 255])) := by decide_cbv
example : run 0 reader binding .unit (eventSource absent) [.event absent] = none := by decide_cbv
example : run 0 reader binding .unit (eventSource captured) [.event absent] = none := by decide_cbv
example : (eventSource {absent with sensory := some ⟨[], []⟩}).sensory = true := by decide_cbv
example : apply 0 .eventSensory [.event {absent with sensory := some ⟨[], []⟩}] =
    .ok (.pair (.bytes []) (.bytes [])) := by decide_cbv
example : apply 0 .querySensory [.query (queryAt absent none)] = .error .missing := by decide_cbv
''')

    def test_install_prefix_and_live_replay_preserve_the_capture(self):
        self.prove('''
example : ObserverInstallation.install remember [] [history 7 captured] (history 7 later) 1 = none := by decide_cbv
example : ObserverInstallation.install remember [] [] (history 7 captured) 1 =
    some [.pair (.bytes [1, 0]) (.bytes [7, 255])] := by decide_cbv
example : retainedPort ([captured] ++ [later]) 0 = captured.sensory := by decide_cbv
example : PartialObservers.online remember [captured, later] (some []) =
    some [.pair (.bytes [1, 0]) (.bytes [7, 255]), .pair (.bytes [1, 0]) (.bytes [255, 7])] := by decide_cbv
example : PartialObservers.fold remember [captured, later] [] =
    PartialObservers.online remember [captured, later] (some []) := (live_replay _ _ _).symm
''')

    def test_changed_or_late_observations_are_rejected(self):
        claims = [
            'captured = later',
            'sensoryValue ⟨[0], [7]⟩ = sensoryValue ⟨[1], [7]⟩',
            '(some (⟨[], []⟩ : Sensory)) = none',
            'eventSource absent = eventSource captured',
            'retainedPort [captured, later] 0 = later.sensory',
            'apply 0 .querySensory [.query (queryAt captured none)] = .ok (.pair (.bytes [1, 0]) (.bytes [255, 7]))',
            'ObserverInstallation.install remember [] [history 7 captured] (history 7 later) 1 = some []',
            'PartialObservers.fold remember [captured, later] [] = PartialObservers.fold remember [captured] []',
        ]
        for claim in claims:
            with self.subTest(claim=claim): self.prove(f'example : {claim} := by decide_cbv', FALSE)

    def test_legacy_adapter_refuses_sensory_extensions(self):
        data = model_fixture()
        data['definitions'][0]['semantics']['kind'] = 'AcquiredV3'
        with self.assertRaisesRegex(ValueError, 'Unsupported semantic adapter'): lifecycle.emit(data)
        event = event_value(0)
        event['value']['sensory'] = dict(schema=[], body=[])
        with self.assertRaises(ValueError): lifecycle.value(event)
        source = dict(epoch='1', raw_space='2', owner=None, shape='Event', sensory=True)
        with self.assertRaises(ValueError): lifecycle.source(source)
        with self.assertRaises(ValueError): lifecycle.value(dict(kind='Bytes', value=[0, 255]))
        data = pose_read_fixture('EventSensory', event_value(0), dict(kind='Unit'), 1)
        with self.assertRaises(ValueError): lifecycle.emit(data)


if __name__ == '__main__':
    unittest.main()
