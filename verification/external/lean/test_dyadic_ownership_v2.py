"""Strict rich ownership evidence; no fallback when required native identities are absent."""
import gzip
import copy
import hashlib
import json
from pathlib import Path
import unittest
from check_dyadic_ownership_export import emit
import test_dyadic_ownership_export as baseline
from test_dyadic_ownership_export import FALSE, visit


def fixture():
    raw = gzip.decompress((Path(__file__).resolve().parent / 'fixtures/dyadic-ownership-v2.json.gz').read_bytes())
    assert hashlib.sha256(raw).hexdigest() == 'd82b02487859c8ced59746adf135b5657acee3f24641a99b3c89b9eb6f3cbf77'
    return json.loads(raw)


class DyadicOwnershipV2Tests(unittest.TestCase):
    setUpClass = classmethod(baseline.DyadicOwnershipTests.setUpClass.__func__)
    replay = baseline.DyadicOwnershipTests.replay

    def test_full_rich_export(self):
        counts = self.replay(fixture())
        self.assertEqual(counts, dict(events=6126, segments=4, snapshots=811, claims=669))

    def test_changed_native_identity_and_stop_evidence(self):
        for mutation in ('promoted_initializer', 'look_authority', 'look_index',
                         'original_limit_retryable', 'accepted_retry', 'undeclared_capacity'):
            data = fixture()
            if mutation == 'accepted_retry':
                next(e for e in data['events'] if e['operation'] == 'retry_materialization')['accepted'] = True
            elif mutation == 'undeclared_capacity':
                data['events'] = data['events'][:1]
                data['events'][0]['after']['capacity']['members'] += 1
            else:
                def change(o):
                    candidate = 'original_name' in o and 'programs_ready' not in o
                    if candidate and mutation == 'promoted_initializer' and o['original_name'] == '0':
                        o['initializer_owner'] += 100
                    elif candidate and mutation == 'look_authority': o['paid_look']['authority'] += 100
                    elif candidate and mutation == 'look_index':
                        o['paid_look']['index'] = str(int(o['paid_look']['index']) + 1)
                        o['look'] = o['paid_look']['index']
                    elif mutation == 'original_limit_retryable' and o.get('initial', {}).get('reason') == 'OriginalDispatch':
                        o['can_resume'] = True
                visit(data, change)
            with self.subTest(mutation=mutation): self.replay(data, FALSE)

    def test_missing_rich_fields_never_select_legacy_interpretation(self):
        for mutation in ('missing_initializer', 'missing_typed_stop', 'missing_active_stop',
                         'missing_authority', 'reused_owner', 'unknown_version'):
            data = fixture()
            def remove(o):
                if mutation == 'missing_initializer' and 'original_name' in o and 'programs_ready' not in o:
                    o.pop('initializer_owner')
                elif mutation == 'missing_typed_stop' and o.get('refusal', {}).get('kind') == 'Materialization':
                    o.pop('initial')
                elif mutation == 'missing_active_stop' and 'materializer_stop' in o:
                    o['materializer_stop'] = None
                elif mutation == 'missing_authority' and 'authority' in o: o.pop('authority')
                elif mutation == 'reused_owner' and 'source_owner' in o and 'owner' in o: o['owner'] = 0
            if mutation == 'unknown_version': data['schema'] = 'synarc-dyadic-cumulative-ownership-v3'
            else: visit(data, remove)
            with self.subTest(mutation=mutation), self.assertRaises(ValueError): emit(data)

    def test_exact_work_and_monotone_capacity_semantics(self):
        data = fixture()
        def active_execution(o):
            if o.get('initial', {}).get('kind') == 'Work': o['initial']['execution_present'] = True
        visit(data, active_execution)
        self.replay(data, FALSE)
        data = fixture()
        index = next(i for i, r in enumerate(data['events']) if r['operation'] == 'extend_capacity')
        data['events'] = data['events'][:index + 1]
        row = data['events'][-1]; row['rows'] = 7
        self.replay(data)  # max(8,7)=8: a smaller grant is a valid no-op.
        row['after']['capacity']['rows'] = 7
        self.replay(data, FALSE)

    def test_parked_stop_and_completed_initializer_cannot_disappear(self):
        data = fixture()
        index = next(i for i, r in enumerate(data['events']) if r['case'] == 'original_dispatch_unknown')
        data['events'] = data['events'][:index + 1]
        row = data['events'][-1]
        row['after'] = copy.deepcopy(row['before']); row['after']['materializer_stop'] = None
        with self.assertRaisesRegex(ValueError, 'Parked owner/stop pair disappeared'): emit(data)
        data = fixture(); data['events'] = data['events'][:100]
        seed = next(r['after']['materializing'] for r in data['events']
                    if r['after']['materializing'] is not None
                    and r['after']['materializing']['initializer_owner'] is None)
        row = data['events'][-1]
        row['after']['materialized'] = None
        row['after']['materializing'] = copy.deepcopy(seed)
        with self.assertRaisesRegex(ValueError, 'Materialized candidate rolled back'): emit(data)


if __name__ == '__main__':
    unittest.main()
