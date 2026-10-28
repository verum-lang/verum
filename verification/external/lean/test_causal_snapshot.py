"""Source-bound causal replay and forged history/input/ownership controls."""
import gzip
import hashlib
import json
import unittest
from check_causal_snapshot import emit
from check_residual_export import ns
from check_typed_programs import ROOT, require_success, run
from replay_assertions import ReplayAssertions

FIXTURE = ROOT/'fixtures/causal-outcome-native-v1.jsonl.gz'

def fixture():
    return dict(packets=[json.loads(line) for line in gzip.decompress(FIXTURE.read_bytes()).splitlines()])

class CausalSnapshotTests(ReplayAssertions, unittest.TestCase):
    emitter = staticmethod(emit)

    @classmethod
    def setUpClass(cls): require_success(run('lake','build','VerumExternalReplay.CausalObservation'))

    def test_complete_native_snapshot(self):
        counts=self.prove(fixture())
        self.assertEqual(counts['assessment_rows'],72)
        self.assertEqual(counts['complete_history_rows'],120)
        self.assertEqual(counts['accepted_commits'],4)
        self.assertEqual(counts['checked_scoring_transitions'],96)

    def test_native_receipts_and_source_identity(self):
        root=ROOT/'fixtures'
        manifest=json.loads((root/'causal-outcome-native-v1.provenance.json').read_text())
        self.assertEqual(hashlib.sha256(gzip.decompress(FIXTURE.read_bytes())).hexdigest(),manifest['export_sha256'])
        original=gzip.decompress((root/'causal-outcome-native-v1.receipt.json.gz').read_bytes())
        receipt=json.loads(original)
        corrected=json.loads(gzip.decompress((root/'causal-outcome-native-v1.corrected-receipt.json.gz').read_bytes()))
        self.assertEqual(corrected['prior_native']['sha256'],hashlib.sha256(original).hexdigest())
        self.assertEqual(receipt['status'],'failed')  # Preserve the runner's classification error.
        self.assertEqual(corrected['status'],'passed')
        self.assertEqual(receipt['source'],corrected['source'])
        acquisition=next(s for s in receipt['stages'] if s['name']=='acquisition')
        self.assertEqual((acquisition['status'],acquisition['returncode']),('passed',0))
        self.assertEqual(receipt['head'],manifest['native_source_commit'])
        self.assertEqual(receipt['kan_binary']['sha256'],manifest['native_binary_sha256'])
        for item in manifest['retained_files']:
            raw=gzip.decompress((root/item['file']).read_bytes())
            self.assertEqual(hashlib.sha256(raw).hexdigest(),item['sha256'])
            if 'source_path' in item:self.assertEqual(item['sha256'],receipt['source'][item['source_path']]['sha256'])

    def test_numeric_and_owner_forgeries_reach_kernel_rejection(self):
        for kind in ('history','pre_read_state','injection','reading','bank','reused_revision'):
            data=fixture();p=data['packets'][0];step=p['branches'][0]['steps'][0]
            if kind=='history':
                target=p['histories'][0]['events'][0]['target_wire'];target[0]+=1
                select=f'= {ns(target)}'
            elif kind=='pre_read_state':
                p['training']['rows'][0]['state_before_read']=1
                select='(p0_history_0.take 0)'
            elif kind=='injection':
                identity=step['completed_update_input']['preparing']['event']
                item=next(i for i in p['native_tables']['inputs'] if i['id']==identity)
                item['values'][-1]=999;select='= some (0, [83, 999])'
            elif kind=='reading':
                before=step['before']['cut']
                def corrupt(node):
                    if isinstance(node,dict):
                        if isinstance(node.get('reading'),dict) and node['reading'].get('cut')==before:
                            node['reading']['value']=3
                        for value in node.values():corrupt(value)
                    elif isinstance(node,list):
                        for value in node:corrupt(value)
                corrupt(p)
                select=f'evaluate p0_d0_read p0_d0_read_binding {before["value"]} 1 [0] = some 3'
            elif kind=='bank':
                p['native_tables']['scores'][1]['frontier']['ratio']['numerator']='1'
                source,_=emit(data)
                clauses=[b for b in source.split('\n\n') if b.startswith('theorem ') and
                         f'OutcomeTransitions.step p0_causal ({step["before"]["cut"]["value"]}, bank_0)' in b
                         and 'some (3,true)' in b]
                self.assertEqual(len(clauses),1)
                select=clauses[0].split(' : ',1)[1].split(' := by',1)[0]
            else:
                replaced=p['branches'][1]['final']['cut']['state']
                replacement=p['branches'][0]['steps'][0]['after']['cut']['state']
                self.assertEqual(p['branches'][1]['final']['cut']['value'],
                                 p['branches'][0]['steps'][0]['after']['cut']['value'])
                def reuse(node):
                    if isinstance(node,dict):
                        if 'state' in node and 'value' in node and node['state']==replaced:
                            node['state']=replacement
                        for value in node.values():reuse(value)
                    elif isinstance(node,list):
                        for value in node:reuse(value)
                reuse(p);select='([0, 1, 2, 3] : List Nat).contains 1 = false'
            with self.subTest(kind=kind):self.prove(data,select)

    def test_missing_or_conflicting_authority_is_refused(self):
        for kind in ('gap','foreign_origin','double_sample','codec','missing_commit','extra_commit','ticket_owner','prepared_owner'):
            data=fixture();p=data['packets'][0];step=p['branches'][0]['steps'][0]
            if kind=='gap':p['histories'][0]['events'][0]={'kind':'gap'}
            elif kind=='foreign_origin':p['fresh']['rows'][0]['origin']=8
            elif kind=='double_sample':p['training']['rows'].append(p['training']['rows'][0])
            elif kind=='codec':p['native_tables']['wire_protocol']['update_injection']['encoding']='host'
            elif kind=='missing_commit':p['native_tables']['events'].pop()
            elif kind=='extra_commit':p['native_tables']['events'].append(p['native_tables']['events'][-1])
            elif kind=='ticket_owner':step['ticket']['reading']['cut']['owner']=999
            else:step['completed_update_input']['preparing']['before']['owner']=999
            with self.subTest(kind=kind),self.assertRaises(ValueError):emit(data)

if __name__=='__main__':unittest.main()
