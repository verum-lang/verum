"""Actual archived driver projections and strict corruption controls."""
import copy
import gzip
import hashlib
import json
from pathlib import Path
import re
import tempfile
import unittest
from check_guarded_driver_export import emit
from check_typed_programs import ROOT, check, require_success, run
from observer_export_replay import unique_object

FIXTURE = ROOT/'fixtures/guarded-driver-native-v1.jsonl.gz'
SHA = 'b2b29bad7c6c040d4470ea074d49031a97f4a6e15793db0b00bf33061dcb0b97'
FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'


def packet():
    content = gzip.decompress(FIXTURE.read_bytes())
    if hashlib.sha256(content).hexdigest() != SHA:
        raise ValueError('Native fixture bytes changed')
    return {'packets': [json.loads(line, object_pairs_hook=unique_object) for line in content.splitlines()]}


class GuardedDriverExportTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake','build','VerumExternalReplay.GuardedDrainTrace'))

    def prove(self, data, failure=False):
        source, _ = emit(data)
        with tempfile.TemporaryDirectory(prefix='verum-driver-export-') as directory:
            path = Path(directory)/'check.lean'; path.write_text(source)
            result = check(path, FALSE if failure else None)
            if failure:
                errors = [d for d in result['diagnostics'] if d['severity'] == 'error']
                self.assertEqual(len(errors),1,errors)
                self.assertTrue(errors[0]['data'].startswith(FALSE),errors)

    def test_archived_zero_free_and_native_selector_cases(self):
        data = packet(); data['packets'] = data['packets'][1:]
        self.prove(data)

    def test_complete_packet_roster(self):
        data = packet(); _, counts = emit(data)
        self.assertEqual(counts, dict(cases=4,services=2335,callbacks=1658,yields=2,exhaustions=1642,claims=76))
        provenance = json.loads((ROOT/'fixtures/guarded-driver-native-v1.provenance.json').read_text())
        self.assertEqual(provenance['export_sha256'],SHA)
        self.assertEqual(provenance['native_scoped_status'],'passed')
        self.assertTrue(provenance['native_original_status'].startswith('failed:'))

    def test_semantic_corruptions_rejected_by_kernel(self):
        original = packet()['packets'][1:]
        # Mutations target real archived observations, never relabel synthetic
        # traces as native evidence. Each must reach the intended false proof.
        controls = [
            (0,lambda events: events.pop(2)),
            (0,lambda events: events[1].update(before_remaining=2)),
            (0,lambda events: events[1].update(after_remaining=2)),
            (0,lambda events: events[1].update(after=copy.deepcopy(events[1]['before']))),
            (0,lambda events: events[-1].update(remaining=0)),
            (0,lambda events: events.pop()),
            (1,lambda events: events[0].update(remaining=1)),
            (1,lambda events: events[1].update(remaining=1)),
            (1,lambda events: events[-1].update(kind='yielded')),
            (2,lambda events: events[-1].update(result={'kind':'pending'})),
            (2,lambda events: events[3].update(kind='exhausted')),
        ]
        for index,(ci,mutate) in enumerate(controls):
            with self.subTest(control=index):
                case = copy.deepcopy(original[ci]); mutate(case['events'])
                self.prove({'packets':[case]},True)

    def test_schema_and_scope_rejections(self):
        original = packet()['packets'][1]
        mutations = [
            lambda p:p.update(format='unverified'),
            lambda p:p.update(scope='full native ownership proved'),
            lambda p:p['events'][1]['before'].update(level='Unknown'),
            lambda p:p['events'][1]['before'].update(ordinal=2**64),
            lambda p:p['events'][1]['after'].update(phase=-1),
            lambda p:p['events'][0].update(remaining=True),
            lambda p:p['events'][1]['result'].update(extra=1),
            lambda p:p['events'][1].update(kind='invented'),
        ]
        for index,mutate in enumerate(mutations):
            with self.subTest(control=index):
                p = copy.deepcopy(original); mutate(p)
                with self.assertRaises(ValueError): emit({'packets':[p]})
        with self.assertRaises(ValueError): emit({'packets':[original,original]})

    def test_constructive_axiom_roster(self):
        source = (ROOT/'VerumExternalReplay/GuardedDrainTrace.lean').read_text()
        names = re.findall(r'^theorem ([\w.]+)',source,re.M)+['advance','valid']
        with tempfile.TemporaryDirectory(prefix='verum-driver-axioms-') as directory:
            path=Path(directory)/'audit.lean'
            path.write_text('import VerumExternalReplay.GuardedDrainTrace\n'+
                '\n'.join('#print axioms GuardedDrainTrace.'+n for n in names))
            result=check(path)
        rows=[d['data'] for d in result['diagnostics'] if d['severity']=='information']
        self.assertEqual(len(rows),len(names))
        for row in rows:
            if 'depends on axioms: [' in row:
                self.assertLessEqual(set(row.split('depends on axioms: [')[1].removesuffix(']').split(', ')),
                                     {'propext','Quot.sound'},row)
            else:self.assertTrue(row.endswith('does not depend on any axioms'),row)


if __name__=='__main__':unittest.main()
