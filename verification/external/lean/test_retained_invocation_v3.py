"""Controls mutate preserved v39 evidence; mutations are never native runs."""
import copy
import gzip
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from check_typed_programs import check, require_success, run
from check_retained_invocation_export import emit


class NativeInvocationV3Tests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.AffineService',
                            'VerumExternalReplay.RetainedNative', 'VerumExternalReplay.ObserverAdmission'))
        directory = Path(__file__).parent / 'fixtures'
        cls.raw = gzip.decompress((directory / 'shared-invocation-native-v3.json.gz').read_bytes())
        cls.original = json.loads(cls.raw)
        cls.provenance = json.loads((directory / 'shared-invocation-native-v3.provenance.json').read_bytes())

    def case(self, name):
        data = copy.deepcopy(self.original)
        data['cases'] = [next(c for c in data['cases'] if c['id'] == name)]
        data['associations'] = []
        return data, data['cases'][0]

    def test_original_bytes_and_complete_event_accounting(self):
        self.assertEqual(hashlib.sha256(self.raw).hexdigest(), self.provenance['export_sha256'])
        self.assertEqual(self.provenance['wider_run_final_status'], 'failed')
        self.assertEqual(emit(self.original)[1], {'cases': 19, 'associations': 2,
            'services': 248, 'native_events': 6297, 'claims': 2241,
            'nonpoisoned_terminal_retries': 0, 'consumed_retries': 25})

    def test_terminal_results_and_retries_kernel_checked(self):
        data = copy.deepcopy(self.original)
        data['cases'] = [c for c in data['cases'] if c['id'].startswith('terminal_')]
        data['associations'] = []
        source, counts = emit(data)
        self.assertEqual(counts['consumed_retries'], 24)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'terminal.lean'; path.write_text(source); check(path)

    def test_retry_side_effects_and_classification_corruptions_rejected(self):
        def work(s): s['meter']['after'] = str(int(s['meter']['after']) - 1)
        def dispatch(s): s['dispatchDelta'] = '1'
        def callback(s): s['trace'].append(copy.deepcopy(s['trace'][0]))
        def republish(s): s['returned'] = {'kind': 'complete', 'result': {}}
        def reclassify(s): s['returned']['refusal']['classification'] = 'Poisoned'
        def prior(s): s['returned']['refusal']['raw']['priorClassification'] = 'Undefined'
        def reopen(s): s['after']['finished'] = False
        for edit in (work, dispatch, callback, republish, reclassify, prior, reopen):
            for name in ('terminal_complete_read', 'terminal_resource_update'):
                with self.subTest(edit=edit.__name__, case=name):
                    data, case = self.case(name); edit(case['services'][1])
                    with self.assertRaises(ValueError): emit(data)

    def test_terminal_snapshot_changes_rejected(self):
        for name in ('terminal_complete_read', 'terminal_resource_read', 'terminal_missing_output_update'):
            for kind in ('call', 'invocation'):
                with self.subTest(case=name, kind=kind):
                    data, case = self.case(name)
                    event = next(e for e in case['services'][0]['trace']
                                 if e['kind'] == kind and e['event']['kind'] == 'service_leave')
                    event['state']['terminalClass'] = 'Poisoned'
                    with self.assertRaises(ValueError): emit(data)

    def test_schema_is_not_silently_downgraded(self):
        for version in ('v1', 'v2', 'v4'):
            data, _ = self.case('read_whole'); data['schema'] = 'synarc-shared-vm-native-' + version
            with self.assertRaises(ValueError): emit(data)

    def test_numeric_terminal_guard_rejected(self):
        data, case = self.case('terminal_complete_read'); case['services'][1]['after']['finished'] = 1
        with self.assertRaisesRegex(ValueError, 'Native scalar'): emit(data)


if __name__ == '__main__': unittest.main()
