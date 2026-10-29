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
    row = lambda n: dict(id=id_(n), source=source, origin=dict(epoch='1', lineage=lineage, ordinal=0),
        context=dict(epoch='1', input=n, action=0), outcome=dict(kind='Outcome', value=dict(kind='Next', key=n)))
    base = dict(source=source, origin=dict(epoch='1', lineage=lineage, ordinal=0), declaration_end=id_(0),
        reserved_look='2', paid_look=None, next_reservation='3', rows=[],
        unscored=None, validation_from=None, collecting_validation=False, selected_training=None,
        capacity=8, signature=None, address=dict(role='Categorical',depth=1,evidence_round=0,imported=None,composition=None),
        family_count=None, declared_criterion='ExactPredictiveV1', risk_denominator='20', issued_criterion=None)
    events = []
    for kind in ['Declared', 'Buffered', 'OtherReservations', 'Opened', 'TrainingBoundary']:
        event = copy.deepcopy(base); event['kind'] = kind
        if kind != 'Declared': event['rows'] = [row(1),row(2)]
        if kind in ('OtherReservations','Opened','TrainingBoundary'): event['next_reservation'] = '7'
        if kind in ('Opened','TrainingBoundary'):
            event.update(paid_look='2',family_count='10',issued_criterion='ExactPredictiveV1 { risk: Mass { denominator: 20 } }')
        if kind == 'TrainingBoundary':
            event.update(rows=[row(1)], selected_training=1, unscored=[id_(2)])
        events.append(event)
    return events


def freshness():
    event = fixture()[-1]
    event.update(validation_from=dict(lineage=event['declaration_end']['lineage'],ordinal=3),
        collecting_validation=True, rows=[], unscored=[], issued_criterion=None)
    events=[]
    for kind in ['Selected','OldRowRefused','ActualFreshRow']:
        current=copy.deepcopy(event); current['kind']=kind
        if kind=='ActualFreshRow':
            row=copy.deepcopy(fixture()[1]['rows'][0]); row['id']['ordinal']=3
            current['rows']=[row]
        events.append(current)
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
        self.assertIn('issued criterion semantics and exact family cardinality',counts['unchecked'])
        self.assertFalse(counts['validation'])
        fresh=self.prove(freshness())
        self.assertTrue(fresh['validation'])
        self.assertEqual(fresh['buffered_rows'],1)
        self.assertIn('refused input and error absent from snapshots',fresh['unchecked'])

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
        for key,value in [('capacity',1),('signature',3),('address',dict(role='Categorical',depth=2,evidence_round=0,imported=None,composition=None)),
                          ('risk_denominator','21'),('family_count','11')]:
            data=fixture(); data[3][key]=value; cases.append(data)
        data=fixture()
        for event in data:
            for row in event['rows']: row['origin']['lineage']['authority']=9
        cases.append(data)
        for index,data in enumerate(cases):
            with self.subTest(index=index): self.prove(data,True)

    def test_selection_frontier_and_retained_owner_are_enforced(self):
        cases=[]
        data=freshness()
        for event in data: event['validation_from']['ordinal']=4
        cases.append(data)
        data=freshness(); data[-1]['rows'][0]['id']['ordinal']=2; cases.append(data)
        data=freshness(); data[-1]['rows'][0]['source']['owner']['birth']=1; cases.append(data)
        data=freshness(); data[-1]['rows'][0]['origin']['epoch']='2'; cases.append(data)
        data=freshness(); data[1]['rows']=copy.deepcopy(data[2]['rows']); cases.append(data)
        data=freshness()
        for event in data: event['capacity']=0
        cases.append(data)
        data=freshness(); data[-1]['validation_from']['ordinal']=4; cases.append(data)
        for index,data in enumerate(cases):
            with self.subTest(index=index): self.prove(data,True)

    def test_incomplete_or_unsupported_exports_do_not_gain_evidence(self):
        data=fixture(); data.pop()
        with self.assertRaisesRegex(ValueError,'complete'): emit(data)
        data=fixture(); data[0]['extra']='invented'
        with self.assertRaisesRegex(ValueError,'Expected fields'): emit(data)
        data=fixture(); data[-1]['collecting_validation']=True
        with self.assertRaisesRegex(ValueError,'phase'): emit(data)
        data=fixture(); data[1]['rows'][0]['id']['ordinal']=True
        with self.assertRaisesRegex(ValueError,'Out-of-range'): emit(data)


if __name__ == '__main__': unittest.main()
