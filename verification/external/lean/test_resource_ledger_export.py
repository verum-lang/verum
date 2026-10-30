"""Boundary controls for the native memory export adapter; Lean checks the claims."""
import copy
import json
from pathlib import Path
import unittest
from check_resource_ledger_export import emit
from observer_export_replay import unique_object

FIXTURE = Path(__file__).parent / 'fixtures/resource-ledger-v1.json'


class ResourceExportTests(unittest.TestCase):
    def setUp(self):
        self.data = json.loads(FIXTURE.read_text())

    def test_complete_trace_has_operation_and_observation_proofs(self):
        source, counts = emit(self.data)
        self.assertEqual(counts, {'claims': 16, 'native_snapshots': 8})
        self.assertEqual(source.count('\ntheorem '), 16)
        self.assertIn('18446744073709551615', source)
        self.assertNotIn('sorry', source)
        self.assertNotIn('native_decide', source)

    def test_partial_or_reordered_lifecycle_is_not_a_replay(self):
        for rows in (self.data['events'][:-1], list(reversed(self.data['events']))):
            with self.subTest(rows=len(rows)), self.assertRaises(ValueError):
                emit({**self.data, 'events': rows})

    def test_unknown_schema_or_fields_refused(self):
        for data in ({**self.data, 'protocol': 'v2'}, {**self.data, 'trusted': True}):
            with self.assertRaises(ValueError):
                emit(data)

    def test_non_natural_and_out_of_word_values_refused(self):
        for key in ('held', 'peak', 'reservations', 'refusals', 'limit'):
            for value in (True, -1, 2**64, 1.5, '16', None):
                data = copy.deepcopy(self.data)
                data['events'][1][key] = value
                with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                    emit(data)

    def test_class_shape_and_values_refused(self):
        for value in ([], [0]*4, [0]*6, [False]*5, [0, -1, 0, 0, 0], '0'):
            data = copy.deepcopy(self.data)
            data['events'][1]['classes'] = value
            with self.subTest(value=value), self.assertRaises(ValueError):
                emit(data)

    def test_missing_counter_is_not_assumed_zero(self):
        for key in self.data['events'][0]:
            data = copy.deepcopy(self.data)
            del data['events'][0][key]
            with self.subTest(key=key), self.assertRaises(ValueError):
                emit(data)

    def test_export_content_is_only_the_assertion_not_model_state(self):
        data = copy.deepcopy(self.data)
        data['events'][4]['held'] = 33
        source, _ = emit(data)
        self.assertIn('total s4.slots = 33', source)
        self.assertIn('[⟨1,16,0⟩,⟨1,32,1⟩]', source)
        # The independent Lean rejection check must refuse this assertion.

    def test_duplicate_json_keys_are_refused(self):
        with self.assertRaises(ValueError):
            json.loads('{"held": 0, "held": 16}', object_pairs_hook=unique_object)


if __name__ == '__main__':
    unittest.main()
