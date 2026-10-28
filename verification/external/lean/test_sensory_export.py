"""Strict native sensory extraction and mutations of the original frozen evidence."""
import copy
from pathlib import Path
import tempfile
import unittest

from acquired_export_codec import LEGACY, SENSORY
from check_sensory_export import emit
import check_lifecycle_export as lifecycle
from check_typed_programs import ROOT, check, require_success, run
from observer_export_replay import batches, load
from test_lifecycle_export import model_fixture


def fixture():
    return load(ROOT/'fixtures/sensory-lifecycle-v1.json')


class SensoryExportTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.SensoryPort',
                            'VerumExternalReplay.TraceReduction', 'VerumExternalReplay.ExactNames',
                            'VerumExternalReplay.ObserverOwnership'))

    def prove_batch(self, data, name, expected=None):
        source, counts = emit(data)
        parts = list(batches(source))
        self.assertEqual(sum(len(names) for _, names in parts), counts['claims'])
        selected = [(text, names) for text, names in parts if name in names]
        self.assertEqual(len(selected), 1)
        with tempfile.TemporaryDirectory(prefix='sensory-export-control-') as directory:
            path = Path(directory)/'check.lean'; path.write_text(selected[0][0])
            return check(path, expected)

    def test_actual_physical_prefix_and_continuation(self):
        data = fixture()
        self.assertEqual(emit(data)[1], dict(definitions=1, ownership=9, naming=0,
            installation=4, refusals=0, claims=112, physical_events=2, physical_continuation=1))
        self.prove_batch(data, 'sensory_4')
        self.prove_batch(data, 'sensory_6')

    def test_actual_capture_and_prefix_mutations_fail_proofs(self):
        for change in ('presence', 'prefix_body', 'initial', 'continuation_body', 'dropped_row'):
            data = fixture(); physical = data['physical_replay']
            if change == 'presence': del physical['events'][0]['interface']['sensory']
            elif change == 'prefix_body': physical['events'][0]['values'][0]['value']['sensory']['body'][0] = 8
            elif change == 'initial': physical['installed_before']['events'][0]['sensory']['schema'][0] = 0
            elif change == 'continuation_body': physical['events'][1]['values'][0]['value']['sensory']['body'][0] = 8
            else: physical['replay_after']['events'].pop()
            with self.subTest(change=change):
                self.prove_batch(data, 'sensory_4', 'Tactic `rfl` failed:')

    def test_refusal_metadata_is_preserved(self):
        data = fixture()
        row = next(r for r in data['ownership'] if r['name']=='read_forged_presence')
        interface, values = SENSORY.input_row(row['input'])
        self.assertIn('true', interface)
        self.assertNotIn('(some ⟨', values)
        row['input']['values'][0]['value']['sensory'] = dict(schema=[], body=[7])
        # The expected missing-port refusal is now false, not repaired by decoding.
        source, _ = emit(data)
        part = next((text, names) for text, names in batches(source)
                    if 'observe o2' in text)
        with tempfile.TemporaryDirectory(prefix='sensory-forged-presence-') as directory:
            path = Path(directory)/'check.lean'; path.write_text(part[0])
            check(path, 'Tactic `rfl` failed:')

    def test_canonical_byte_and_presence_schema(self):
        for bad in (True, 1.0, -1, 256):
            with self.subTest(byte=bad), self.assertRaises(ValueError):
                SENSORY.value(dict(kind='Bytes', value=[bad]))
        self.assertEqual(SENSORY.value(dict(kind='Bytes', value=[])), '(.bytes [])')
        event = fixture()['physical_replay']['events'][0]['values'][0]
        for payload in (None, {}, dict(schema=[], body=[], trusted=True)):
            changed = copy.deepcopy(event); changed['value']['sensory'] = payload
            with self.subTest(payload=payload), self.assertRaises(ValueError): SENSORY.value(changed)
        source = fixture()['physical_replay']['events'][0]['interface']
        for presence in (False, None, 1, 'true'):
            with self.subTest(presence=presence), self.assertRaises(ValueError):
                SENSORY.source({**source, 'sensory': presence})

    def test_legacy_is_closed_and_definition_versions_are_explicit(self):
        data = fixture(); definition = data['definitions'][0]
        old = model_fixture()['definitions'][0]; old['identity'] = 1
        old['semantics']['kind'] = 'AcquiredV2'
        base = dict(schema='synarc-observer-lifecycle-v1', definitions=[definition, old],
                    ownership=[], naming=[], installation=[], refusals=[])
        lifecycle.emit(base, decoder=SENSORY)
        with self.assertRaises(ValueError): lifecycle.emit(base)
        with self.assertRaises(ValueError): LEGACY.value(dict(kind='Bytes', value=[0]))
        old['read']['steps'][0]['operation'] = dict(Primitive='QuerySensory')
        with self.assertRaises(ValueError): lifecycle.emit(base, decoder=SENSORY)

    def test_missing_or_substituted_evidence_is_rejected(self):
        for change in ('unknown', 'no_continuation', 'request', 'frame', 'definition'):
            data = fixture()
            if change == 'unknown': data['physical_replay']['trusted'] = True
            elif change == 'no_continuation': data['physical_replay']['events'].pop()
            elif change == 'request': data['installation'][1]['request']['known_histories'] = []
            elif change == 'frame': data['installation'][1]['current_before'] = data['installation'][0]['installed']
            else: data['definitions'][0]['semantics']['kind'] = 'AcquiredV2'
            with self.subTest(change=change), self.assertRaises(ValueError): emit(data)


if __name__ == '__main__':
    unittest.main()
