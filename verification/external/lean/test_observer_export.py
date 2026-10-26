"""Extraction refuses ambiguous encodings; corrupted observations reach Lean rejection."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from check_observer_export import ROOT, batches, emit, load, unique_object, value
from check_typed_programs import check, sha256

FIXTURE = ROOT / 'fixtures/observer-rust-boundary-v1.json'


class ExtractionTests(unittest.TestCase):
    def setUp(self):
        self.data = load(FIXTURE)

    def test_frozen_fixture_identity(self):
        receipt = load(FIXTURE.with_suffix('.provenance.json'))
        self.assertEqual(sha256(FIXTURE), receipt['export_sha256'])
        _, counts = emit(self.data)
        self.assertEqual(counts, {'programs': 4, 'trace_rows': 20,
                                  'primitive_cases': 14, 'claims': 74})

    def test_batches_cover_each_claim_once(self):
        text, counts = emit(self.data)
        names = []
        for part, group in batches(text):
            self.assertLessEqual(len(group), 8)
            self.assertEqual(part.count('theorem '), len(group))
            self.assertEqual(part.count('#print axioms '), len(group))
            self.assertEqual(part.count('def p'), 8)
            names.extend(group)
        self.assertEqual(len(names), counts['claims'])
        self.assertEqual(len(set(names)), counts['claims'])

    def test_ambiguous_numbers_and_values_are_refused(self):
        for obj in [{'Nat': True}, {'Nat': -1}, {'Nat': 2**64}, {'Nat': 1.0},
                    {'Bool': 1}, {'Pair': []}, {'Nat': 0, 'Bool': False}]:
            with self.subTest(obj=obj), self.assertRaises(ValueError):
                value(obj)
        with self.assertRaises(ValueError):
            json.loads('{"Nat":1,"Nat":2}', object_pairs_hook=unique_object)

    def test_unknown_semantics_and_fields_are_refused(self):
        for name, val in [('semantics', 'FutureVersion'), ('schema_version', True),
                          ('schema_version', 2), ('unmodeled_effect', 'write')]:
            data = copy.deepcopy(self.data)
            data['programs'][0]['description'][name] = val
            with self.subTest(name=name), self.assertRaises(ValueError):
                emit(data)

    def test_invalid_binding_and_diagnostic_are_refused(self):
        self.data['programs'][0]['description']['read_binding']['slots'][0] = {'External': 4}
        with self.assertRaises(ValueError):
            emit(self.data)
        self.data = load(FIXTURE)
        self.data['primitives'][1]['error'] = 'Budget'
        with self.assertRaisesRegex(ValueError, 'Unknown refusal'):
            emit(self.data)

    def test_independent_read_descriptor_is_preserved(self):
        d = self.data['programs'][0]['description']
        d['read_binding']['descriptor']['input_epoch'] += 1
        # The export format supplies observations under the definition's own
        # interface. Equality of the two interface descriptors is not a premise.
        emit(self.data)

    def test_corrupted_observations_are_rejected_by_kernel(self):
        data = copy.deepcopy(self.data)
        data['programs'] = data['programs'][:1]
        data['programs'][0]['executions'] = data['programs'][0]['executions'][:1]
        data['primitives'] = data['primitives'][1:2]
        changes = {
            'wrong_initial': lambda d: d['programs'][0]['description']['initial']['Pair'][0].update(Nat=8),
            'wrong_read': lambda d: d['programs'][0]['executions'][0]['read']['Pair'][0].update(Nat=0),
            'wrong_binding': lambda d: d['programs'][0]['description']['read_binding']['slots'].__setitem__(1, {'External': 1}),
            'wrong_error': lambda d: d['primitives'][0].update(error='ZeroDivisor'),
        }
        with tempfile.TemporaryDirectory(prefix='observer-export-controls-') as tmp:
            for name, mutate in changes.items():
                candidate = copy.deepcopy(data)
                mutate(candidate)
                path = Path(tmp) / (name + '.lean')
                path.write_text(emit(candidate)[0])
                with self.subTest(name=name):
                    check(path, 'Tactic `rfl` failed:')


if __name__ == '__main__':
    unittest.main()
