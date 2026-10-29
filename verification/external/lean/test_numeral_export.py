"""Actual frozen native numeral programs and mutations of their observations."""
import copy
import gzip
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from check_numeral_export import emit
from check_typed_programs import ROOT, check, require_success, run
from observer_export_replay import batches

FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'


def fixture(name):
    raw = gzip.decompress((ROOT / 'fixtures' / (name + '.json.gz')).read_bytes())
    provenance = json.loads((ROOT / 'fixtures' / (name + '.provenance.json')).read_text())
    if hashlib.sha256(raw).hexdigest() != provenance['export_sha256']:
        raise ValueError('Frozen fixture hash mismatch')
    return json.loads(raw)


class NativeNumeralExportTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.CheckedNumerals',
                            'VerumExternalReplay.FiniteExecution'))

    def prove(self, data, expected=None):
        source, counts = emit(data)
        with tempfile.TemporaryDirectory(prefix='verum-native-numeral-') as directory:
            path = Path(directory) / 'check.lean'
            for batch, _ in batches(source):
                path.write_text(batch)
                check(path, expected)
        return counts

    def test_actual_closed_boundary_and_nested_native_reports(self):
        self.assertEqual(self.prove(fixture('native-numerals-v1')),
                         dict(executions=37, claims=37))
        self.assertEqual(self.prove(fixture('native-numerals-nested-v1')),
                         dict(executions=3, claims=6))

    def test_wrong_reports_are_rejected_by_the_kernel(self):
        base = fixture('native-numerals-v1')
        payload = base['payload']
        mutations = [
            ('closed', 1, lambda r: r['report']['registers'][-1].update(value=7)),
            ('closed', 1, lambda r: r['report'].update(dispatch_attempts=1)),
            ('closed', 1, lambda r: r['code']['steps'][-1]['operation'].update(Primitive='Identity')),
            ('boundary', 1, lambda r: r['report']['halt']['reason'].update(Primitive='Sort')),
            ('boundary', 1, lambda r: r['report']['halt'].update(path=[1])),
            ('boundary', 1, lambda r: r['report'].update(stop='Complete')),
            ('boundary', 1, lambda r: r['report'].update(expanded_attempts=0)),
        ]
        for group, index, mutate in mutations:
            with self.subTest(group=group, index=index, mutate=mutate):
                row = copy.deepcopy(payload[group][index]); mutate(row)
                # Keep both required groups nonempty; each contains the same
                # altered observation so the batch must reach a false claim.
                altered = {**base, 'payload': dict(closed=[row], boundary=[row])}
                self.prove(altered, FALSE)

    def test_changed_decoded_code_and_invalid_schema_are_rejected(self):
        nested = fixture('native-numerals-nested-v1')
        nested['payload']['original_parts'][0]['library'][0]['output'] = 0
        self.prove(nested, 'Tactic `rfl` failed')
        for mutate in [
            lambda d: d.update(format='unknown'),
            lambda d: d['payload']['closed'][0].update(limit={'Dispatches': 1}),
            lambda d: d['payload']['closed'][0]['semantics'].update(kind='AcquiredV1'),
            lambda d: d['payload']['closed'][0]['code']['steps'][0]['operation'].update(Primitive='UncheckedSucc'),
            lambda d: d['payload']['closed'][0]['report']['registers'][0].update(value=2**64),
        ]:
            altered = fixture('native-numerals-v1'); mutate(altered)
            with self.assertRaises(ValueError): emit(altered)


if __name__ == '__main__':
    unittest.main()
