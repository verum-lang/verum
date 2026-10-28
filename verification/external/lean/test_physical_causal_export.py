"""Frozen physical boundaries and controls that must reach the intended rejection."""
import gzip
import hashlib
import json
import unittest
from check_physical_causal_export import emit, key, PhysicalReplay
from check_typed_programs import ROOT, require_success, run
from replay_assertions import ReplayAssertions

STEM='physical-causal-native-v2'

def fixture():
    raw=gzip.decompress((ROOT/'fixtures'/(STEM+'.jsonl.gz')).read_bytes())
    return dict(packets=[json.loads(line) for line in raw.splitlines()])

def case(packet,kind):return next(c for c in packet['cases'] if c['kind']==kind)

class PhysicalCausalTests(ReplayAssertions,unittest.TestCase):
    emitter=staticmethod(emit)

    @classmethod
    def setUpClass(cls):require_success(run('lake','build','VerumExternalReplay.CausalJournal'))

    def test_complete_frozen_boundary(self):
        counts=self.prove(fixture())
        for k,v in dict(cases=11,guards=41,preparation_visits=28,published=6,refused=5).items():
            self.assertEqual(counts[k],v)

    def test_receipts_and_source_identity(self):
        root=ROOT/'fixtures';manifest=json.loads((root/(STEM+'.provenance.json')).read_text())
        receipt=json.loads(gzip.decompress((root/(STEM+'.receipt.json.gz')).read_bytes()))
        old=gzip.decompress((root/(STEM+'.prior-receipt.json.gz')).read_bytes());prior=json.loads(old)
        self.assertEqual(receipt['status'],'passed');self.assertEqual(prior['status'],'passed')
        self.assertEqual(receipt['prior_native']['sha256'],hashlib.sha256(old).hexdigest())
        changed=[p for p in receipt['source'] if receipt['source'][p]['sha256']!=prior['source'][p]['sha256']]
        self.assertEqual(changed,receipt['changed_sources']);self.assertEqual(len(changed),1)
        self.assertEqual(receipt['kan_binary']['sha256'],manifest['native_binary_sha256'])
        self.assertEqual(receipt['head'],manifest['native_source_commit'])
        for item in manifest['retained_files']:
            raw=gzip.decompress((root/item['file']).read_bytes())
            self.assertEqual(len(raw),item['bytes'])
            self.assertEqual(hashlib.sha256(raw).hexdigest(),item['sha256'])
            if 'source_path' in item:self.assertEqual(item['sha256'],receipt['source'][item['source_path']]['sha256'])
        raw=gzip.decompress((root/(STEM+'.jsonl.gz')).read_bytes())
        self.assertEqual(hashlib.sha256(raw).hexdigest(),manifest['export_sha256'])

    def test_semantic_forgeries_fail_in_kernel(self):
        for kind in ('target','slot','same_cut_slot_alias','reverse','missing_body','legacy','legacy_input','guard','action_width'):
            data=fixture();p=data['packets'][0];first=p['cases'][0]
            if kind=='target':
                first['invocation']['capture']['target']['Ok'][-1]=254
                first['returned_capture']=first['invocation']['capture']
                replay=PhysicalReplay();replay.packet(p)
                name=replay.captures[key(first['invocation']['capture'])]
                select=f'ActionTargetWire.materialize (2^64-1) {name} with'
            elif kind=='slot':
                first['published_slot']=999;select='(999 : Nat) = 1'
            elif kind=='same_cut_slot_alias':
                c=case(p,'stale_reservation');identity=c['concurrent_invocation']['pending']['reservation']['slot']
                c['reservation']['slot']=identity;c['returned']['slot']=identity
                source,_=emit(data)
                clauses=[b for b in source.split('\n\n') if b.startswith('theorem ') and
                         f', {identity}] : List Nat).contains {identity} = false' in b]
                self.assertEqual(len(clauses),1)
                select=clauses[0].split(' : ',1)[1].split(' := by',1)[0]
            elif kind=='reverse':
                c=case(p,'independent_reverse_completion');c['after'].reverse()
                select='(CausalJournal.observe invocation_9).toOption.map'
            elif kind=='missing_body':
                c=case(p,'missing_body');c['after']['history']['events'][0]['captured']['observation']={'schema':[7],'body':[99]}
                select='(CausalJournal.observe invocation_3).toOption.map'
            elif kind=='legacy':
                c=case(p,'legacy_confirmation')
                c['after']['history']['events'][0]['captured']={'observed':'Step','observation':None,'target':{'Err':'Missing'}}
                select='(CausalJournal.completedRows '
            elif kind=='legacy_input':
                c=case(p,'legacy_confirmation')
                c['after']['history']['events'][0]['action']=258
                c['reservation']['unpublished_history']['events'][0]['action']=258
                select='(CausalJournal.publish '
            elif kind=='guard':
                p['bindings'][0]['guard']['comparisons']['layers_pointer']=False
                source,_=emit(data)
                clauses=[b for b in source.split('\n\n') if b.startswith('theorem ') and
                         'List Bool).all id = true' in b and 'false' in b]
                self.assertEqual(len(clauses),1)
                select=clauses[0].split(' : ',1)[1].split(' := by',1)[0]
            else:
                first['callback_actions'][0]=1;select='([1] : List Nat) = [257]'
            with self.subTest(kind=kind):self.prove(data,select)

    def test_incomplete_or_ambiguous_evidence_is_refused(self):
        for kind in ('version','debug_error','missing_bind','extra_bind','broken_chain','unknown_stage','missing_case','boolean_alias','failure_action'):
            data=fixture();p=data['packets'][0]
            if kind=='version':p['format']='synarc-physical-causal-lifecycle-v1'
            elif kind=='debug_error':case(p,'unobserved')['invocation']['capture']['result']['Err']='unobserved'
            elif kind=='missing_bind':p['bindings'].pop()
            elif kind=='extra_bind':p['bindings'].append(p['bindings'][-1])
            elif kind=='broken_chain':p['cases'][0]['visits'][1]['before']['slot']=999
            elif kind=='unknown_stage':p['cases'][0]['visits'][0]['before']['stage']='Unknown'
            elif kind=='missing_case':p['cases'].pop()
            elif kind=='boolean_alias':p['bindings'][0]['guard']['comparisons']['layers_pointer']=1
            else:case(p,'unobserved')['invocation']['capture']['result']['Err']['action']=9
            with self.subTest(kind=kind),self.assertRaises(ValueError):emit(data)

if __name__=='__main__':unittest.main()
