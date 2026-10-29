"""Malformed and semantically false frozen declaration exports must be rejected."""
import copy
from pathlib import Path
import tempfile
import unittest
from check_predeclared_export import emit
from check_typed_programs import check, require_success, run


def fixture():
    lineage = dict(authority=5, birth=0)
    source = dict(epoch='1', raw_space='2', owner=lineage, shape='Event')
    id_ = lambda n: dict(lineage=lineage, ordinal=n)
    row = lambda n: dict(id=id_(n), source=source, origin=0,
        context=dict(epoch='1', input=n, action=0), outcome=dict(kind='Outcome', value=dict(kind='Next', key=n)))
    base = dict(source=source, origin=dict(epoch='1', lineage=lineage, ordinal=0), declaration_end=id_(0),
        reserved_look='2', paid_look=None, next_reservation='3', rows=[],
        unscored=None, validation_selected=False, selected_training=None)
    events = []
    for kind in ['Declared', 'Buffered', 'OtherReservations', 'Opened', 'TrainingBoundary']:
        event = copy.deepcopy(base); event['kind'] = kind
        if kind != 'Declared': event['rows'] = [row(1),row(2)]
        if kind in ('OtherReservations','Opened','TrainingBoundary'): event['next_reservation'] = '7'
        if kind in ('Opened','TrainingBoundary'): event['paid_look'] = '2'
        if kind == 'TrainingBoundary':
            event.update(rows=[row(1)], selected_training=1, unscored=[id_(2)])
        events.append(event)
    return events


class PredeclaredExportTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake','build','VerumExternalReplay.PredeclaredEvidence'))

    def prove(self, data, false=False):
        source, counts = emit(data)
        with tempfile.TemporaryDirectory(prefix='verum-declared-export-') as directory:
            path = Path(directory)/'claims.lean'; path.write_text(source)
            result = check(path, '`decide_cbv` failed: the proposition evaluates to `false`' if false else None)
        errors = [d for d in result['diagnostics'] if d['severity'] == 'error']
        if false:
            self.assertTrue(errors)
            self.assertTrue(all('proposition evaluates to `false`' in d['data'] for d in errors))
        else:
            self.assertEqual(len([d for d in result['diagnostics'] if d['severity']=='information']), counts['claims'])
        return counts

    def test_projection_and_scope_are_explicit(self):
        counts = self.prove(fixture())
        self.assertEqual((counts['buffered_rows'],counts['selected_rows'],counts['unscored_ids']),(2,1,1))
        self.assertIn('signature/family/criterion freeze',counts['unchecked'])
        self.assertIn('actual candidate selection and fresh validation',counts['unchecked'])

    def test_changed_ordinals_owners_payloads_and_splits_fail_semantically(self):
        cases = []
        data=fixture(); data[3]['paid_look']='6'; cases.append(data)
        data=fixture(); data[2]['next_reservation']='1'; cases.append(data)
        data=fixture(); data[3]['source']['raw_space']='3'; cases.append(data)
        data=fixture(); data[3]['rows'][0]['context']['input']=99; cases.append(data)
        data=fixture(); data[-1]['rows']=[copy.deepcopy(data[3]['rows'][1])]; cases.append(data)
        data=fixture(); data[-1]['unscored'][0]['ordinal']=1; cases.append(data)
        data=fixture()
        for event in data[1:4]: event['rows'].reverse()
        data[-1]['rows']=[copy.deepcopy(data[3]['rows'][0])]
        data[-1]['unscored']=[copy.deepcopy(data[3]['rows'][1]['id'])]
        cases.append(data)
        for index,data in enumerate(cases):
            with self.subTest(index=index): self.prove(data,True)

    def test_incomplete_or_unsupported_exports_do_not_gain_evidence(self):
        data=fixture(); data.pop()
        with self.assertRaisesRegex(ValueError,'five'): emit(data)
        data=fixture(); data[0]['signature']='invented'
        with self.assertRaisesRegex(ValueError,'Expected fields'): emit(data)
        data=fixture(); data[-1]['validation_selected']=True
        with self.assertRaisesRegex(ValueError,'unsupported'): emit(data)
        data=fixture(); data[1]['rows'][0]['id']['ordinal']=True
        with self.assertRaisesRegex(ValueError,'Out-of-range'): emit(data)


if __name__ == '__main__': unittest.main()
