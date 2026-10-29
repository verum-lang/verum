"""Frozen registration observations with independent false-proof controls."""
import copy
import gzip
import hashlib
import json
from pathlib import Path
import re
import tempfile
import unittest
from check_preaction_export import emit
from check_typed_programs import ROOT, check, require_success, run
from observer_export_replay import unique_object

SHA='2fc70f552ca8e83aa807d03f64035cfa3b0f54b8aa58d06365f35c6dae18a445'
FALSE='`decide_cbv` failed: the proposition evaluates to `false`'


def packet():
    content=gzip.decompress((ROOT/'fixtures/preaction-native-v1.jsonl.gz').read_bytes())
    if hashlib.sha256(content).hexdigest()!=SHA: raise ValueError('Native fixture changed')
    return {'packets':[json.loads(line,object_pairs_hook=unique_object) for line in content.splitlines()]}


class PreActionExportTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake','build','VerumExternalReplay.InvocationOwnership','VerumExternalReplay.AtomicPublication'))

    def prove(self,data,failure=False):
        source,counts=emit(data)
        with tempfile.TemporaryDirectory(prefix='verum-preaction-') as directory:
            path=Path(directory)/'check.lean';path.write_text(source)
            result=check(path,FALSE if failure else None)
        if failure:
            errors=[d for d in result['diagnostics'] if d['severity']=='error']
            self.assertTrue(errors)
            self.assertTrue(all(d['data'].startswith(FALSE) for d in errors),errors)
        return counts

    def test_frozen_packet(self):
        self.assertEqual(self.prove(packet()),dict(events=2093,phase_observations=111,
            registrations=79,preserved_refusals=3,cases=3,conditions=82,claims=3))
        p=json.loads((ROOT/'fixtures/preaction-native-v1.provenance.json').read_text())
        self.assertEqual(p['export_sha256'],SHA)
        self.assertTrue(p['native_original_status'].startswith('failed:'))

    def test_semantic_corruptions_reach_kernel(self):
        cases=packet()['packets']
        controls=[
            (1,lambda es:es[2]['after']['registration'].update(current=True)),
            (1,lambda es:es[2]['after']['runtime']['cut'].update(owner=1)),
            (1,lambda es:es[0]['after']['registration']['predicting'].update(previous_score=1)),
            (1,lambda es:es[0]['after']['registration']['predicting'].update(event=0)),
            (1,lambda es:es[0]['after']['registration']['predicting'].update(done=True)),
            (1,lambda es:es[0]['after']['registration']['sample'].update(ordinal=49)),
            (1,lambda es:es[0]['after']['registration']['query']['values'][0]['value'].update(raw=0)),
            (1,lambda es:es[6]['invocation'].update(phase='Prepared')),
            (1,lambda es:es[8].update(journal_matches_invocation=False)),
            (1,lambda es:es[14]['after'].update(next=49)),
            (2,lambda es:es[5]['invocation'].update(phase='Cancelled')),
            (2,lambda es:es[1].update(tracked_cut=es[0]['after']['registration']['sample'])),
        ]
        for index,(ci,mutate) in enumerate(controls):
            with self.subTest(control=index):
                case=copy.deepcopy(cases[ci]);mutate(case['events'])
                self.prove({'packets':[case]},True)

    def test_registered_and_late_timing_not_interchangeable(self):
        data=packet();data['packets']=data['packets'][:1]
        for event in data['packets'][0]['events']:
            after=event.get('after',{})
            if after.get('evaluation',{}):
                after['evaluation']['timing']='late_replay'
                break
        else:self.fail('Missing actual registered evaluation')
        self.prove(data,True)

    def test_schema_rejections(self):
        original=packet()['packets'][1]
        controls=[
            lambda p:p.update(format='unverified'),
            lambda p:p.update(scope='complete ownership proof'),
            lambda p:p['events'][0].update(method='invented'),
            lambda p:p['events'][0]['after']['registration'].update(extra=True),
            lambda p:p['events'][0]['after']['registration']['predicting'].update(query=999),
            lambda p:p['events'][0]['after']['registration']['invocation'].update(phase='Other'),
            lambda p:p['events'][0]['after'].update(owner=True),
            lambda p:p['native']['events'].append({'kind':'unreplayed'}),
            lambda p:p['native']['inputs'].append(copy.deepcopy(p['native']['inputs'][0])),
        ]
        for index,mutate in enumerate(controls):
            with self.subTest(control=index):
                case=copy.deepcopy(original);mutate(case)
                with self.assertRaises(ValueError):emit({'packets':[case]})

    def test_phase_laws_and_constructive_axioms(self):
        model=(ROOT/'VerumExternalReplay/InvocationOwnership.lean').read_text()
        names=re.findall(r'^theorem ([\w.]+)',model,re.M)+['step','run','interval','follows','authentic']
        with tempfile.TemporaryDirectory(prefix='verum-invocation-axioms-') as directory:
            path=Path(directory)/'audit.lean'
            path.write_text('import VerumExternalReplay.InvocationOwnership\n'+
                '\n'.join('#print axioms InvocationOwnership.'+n for n in names))
            result=check(path)
        rows=[d['data'] for d in result['diagnostics'] if d['severity']=='information']
        self.assertEqual(len(rows),len(names))
        for row in rows:
            if 'depends on axioms: [' in row:
                self.assertLessEqual(set(row.split('depends on axioms: [')[1].removesuffix(']').split(', ')),
                    {'propext','Quot.sound'},row)
            else:self.assertTrue(row.endswith('does not depend on any axioms'),row)
        for false in ['follows .unknown .completed = true','follows .cancelled .prepared = true',
                      'authentic .invoked true true = true','authentic .completed false true = true',
                      'authentic .completed true false = true']:
            with tempfile.TemporaryDirectory(prefix='verum-invocation-control-') as directory:
                path=Path(directory)/'control.lean'
                path.write_text('import VerumExternalReplay.InvocationOwnership\nopen InvocationOwnership\n'+
                    f'example : {false} := by decide_cbv\n')
                result=check(path,FALSE)
                errors=[d for d in result['diagnostics'] if d['severity']=='error']
                self.assertEqual(len(errors),1,errors)
                self.assertTrue(errors[0]['data'].startswith(FALSE),errors)


if __name__=='__main__':unittest.main()
