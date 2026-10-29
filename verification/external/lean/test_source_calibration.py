"""Independent semantic corruption controls; synthetic fixtures are not runtime evidence."""
import copy
from pathlib import Path
import re
import tempfile
import unittest
from check_source_calibration_export import emit
from check_typed_programs import ROOT, check, require_success, run


def state():
    return dict(identity=1, source=None, origin=None, revision=0, last_novel=None,
                end=None, first_unretained=None, maximum=0, cells=1, support=[])


def agenda(profile):
    return dict(source_owner=0, profile=profile, current=dict(identity=2, profile=profile['identity'],
        declared=None, look=None, next_source='0', snapshot=None, materializer=None, alphabets=[]),
        workers=[], retired=[], next_look='0', next_source='0', first_unretained=None,
        halted=False, capacity=16, observations=[])


def fixture():
    line = dict(authority=5, birth=0)
    source = dict(epoch='1', raw_space='2', owner=line, shape='Event')
    origin = dict(lineage=line, epoch='1', ordinal=0)
    sample = dict(lineage=line, ordinal=0)
    context = dict(epoch='1', input=7, action=0)
    outcome = dict(kind='Outcome', value=dict(kind='Next', key=3))
    after = dict(identity=3, source=source, origin=origin, revision=1, last_novel=sample,
        end=sample, first_unretained=None, maximum=1, cells=5,
        support=[dict(context=context, values=[outcome])])
    physical = dict(kind='Physical', name='capture', input=dict(id=sample, epoch='1', raw_space='2',
        event=dict(raw_state=7, situation=7, action=0), outcome=outcome),
        projected_context=context, before=agenda(state()), after=agenda(after))
    return dict(schema='synarc-source-profile-lifecycle-v1', scope='synthetic control',
        identity_scope='fixture-local nominal classes', events=[physical])


def service_fixture():
    data = fixture(); s = data['events'][0]['after']
    data['events'] = [dict(kind='Service', name='stutter', grant=0, remaining=0,
        result=dict(kind='Ok'), before=copy.deepcopy(s), after=copy.deepcopy(s))]
    return data


def refusal_fixture():
    data = fixture(); s = data['events'][0]['after']['profile']
    data['events'] = [dict(kind='Preflight', name='capacity', before=copy.deepcopy(s),
        after=copy.deepcopy(s), refusal='Capacity', input=dict(id=dict(lineage=s['source']['owner'], ordinal=1),
        context=dict(epoch='1', input=8, action=0), outcome=dict(kind='Outcome', value=dict(kind='Next', key=4)), capacity=5))]
    return data


class SourceCalibrationTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.SourceCalibration'))

    def prove(self, data, false=False):
        source, counts = emit(data)
        with tempfile.TemporaryDirectory(prefix='verum-source-calibration-') as directory:
            path = Path(directory)/'check.lean'; path.write_text(source)
            result = check(path, '`decide_cbv` failed: the proposition evaluates to `false`' if false else None)
        if not false:
            rosters = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
            self.assertEqual(len(rosters), counts['claims'])
            for row in rosters:
                self.assertTrue(row.endswith(('does not depend on any axioms',
                    'depends on axioms: [propext]', 'depends on axioms: [propext, Quot.sound]')), row)
        return counts

    def test_projection_and_actual_refusal_are_separate_checked_scopes(self):
        self.assertEqual(self.prove(fixture())['unchecked'], {'CutLayerProjection': 1})
        self.prove(service_fixture()); self.prove(refusal_fixture())

    def test_existing_member_preserves_revision_with_a_later_end(self):
        data=fixture(); event=data['events'][0]
        event['before']=copy.deepcopy(event['after'])
        event['input']['id']=copy.deepcopy(event['input']['id']); event['input']['id']['ordinal']=1
        event['after']['profile']['end']=copy.deepcopy(event['input']['id'])
        self.prove(data)

    def test_model_rejects_false_support_revision_source_and_refusal_claims(self):
        cases=[]
        data=fixture(); data['events'][0]['after']['profile']['revision']=2; cases.append(data)
        data=fixture(); data['events'][0]['input']['outcome']=dict(kind='Outcome',value=dict(kind='Next',key=9)); cases.append(data)
        data=fixture(); data['events'][0]['after']['profile']['cells']=4; cases.append(data)
        data=fixture(); data['events'][0]['after']['profile']['origin']['epoch']='2'; cases.append(data)
        data=service_fixture(); data['events'][0]['after']['profile']['revision']=2; cases.append(data)
        data=refusal_fixture(); data['events'][0]['refusal']='Arithmetic'; cases.append(data)
        data=refusal_fixture(); data['events'][0]['input']['capacity']=16; cases.append(data)
        for i,data in enumerate(cases):
            with self.subTest(case=i): self.prove(data, false=True)

    def test_decoder_refuses_duplicate_and_unknown_evidence(self):
        data=fixture(); row=data['events'][0]['after']['profile']['support'][0]
        row['values'].append(copy.deepcopy(row['values'][0]))
        with self.assertRaisesRegex(ValueError, 'Duplicate support value'): emit(data)
        data=fixture(); rows=data['events'][0]['after']['profile']['support']; rows.append(copy.deepcopy(rows[0]))
        with self.assertRaisesRegex(ValueError, 'Duplicate support context'): emit(data)
        data=fixture(); data['events'][0]['projected_context']['action']=True
        with self.assertRaisesRegex(ValueError, 'Out-of-range integer'): emit(data)
        data=fixture(); data['events'][0]['kind']='AssumedProof'
        with self.assertRaisesRegex(ValueError, 'Unknown calibration event'): emit(data)
        data=fixture(); data['events'][0]['input']['extra']=0
        with self.assertRaisesRegex(ValueError, 'Expected fields'): emit(data)

    def test_missing_prepared_token_never_becomes_an_ownership_proof(self):
        data=refusal_fixture(); event=data['events'][0]
        event.update(kind='Commit', refusal='StalePreparation'); del event['input']
        counts=self.prove(data)
        self.assertEqual(counts['unchecked'], {'PreparedTokenAuthorization': 1})
        self.assertEqual(counts['checked'], {})

    def test_representation_laws_have_no_unexpected_axioms(self):
        names=re.findall(r'^theorem (\w+)', (ROOT/'VerumExternalReplay/SourceCalibration.lean').read_text(), re.M)
        self.assertEqual(len(names), 2)
        with tempfile.TemporaryDirectory(prefix='verum-calibration-laws-') as directory:
            path=Path(directory)/'check.lean'
            path.write_text('import VerumExternalReplay.SourceCalibration\n'+
                '\n'.join('#print axioms SourceCalibration.'+name for name in names))
            result=check(path)
        roster=[d['data'] for d in result['diagnostics'] if d['severity']=='information']
        self.assertEqual(len(roster), len(names))
        self.assertTrue(all(r.endswith(('does not depend on any axioms','depends on axioms: [propext]')) for r in roster), roster)


if __name__ == '__main__': unittest.main()
