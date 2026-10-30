"""Strict replay of actual native append operations, without missing-field fallback."""
import gzip
import copy
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from check_persistent_history_export import emit
from check_typed_programs import check, require_success, run

ROOT = Path(__file__).resolve().parent
SHA256 = '24fcf1e06e5d6b38dd4fcc78427b0cf3a297768c303e4316d09a958285f0ac8a'
FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'


def fixture():
    raw = gzip.decompress((ROOT / 'fixtures/persistent-history-v1.json.gz').read_bytes())
    assert hashlib.sha256(raw).hexdigest() == SHA256
    return json.loads(raw)


class PersistentHistoryExportTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.PersistentEvents'))

    def replay(self, data, expected=None):
        source, counts = emit(data)
        if expected is None:
            source += '\n' + '\n'.join('#print axioms FrozenObserverExport.c' + str(i)
                                       for i in range(counts['claims']))
        with tempfile.TemporaryDirectory(prefix='verum-history-export-') as directory:
            path = Path(directory) / 'proof.lean'; path.write_text(source)
            result = check(path, expected)
        if expected is None:
            axioms = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
            self.assertEqual(len(axioms), counts['claims'])
            for row in axioms:
                self.assertTrue(row.endswith(('does not depend on any axioms',
                    'depends on axioms: [propext]', 'depends on axioms: [propext, Quot.sound]')), row)
        else:
            errors = [d['data'] for d in result['diagnostics'] if d['severity'] == 'error']
            self.assertTrue(errors)
            self.assertTrue(all(error.startswith(FALSE) for error in errors), errors)
        return counts

    def test_actual_history_export(self):
        self.assertEqual(self.replay(fixture()), dict(calls=44, snapshots=5, nodes=5, claims=107))

    def test_corrupt_native_semantics_reach_kernel_rejection(self):
        for mutation in ('branch_length', 'outer_length', 'rank', 'reverse_branch',
                         'owner_rebinding', 'work_spent', 'refusal', 'suffix_copy',
                         'early_end', 'iterator_reverse', 'census', 'shape_overflow',
                         'event_capacity', 'root_capacity'):
            data = fixture(); rows = data['calls']
            if mutation == 'branch_length': data['nodes'][2]['event_len'] -= 1
            elif mutation == 'outer_length': data['histories'][3]['end'] -= 1
            elif mutation == 'rank': data['nodes'][2]['rank'] = 0
            elif mutation == 'reverse_branch':
                data['nodes'][2]['body'].update(left=1, right=0)
            elif mutation == 'owner_rebinding':
                data['nodes'][1]['body']['events'][0]['input_owner'] = 0
            elif mutation == 'work_spent': rows[1]['spent'] = 1
            elif mutation == 'refusal': rows[1]['returned']['reason'] = 'Capacity'
            elif mutation == 'suffix_copy':
                row = next(r for r in rows if r['case'] == 'capacity_retry' and r['operation'] == 'begin_append')
                row['suffix'][0]['values'] = [99]
            elif mutation == 'early_end':
                next(r for r in rows if r['operation'] == 'iter')['end_get_is_none'] = False
            elif mutation == 'iterator_reverse':
                next(r for r in rows if r['operation'] == 'iter')['reverse'].reverse()
            elif mutation == 'census': data['retained']['event_slots'] += 1
            elif mutation == 'shape_overflow': rows[-3]['returned'] = [0, 2]
            elif mutation == 'event_capacity': data['retained']['event_capacity'] = 0
            elif mutation == 'root_capacity': data['retained']['snapshot_root_capacity'] = 0
            with self.subTest(mutation=mutation): self.replay(data, FALSE)

    def test_schema_and_lifecycle_refuse_missing_evidence(self):
        for mutation in ('schema', 'missing_owner', 'extra_state', 'boolean_grant',
                         'unknown_root', 'cycle', 'premature_finish', 'truncated'):
            data = fixture(); rows = data['calls']
            if mutation == 'schema': data['schema'] = 'synarc.native-prefix-history.v0'
            elif mutation == 'missing_owner': del data['nodes'][0]['body']['events'][0]['input_owner']
            elif mutation == 'extra_state': rows[0]['after']['legacy'] = True
            elif mutation == 'boolean_grant': rows[1]['grant'] = True
            elif mutation == 'unknown_root': data['histories'][1]['roots'] = [999]
            elif mutation == 'cycle': data['nodes'][2]['body']['left'] = 2
            elif mutation == 'premature_finish': rows.insert(1, dict(case='empty', operation='finish', returned=data['histories'][0]))
            elif mutation == 'truncated': data['calls'] = rows[:2]
            with self.subTest(mutation=mutation), self.assertRaises(ValueError): emit(data)

    def test_interleaved_completions_cannot_reuse_an_unpublished_owner(self):
        data = fixture(); rows = data['calls']
        twin = copy.deepcopy(rows[:6])
        for row in twin: row['case'] = 'empty_twin'
        data['calls'] = rows[:5] + twin[:5] + rows[5:6] + twin[5:] + rows[6:]
        with self.assertRaisesRegex(ValueError, 'Reused snapshot owner'): emit(data)

    def test_unobserved_node_cannot_enter_the_retained_owner_table(self):
        data = fixture()
        data['nodes'].append(dict(id=len(data['nodes']), event_len=0, rank=0,
                                  body=dict(kind='Leaf', events=[])))
        with self.assertRaisesRegex(ValueError, 'Unobserved node allocation'): emit(data)


if __name__ == '__main__':
    unittest.main()
