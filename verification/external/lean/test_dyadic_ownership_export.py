"""Frozen native projection replay and controls; missing export fields remain unproved."""
import gzip
import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from check_dyadic_ownership_export import emit
from check_typed_programs import check, require_success, run

FROZEN = Path(__file__).resolve().parent / 'fixtures/dyadic-ownership-v1.json.gz'
SHA256 = 'a53770d8c75fa3b7eef32210d891338a2f839720f84adba69d49676a953e74e4'
FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'


def fixture():
    raw = gzip.decompress(FROZEN.read_bytes())
    assert hashlib.sha256(raw).hexdigest() == SHA256
    return json.loads(raw)


def visit(value, action):
    if isinstance(value, dict):
        action(value)
        for child in value.values(): visit(child, action)
    elif isinstance(value, list):
        for child in value: visit(child, action)


class DyadicOwnershipTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.RectangularNames',
                            'VerumExternalReplay.AdditiveTraversal'))

    def replay(self, data, expected=None):
        source, counts = emit(data)
        if expected is None:
            source += '\n' + '\n'.join('#print axioms FrozenObserverExport.c' + str(i)
                                       for i in range(counts['claims']))
        with tempfile.TemporaryDirectory(prefix='verum-dyadic-ownership-') as directory:
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

    def test_complete_frozen_export(self):
        counts = self.replay(fixture())
        self.assertEqual({k:v for k,v in counts.items() if k != 'claims'},
                         dict(events=6124, segments=4, snapshots=456))
        self.assertGreater(counts['claims'], 545)

    def test_semantic_mutations_reach_kernel_rejection(self):
        for mutation in ('original_name', 'coordinates', 'family_count', 'work_accounting',
                         'unknown_reason', 'refusal_owner'):
            data = fixture()
            def change(o):
                if mutation == 'original_name':
                    if o.get('original_name') == '4': o['original_name'] = '99'
                    if o.get('pending_original_name') == '4': o['pending_original_name'] = '99'
                elif mutation == 'coordinates' and o.get('original_name') == '4':
                    o['coordinates'] = [0, 0, 0]
                elif mutation == 'family_count' and 'family_denominator' in o:
                    o['family_denominator'] = '31'
                elif mutation == 'work_accounting' and 'spent' in o:
                    o['spent'] = 0
                elif mutation == 'unknown_reason' and 'unknown' in o:
                    for u in o['unknown']: u['reason']['can_resume'] = True
                elif mutation == 'refusal_owner' and o.get('kind') == 'Stopped' and o['index'] is not None:
                    o['index'] = '999'
            visit(data, change)
            with self.subTest(mutation=mutation): self.replay(data, FALSE)

    def test_schema_and_chronology_controls(self):
        for mutation in ('version', 'extra', 'boolean_grant', 'wide_grant', 'leading_zero',
                         'zero_work_owner', 'lost_unknown', 'capacity_change', 'legacy_order'):
            data = fixture(); rows = data['events']
            if mutation == 'version': data['schema'] = 'synarc-dyadic-cumulative-ownership-v2'
            elif mutation == 'extra': rows[0]['after']['fabricated'] = True
            elif mutation == 'boolean_grant': rows[0]['grant'] = True
            elif mutation == 'wide_grant': rows[0]['grant'] = 2
            elif mutation == 'leading_zero': rows[0]['before']['enumeration']['issued_names'] = '00'
            elif mutation == 'legacy_order': rows[0]['before']['enumeration']['service_order'] = 'declared-correlated-v1'
            elif mutation == 'zero_work_owner':
                row = next(r for r in rows if r['case'] == 'zero_work')
                row['after']['materializing']['initializer_owner'] += 1
            elif mutation == 'lost_unknown':
                row = next(r for r in rows if r['case'] == 'original_dispatch_unknown')
                row['after']['unknown'][0]['owner']['initializer_owner'] += 1
            elif mutation == 'capacity_change':
                row = next(r for r in rows if r['operation'] == 'extend_capacity')
                row['after']['members'][0]['look'] = '1'
            with self.subTest(mutation=mutation), self.assertRaises(ValueError): emit(data)

    def test_replacement_and_wrong_existing_refusal_owner(self):
        data = fixture()
        def duplicate(o):
            if o.get('original_name') == '4': o['original_name'] = '3'
            if o.get('pending_original_name') == '4': o['pending_original_name'] = '3'
        visit(data, duplicate)
        with self.assertRaisesRegex(ValueError, 'Reissued original name'): emit(data)
        data = fixture(); data['events'] = data['events'][:100]
        row = data['events'][-1]
        row['after']['materialized'].update(original_name='1', coordinates=[0, 0, 1])
        row['returned']['index'] = '1'
        with self.assertRaisesRegex(ValueError, 'Unissued retained owner'): emit(data)
        data = fixture(); data['events'] = data['events'][:180]
        row = data['events'][-1]
        self.assertEqual(row['after']['materializing']['original_name'], '1')
        self.assertEqual(row['after']['members'][0]['original_name'], '0')
        row['returned']['index'] = '0'
        self.replay(data, FALSE)


if __name__ == '__main__':
    unittest.main()
