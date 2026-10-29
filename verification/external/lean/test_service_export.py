"""Frozen native service microsteps and strict corruption controls."""
import gzip
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from check_service_export import emit
from check_typed_programs import ROOT, check, require_success, run

FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'


def fixture(case):
    name = 'native-service-' + case + '-v2'
    raw = gzip.decompress((ROOT / 'fixtures' / (name + '.json.gz')).read_bytes())
    provenance = json.loads((ROOT / 'fixtures' / (name + '.provenance.json')).read_text())
    if hashlib.sha256(raw).hexdigest() != provenance['export_sha256']:
        raise ValueError('Frozen fixture hash mismatch')
    return json.loads(raw)


class NativeServiceExportTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.GuardedDrainTrace'))

    def prove(self, data, expected=None):
        source, counts = emit(data)
        with tempfile.TemporaryDirectory(prefix='verum-service-export-') as directory:
            path = Path(directory) / 'check.lean'; path.write_text(source)
            check(path, expected)
        return counts

    def test_every_native_call_and_operation(self):
        total = dict(services=0, steps=0)
        for case, expected in [('whole', (1, 4)), ('partitioned', (11, 14)),
                               ('zero', (1, 0)), ('publication-capacity', (2, 4)),
                               ('foreign-owner', (2, 3))]:
            result = self.prove(fixture(case))
            self.assertEqual((result['services'], result['steps']), expected)
            for key in total: total[key] += result[key]
        self.assertEqual(total, dict(services=17, steps=25))

    def test_missing_reordered_and_fabricated_operations_fail(self):
        mutations = [
            lambda e: e['steps'].pop(1),  # paid publication gate
            lambda e: e['steps'].pop(2),  # zero-cost handoff
            lambda e: e.update(steps=[]),
            lambda e: e['steps'].reverse(),
            lambda e: e['steps'].append(e['steps'][-1]),  # callback after Published
            lambda e: e['steps'][0]['result'].update(value='Blocked'),
            lambda e: e['steps'][2]['after'].update(remaining=1000001),
            lambda e: e['after']['installed_floor_definitions'][0].update(definition_owner='0'),
            lambda e: e.update(result='Pending'),
        ]
        for index, mutate in enumerate(mutations):
            with self.subTest(index=index):
                data = fixture('whole'); mutate(data['events'][0]); self.prove(data, FALSE)

    def test_refusal_work_and_identity_cannot_be_erased(self):
        data = fixture('publication-capacity'); data['events'] = data['events'][:1]
        data['events'][0]['result'] = 'Published'; self.prove(data, FALSE)
        data = fixture('foreign-owner'); data['events'] = data['events'][:1]
        event = data['events'][0]
        event['steps'][-1]['after']['install_refused'] = False
        event['after']['install_refused'] = False
        self.prove(data, FALSE)
        data = fixture('zero'); data['events'][0]['before']['remaining'] = 1
        self.prove(data, FALSE)

    def test_schema_and_numeric_refusals(self):
        for mutate in [
            lambda d: d.update(schema='unknown'),
            lambda d: d['events'][0].pop('steps'),
            lambda d: d['events'][0]['before'].update(remaining=True),
            lambda d: d['events'][0]['before'].update(next_look=None),
            lambda d: d['events'][0]['steps'][0].update(operation='invented'),
            lambda d: d['events'][0]['steps'][0]['result'].update(extra=True),
        ]:
            data = fixture('whole'); mutate(data)
            with self.assertRaises(ValueError): emit(data)


if __name__ == '__main__':
    unittest.main()
