"""Actual native export replay, with the original native verdict kept explicit."""
import copy
import hashlib
import json
from pathlib import Path
import re
import tarfile
import unittest

from check_source_cycle_export import emit, MODELS
from lean_test_fixture import IsolatedLeanTest
from observer_export_replay import load, require, declarations


class NativeSourceCycleTests(IsolatedLeanTest):
    MODULES = tuple(MODELS)
    FIXTURES = ('fixtures/source-cycle-native-v1.json', 'fixtures/source-cycle-native-v1.tar.gz')

    @classmethod
    def setUpClass(cls):
        super().setUpClass()
        manifest = load(cls.snapshot/cls.FIXTURES[0])
        archive = cls.snapshot/cls.FIXTURES[1]
        require(hashlib.sha256(archive.read_bytes()).hexdigest() == manifest['archive_sha256'], 'Native archive hash mismatch')
        blobs = {}
        with tarfile.open(archive,'r:gz') as tf:
            members=tf.getmembers()
            require(len(members)==len(manifest['members']) and {m.name for m in members}==set(manifest['members']),
                    'Native archive roster mismatch')
            for member in members:
                require(member.isfile() and Path(member.name).name==member.name and member.size<=16*1024*1024,
                        'Invalid native archive member')
                data=tf.extractfile(member).read()
                require(hashlib.sha256(data).hexdigest()==manifest['members'][member.name],'Native archive member changed')
                blobs[member.name]=data
        receipt=json.loads(blobs['native-receipt.json'])
        provenance=json.loads(blobs['source-cycle-export-provenance.json'])
        require(provenance['receipt']['sha256']==hashlib.sha256(blobs['native-receipt.json']).hexdigest(),
                'Producer receipt mismatch')
        require(receipt['status']==provenance['whole_run_status']==manifest['native_whole_run_status'],
                'Native verdict was relabeled')
        require(receipt['source']==provenance['source'],'Producer source roster changed')
        require(hashlib.sha256(blobs['native-runner.py']).hexdigest()==provenance['runner']['sha256'],
                'Native runner changed')
        stage=next(s for s in receipt['stages'] if s['name']=='native-finite-service')
        require(stage['status']=='passed' and stage['returncode']==0,
                'Native ownership/lifecycle suite did not pass')
        text=blobs['native-finite-service.log'].decode()
        for test in (
            'source_cycle_terminal_restart_fork_and_replacement_cannot_retain_physical_authority',
            'pending_membership_keeps_terminal_u_across_admitted_restart',
            'pending_membership_refused_restart_keeps_the_captured_comparison_and_commit_chain',
        ):
            marker='test organism::observer_finite::courts::'+test+' ... '
            require(text.count(marker)==1,'Missing native ownership/lifecycle control')
            row=text.split(marker,1)[1].split('\ntest ',1)[0]
            require(row.rstrip().endswith('ok') and 'FAILED' not in row,
                    'Native ownership/lifecycle control did not pass')
        for source,item in json.loads(blobs['reviewed-sources.json']).items():
            require(hashlib.sha256(blobs[item['file']]).hexdigest()==item['sha256']==receipt['source'][source]['sha256'],
                    'Reviewed source differs from native build')
        cls.cases={}
        for item in provenance['files']:
            name=Path(item['export']['path']).name
            require(hashlib.sha256(blobs[name]).hexdigest()==item['export']['sha256'],'Export bytes changed')
            stage=next(s for s in receipt['stages'] if s['name']==item['stage'])
            require(stage['command']==item['native_command'] and item['emitting_test'] in stage['command'],
                    'Export test detached from native invocation')
            log=blobs[Path(item['log']['path']).name]
            require(hashlib.sha256(log).hexdigest()==item['log']['sha256']==stage['log']['sha256'],'Native log changed')
            # The emitting test must pass independently of the enclosing suite's
            # retained verdict. Another court's failure is never erased.
            text=log.decode();marker='test '+item['emitting_test']+' ... '
            require(text.count(marker)==1,'Missing or duplicate native exporting test')
            row=text.split(marker,1)[1].split('\ntest ',1)[0]
            require(row.rstrip().endswith('ok') and 'FAILED' not in row,'Native exporting test did not pass')
            case=name.removesuffix('.json')
            require(case not in cls.cases,'Duplicate native export')
            cls.cases[case]=json.loads(blobs[name])
        require(set(cls.cases)=={'matching_actual_action','moving_membership_and_physical_lifecycle',
                                'wrong_actual_action_equal_autonomous_successor'},'Incomplete native export roster')

    def test_all_actual_handoffs_and_constructive_axioms(self):
        total={'claims':0,'native_steps':0,'native_commits':0}
        for name,data in self.cases.items():
            with self.subTest(case=name):
                source,counts=emit(data)
                names=['FrozenObserverExport.'+n for n in re.findall(r'^theorem (\w+)',source,re.M)]
                self.assertEqual(len(names),counts['claims'])
                self.assert_axioms(self.prove(source+'\n'+'\n'.join('#print axioms '+n for n in names)),names)
                for k in total: total[k]+=counts[k]
        self.assertEqual(total,{'claims':224,'native_steps':971,'native_commits':14})

    def test_wrong_action_equal_successor_cannot_mint_an_owned_pair(self):
        data=copy.deepcopy(self.cases['wrong_actual_action_equal_autonomous_successor'])
        row=next(r for r in data['steps'] if r['operation']=='pair_native')
        dispatch=next(r for r in data['steps'] if r['operation']=='dispatch')['result']['cut']
        b=copy.deepcopy(row['before']['binding']);certificate=data['certificates'][b['certificate']]
        b['bound_cut']=row['before']['installed']['cut']
        b['state_index']=b['state_index']+1 if b['state_index']+1<len(certificate['carrier']) else certificate['closing_target']
        b['pending_sample']={'lineage':dispatch['lineage'],'ordinal':dispatch['ordinal']}
        row['after']['binding']=b
        with self.assertRaisesRegex(ValueError,'Native pairing disagrees'):
            emit(data)

    def test_actual_execution_report_corruption_reaches_kernel_rejection(self):
        data=copy.deepcopy(self.cases['matching_actual_action'])
        original,_=emit(data)
        # Preserve all endpoint states and input identities. Only the claimed
        # native VM dispatch count is corrupted in its retained receipt chain.
        for row in data['steps']:
            if row['operation'] in ('prepare','native_commit'):
                row['result']['execution']['dispatch_attempts']+=1
        altered,_=emit(data)
        equations={b.split(' : ',1)[1] for b in original.split('\n\n') if b.startswith('theorem ')}
        controls=[b for b in altered.split('\n\n') if b.startswith('theorem ') and b.split(' : ',1)[1] not in equations]
        self.assertEqual(len(controls),1)
        source=declarations(altered).replace('end FrozenObserverExport','')+controls[0]+'\nend FrozenObserverExport\n'
        self.prove(source,failure=True,expected_error='Tactic `rfl` failed')


if __name__=='__main__': unittest.main()
