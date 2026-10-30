"""Frozen r47 move-only cursor edges, without case-label identity or v1 fallback."""
import copy
import gzip
import hashlib
import json
from pathlib import Path
import unittest

from check_persistent_history_export import emit
from check_typed_programs import require_success, run
import test_persistent_history_export as v1

ROOT = Path(__file__).resolve().parent
SHA256 = 'e5da5f1c823b911446db74986475a7fb102ad48b944a26e84c3e102f51ba2dad'


def fixture():
    raw = gzip.decompress((ROOT / 'fixtures/persistent-history-v2.json.gz').read_bytes())
    assert hashlib.sha256(raw).hexdigest() == SHA256
    return json.loads(raw)


def row(data, operation, case=None):
    return next(r for r in data['calls'] if r['operation'] == operation
                and (case is None or r['case'] == case))


class PersistentHistoryV2Tests(unittest.TestCase):
    replay = v1.PersistentHistoryExportTests.replay

    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.PersistentEvents'))

    def test_actual_edge_export(self):
        self.assertEqual(self.replay(fixture()),
                         dict(calls=122, snapshots=9, nodes=15, claims=250))

    def test_labels_do_not_select_append_or_iterator_owners(self):
        data = fixture()
        # Both collisions and changes within the same owner are harmless.
        for i, record in enumerate(data['calls']):
            record['case'] = 'repeated' if i % 2 else str(i)
        self.assertEqual(self.replay(data),
                         dict(calls=122, snapshots=9, nodes=15, claims=250))

    def test_semantic_mutations_reach_kernel_rejection(self):
        for mutation in ('retained_suffix', 'retained_carry', 'retained_previous', 'carry_shape',
                         'zero_work', 'capacity_priority', 'capacity_paid',
                         'unwind_entered', 'unwind_poison', 'unwind_sticky',
                         'poison_retry_paid', 'poison_retry_kind', 'poison_finish',
                         'finish_before', 'iterator_history', 'iterator_owner',
                         'iterator_after', 'iterator_hint', 'iterator_exhaustion'):
            data = fixture()
            if mutation == 'retained_suffix':
                r = row(data, 'finish_retained', 'unstarted_finish')
                r['after']['suffix'][0]['values'] = [42]
            elif mutation == 'retained_carry':
                r = row(data, 'finish_retained', 'carry_finish')
                r['after']['carry'] = 0
                n = data['nodes'][0]
                r['after']['carry_shape'] = [n['rank'], n['event_len']]
            elif mutation == 'retained_previous':
                row(data, 'finish_retained', 'carry_finish')['after']['previous'] = 6
            elif mutation == 'carry_shape':
                row(data, 'advance', 'carry_capacity')['after']['carry_shape'][1] += 1
            elif mutation == 'zero_work':
                row(data, 'advance', 'carry_zero')['spent'] = 1
            elif mutation == 'capacity_priority':
                row(data, 'advance', 'carry_zero')['returned']['reason'] = 'Capacity'
            elif mutation == 'capacity_paid':
                row(data, 'advance', 'carry_capacity')['spent'] = 0
            elif mutation == 'unwind_entered':
                row(data, 'advance_unwind')['meter_calls_entered'] = 0
            elif mutation == 'unwind_poison':
                row(data, 'advance_unwind')['after']['poisoned'] = False
            elif mutation == 'unwind_sticky':
                row(data, 'advance_unwind')['after']['stopped'] = 'Poisoned'
            elif mutation == 'poison_retry_paid':
                row(data, 'advance', 'poison_retry')['spent'] = 1
            elif mutation == 'poison_retry_kind':
                row(data, 'advance', 'poison_retry')['returned']['reason'] = 'Work'
            elif mutation == 'poison_finish':
                row(data, 'finish_retained', 'poison_finish')['returned']['kind'] = 'History'
            elif mutation == 'finish_before':
                row(data, 'finish', 'carry_resume')['before']['origin'] = 7
            elif mutation == 'iterator_history':
                row(data, 'next', 'mixed_old_reader')['history'] = 8
            elif mutation == 'iterator_owner':
                row(data, 'next', 'mixed_old_reader')['returned']['input_owner'] = 999
            elif mutation == 'iterator_after':
                row(data, 'next', 'mixed_old_reader')['after']['front'] = 0
            elif mutation == 'iterator_hint':
                row(data, 'begin_iter', 'mixed_old_reader')['size_hint'] = [0, 0]
            elif mutation == 'iterator_exhaustion':
                r = next(r for r in data['calls'] if r['operation'] == 'next'
                         and r['returned'] is None)
                r['returned'] = copy.deepcopy(data['histories'][7]['events'][0])
            with self.subTest(mutation=mutation): self.replay(data, v1.FALSE)

    def test_missing_edge_evidence_and_invalid_lifetimes_refuse(self):
        for mutation in ('v1_downgrade', 'unknown_schema', 'missing_suffix', 'missing_carry_shape',
                         'downgraded_owner', 'missing_finish_before', 'invented_charge',
                         'extra_grant', 'boolean_meter', 'overlap_append', 'overlap_iterator',
                         'unopened_reader', 'past_snapshot_end', 'unfinished_poison',
                         'unfinished_iterator', 'unwind_retry', 'completed_owner', 'cleared_poison'):
            data = fixture()
            if mutation == 'v1_downgrade': data['schema'] = 'synarc.native-prefix-history.v1'
            elif mutation == 'unknown_schema': data['schema'] = 'synarc.native-prefix-history.v3'
            elif mutation == 'missing_suffix':
                del row(data, 'finish_retained')['before']['suffix']
            elif mutation == 'missing_carry_shape':
                del row(data, 'advance', 'carry_zero')['after']['carry_shape']
            elif mutation == 'downgraded_owner':
                # Erasing both fields in the whole edge lifetime still cannot
                # turn finish_retained or advance_unwind into old setup calls.
                for r in data['calls']:
                    for k in ('before', 'after'):
                        if k in r:
                            r[k].pop('suffix', None); r[k].pop('carry_shape', None)
            elif mutation == 'missing_finish_before':
                del row(data, 'finish', 'carry_resume')['before']
            elif mutation == 'invented_charge': row(data, 'advance_unwind')['spent'] = 1
            elif mutation == 'extra_grant': row(data, 'advance_unwind')['grant'] = 1
            elif mutation == 'boolean_meter': row(data, 'advance_unwind')['meter_calls_entered'] = True
            elif mutation == 'overlap_append':
                data['calls'].insert(1, copy.deepcopy(data['calls'][0]))
            elif mutation == 'overlap_iterator':
                i = next(i for i, r in enumerate(data['calls']) if r['operation'] == 'begin_iter')
                data['calls'].insert(i + 1, copy.deepcopy(data['calls'][i]))
            elif mutation == 'unopened_reader':
                data['calls'].remove(row(data, 'begin_iter'))
            elif mutation == 'past_snapshot_end':
                row(data, 'begin_iter')['back'] = 8
            elif mutation == 'unfinished_poison': data['calls'].pop()
            elif mutation == 'unfinished_iterator':
                data['calls'] = [r for r in data['calls']
                    if not (r['case'] == 'mixed_prefix' and r['operation'] in ('next', 'next_back'))]
            elif mutation == 'unwind_retry':
                r = row(data, 'advance_unwind'); r['before']['poisoned'] = True
            elif mutation == 'completed_owner':
                row(data, 'finish_retained')['before']['complete'] = 8
            elif mutation == 'cleared_poison':
                row(data, 'finish_retained', 'poison_finish')['after']['poisoned'] = False
            with self.subTest(mutation=mutation), self.assertRaises(ValueError): emit(data)


if __name__ == '__main__':
    unittest.main()
