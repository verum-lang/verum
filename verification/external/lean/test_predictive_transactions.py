"""Synthetic controls; native evidence is retained separately with provenance."""
import copy
import gzip
import hashlib
import json
import re
from pathlib import Path
import shutil
import tempfile
import unittest

from check_predictive_transactions import emit, Replay, SEMANTICS, FORECAST_LAW
from check_typed_programs import ROOT, check, require_success, run
from observer_export_replay import batches, declarations
from predictive_cursor_proofs import cursor_trace

FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'


def fixture():
    clone = copy.deepcopy
    target = [1]+[0]*7
    bank = dict(forecast_law=FORECAST_LAW,samples=0,bits=0,
        ratio=dict(numerator='1',denominator='1'),lengths=dict(total=0,counts={}),
        reference=[],residual=[])
    prediction = dict(context=[],mask=target)
    def accumulator(position):
        snapshot = clone(bank)
        if position:
            n = min(position,64)
            snapshot['residual'] = [dict(context=[],counts=[[1,0] for _ in range(n)])]
            snapshot['reference'] = [[0,1]]+[[1,0] for _ in range(n-1)]
        if position == 65:
            snapshot.update(samples=1,bits=64,lengths=dict(total=1,counts={'8':1}))
        snapshot.update(poisoned=False,invalid=None,pending=None if position==65 else
            dict(prediction=clone(prediction),target=target,position=position,bits=64))
        return snapshot
    final = accumulator(65)
    for k in ('poisoned','invalid','pending'): del final[k]
    cut = dict(definition=0,owner=0,state=0,value=0)
    runtime = dict(cut=cut,score=0,protocol=0)
    branch = dict(cut={**cut,'owner':1},score=0,protocol=0)
    sources = [dict(query=0,event=1)]
    reading = dict(cut=cut,value=1,retained_sources=sources)
    prepared = dict(before=cut,after=dict(state=1,value=1))
    forecast = {k:clone(bank[k]) for k in ('forecast_law','lengths','reference','residual')}
    forecast['prediction'] = prediction
    ticket = dict(reading=reading,event=1,previous_score=0,protocol=0,forecast=forecast)
    predicting = dict(cut=cut,query=0,event=1,previous_score=0,protocol=0,done=False)
    learning = dict(reading=reading,preparing=dict(before=cut,event=1),prepared=None,
        retained_sources=sources,previous_score=0,target=target,accumulator=accumulator(0),done=False)
    events = [dict(kind='fork',before=runtime,after=runtime,branch=branch),
        dict(kind='predicting_advance',grant=10,paid=5,
            before=dict(runtime=runtime,predicting=predicting),
            after=dict(runtime=runtime,predicting={**predicting,'done':True,'event':None}),
            result=dict(kind='ticket',value=ticket)),
        dict(kind='frozen_forecast',label='retained-before',frontier=forecast),
        dict(kind='observe',outcome=1,runtime=runtime,learning=clone(learning))]
    def advance(after,result,grant=1,paid=1):
        events.append(dict(kind='learning_advance',grant=grant,paid=paid,
            before=dict(runtime=runtime,learning=clone(learning)),
            after=dict(runtime=runtime,learning=clone(after)),result=result))
        learning.clear(); learning.update(clone(after))
    advance({**learning,'prepared':prepared},dict(kind='pending'),10,5)
    advance(clone(learning),dict(kind='work',reason='Work(())'),0,0)
    for n in range(1,66):
        advance({**learning,'accumulator':accumulator(n)},dict(kind='pending'))
    update = dict(reading=reading,prepared=prepared,retained_sources=sources,
                  previous_score=0,next_score=1,target=target)
    advance({**learning,'done':True,'reading':None,'prepared':None,'accumulator':None},
            dict(kind='update',value=update),0,0)
    after_cut = {**cut,**prepared['after']}
    after_runtime = dict(cut=after_cut,score=1,protocol=0)
    receipt = dict(reading=dict(cut=cut,value=1),committed=dict(before=cut,after=after_cut),
        retained_sources=sources,target=target,previous_score=0,next_score=1)
    events += [dict(kind='commit',before=branch,incoming_update=update,
                    result=dict(kind='refused',returned_update=update),after=branch),
        dict(kind='commit',before=runtime,incoming_update=update,
             result=dict(kind='accepted',receipt=receipt),after=after_runtime),
        dict(kind='frozen_forecast',label='retained-after',frontier=clone(forecast))]
    code = dict(steps=[dict(operation=dict(primitive='checked_add'),
        arguments=[dict(input=0),dict(input=1)])],output=0,library=[])
    binding = dict(interface=0,width=1,slots=[dict(state=True),dict(external=0)])
    encoding = dict(encoding='u64_le',width_bytes=8,padding=dict(kind='none'))
    protocol = dict(evidence='declared_fixture_callback_premise',
        prediction=dict(context=[],mask=dict(source='native_reading',**encoding)),
        target=dict(source='observed_outcome',**encoding))
    return clone(dict(packets=[dict(format='synarc-residual-transactions-v1',
        case='synthetic',scope='synthetic controls only',semantics=SEMANTICS,wire_protocol=protocol,
        definitions=[dict(id=0,initial=0,update=code,read=code,update_binding=binding,
                          read_binding={**binding,'interface':1})],
        inputs=[dict(id=0,interface=1,values=[1]),dict(id=1,interface=0,values=[1])],
        models=[dict(id=0),dict(id=1)],protocols=[dict(id=0)],
        scores=[dict(id=0,model=0,frontier=bank),dict(id=1,model=1,frontier=final)],events=events)]))


class TransactionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake','build','VerumExternalReplay.PredictiveObservation',
                            'VerumExternalReplay.AtomicPublication'))

    def prove(self,data,expected=None,select=None):
        source,counts = emit(data)
        with tempfile.TemporaryDirectory(prefix='verum-predictive-control-') as directory:
            directory = Path(directory)
            for name in ('lakefile.toml','lean-toolchain'):
                shutil.copyfile(ROOT/name,directory/name)
            library = directory/'.lake/build/lib/lean'; library.mkdir(parents=True)
            (library/'VerumExternalReplay').symlink_to(ROOT/'.lake/build/lib/lean/VerumExternalReplay',
                                                       target_is_directory=True)
            shared = directory/'TransactionDeclarations.lean'
            shared.write_text(declarations(source))
            require_success(run('lake','env','lean','-o',str(library/'TransactionDeclarations.olean'),
                                str(shared),cwd=directory))
            if expected:
                claims = [b for b in source.split('\n\n') if b.startswith('theorem ') and select in b]
                self.assertEqual(len(claims),1,claims)
                header = '\n\n'.join(b for b in source.split('\n\n') if b.startswith(
                    ('import ','open ','set_option ','namespace ')))
                parts = [('import TransactionDeclarations\n\n'+header+'\n\n'+claims[0]+
                          '\nend FrozenObserverExport\n',[])]
            else: parts = batches(source,shared_module='TransactionDeclarations')
            for i,(part,names) in enumerate(parts):
                path = Path(directory)/f'control{i}.lean'; path.write_text(part)
                result = check(path,expected,cwd=directory)
                if expected:
                    errors = [d for d in result['diagnostics'] if d['severity']=='error']
                    self.assertTrue(errors)
                    self.assertTrue(all(d['data'].startswith(FALSE) for d in errors),errors)
                else:
                    roster = [d['data'] for d in result['diagnostics'] if d['severity']=='information']
                    self.assertEqual(len(roster),len(names))
                    for row in roster:
                        self.assertTrue(row.endswith(('does not depend on any axioms',
                            'depends on axioms: [propext]', 'depends on axioms: [propext, Quot.sound]')),row)
        return source,counts

    def test_complete_transaction_and_retained_forecast(self):
        source,counts = self.prove(fixture())
        self.assertEqual(counts['accepted_commits'],1)
        self.assertEqual(counts['refused_commits'],1)
        self.assertEqual(counts['retained_forecasts'],1)
        self.assertEqual(sum(len(names) for _,names in batches(source)),counts['claims'])

    def test_frozen_native_input_and_receipt_identities(self):
        root = ROOT/'fixtures'
        provenance = json.loads((root/'predictive-transactions-v1.provenance.json').read_text())
        raw = gzip.decompress((root/'predictive-transactions-v1.jsonl.gz').read_bytes())
        receipt = gzip.decompress((root/provenance['native_receipt_file']).read_bytes())
        self.assertEqual(hashlib.sha256(raw).hexdigest(),provenance['export_sha256'])
        self.assertEqual(hashlib.sha256(receipt).hexdigest(),provenance['native_receipt_sha256'])
        packets = [json.loads(line) for line in raw.splitlines()]
        self.assertEqual(len(packets),2)
        self.assertEqual(sum(len(p['events']) for p in packets),1096)

    def test_equal_payload_does_not_allow_reused_revision_identity(self):
        raw = gzip.decompress((ROOT/'fixtures/predictive-transactions-v1.jsonl.gz').read_bytes())
        packet = json.loads(raw.splitlines()[1])
        completed = [e['result']['value'] for e in packet['events']
                     if e['kind']=='learning_advance' and e['result']['kind']=='update']
        first,second = [u['prepared']['after'] for u in completed]
        self.assertEqual(first['value'],second['value'])
        self.assertNotEqual(first['state'],second['state'])
        replaced,replacement = second['state'],first['state']
        def reuse(node):
            if isinstance(node,dict):
                if 'state' in node and 'value' in node and node['state']==replaced:
                    node['state']=replacement
                for v in node.values(): reuse(v)
            elif isinstance(node,list):
                for v in node: reuse(v)
        reuse(packet)
        source,_ = emit(dict(packets=[packet]))
        false_claims = []
        for block in source.split('\n\n'):
            if not block.startswith('theorem '): continue
            m = re.search(r'\(\[([\d, ]*)\] : List Nat\)\.contains (\d+) = false',block)
            if m and int(m[2]) in [int(v) for v in m[1].split(',') if v.strip()]:
                false_claims.append(block)
        self.assertTrue(false_claims,'reused allocation was not checked')
        with tempfile.TemporaryDirectory(prefix='verum-reused-revision-') as directory:
            path = Path(directory)/'control.lean'
            path.write_text('import VerumExternalReplay.PredictiveObservation\n'+false_claims[0])
            result = check(path,FALSE)
            errors = [d for d in result['diagnostics'] if d['severity']=='error']
            self.assertEqual(len(errors),1)
            self.assertTrue(errors[0]['data'].startswith(FALSE))

    def test_numeric_and_semantic_forgery_fails_in_kernel(self):
        for change in ('ratio','program','target_codec'):
            data = fixture(); packet = data['packets'][0]
            if change == 'ratio': packet['scores'][1]['frontier']['ratio']['numerator'] = '2'
            elif change == 'program':
                packet['definitions'][0]['read']['steps'][0]['operation']['primitive'] = 'identity'
            else: packet['wire_protocol']['target']['padding'] = dict(kind='value_mod',modulus=3,byte=0)
            selector = {'ratio':'banksAgree (learn', 'program':'evaluate p0_d0_read',
                        'target_codec':'= wire 8 3'}[change]
            with self.subTest(change=change): self.prove(data,FALSE,selector)

    def test_ambiguous_owners_and_missing_evidence_are_rejected(self):
        for change in ('format','codec','semantics','input','model','source','poison',
                       'partial_target','denial','returned','prepared','unknown_event'):
            data = fixture(); p = data['packets'][0]; events = p['events']
            if change == 'format': p['format'] = 'unknown'
            elif change == 'codec': del p['wire_protocol']
            elif change == 'semantics': p['semantics'] = 'guess from case'
            elif change == 'input': p['inputs'].append(copy.deepcopy(p['inputs'][0]))
            elif change == 'model': p['scores'][1]['model'] = 0
            elif change == 'source': events[1]['result']['value']['reading']['retained_sources'] = []
            elif change == 'poison': events[3]['learning']['accumulator']['poisoned'] = True
            elif change == 'partial_target': events[3]['learning']['accumulator']['pending']['target'] = []
            elif change == 'denial': events[5]['after']['learning']['accumulator']['bits'] = 1
            elif change == 'returned': events[-3]['result']['returned_update'] = {**events[-3]['incoming_update'],'target':[]}
            elif change == 'prepared': events[4]['after']['learning']['prepared']['before'] = dict(definition=0,owner=1,state=0,value=0)
            else: events.append(dict(kind='ignore_me'))
            with self.subTest(change=change),self.assertRaises(ValueError): emit(data)

    def test_shared_cursor_witnesses_are_checked(self):
        base = fixture()['packets'][0]['scores'][0]['frontier']
        base.update(samples=48,bits=3840,lengths=dict(total=48,counts={'10':48}),
            ratio=dict(numerator=str(2**2048),denominator=str(3**1024)),
            reference=[[48,0] for _ in range(80)],
            residual=[dict(context=[],counts=[[48,0] for _ in range(80)])])
        replay = Replay(); name = replay.bank(base)
        blocks,count = cursor_trace('hint',base,[],[],[])
        self.assertEqual(count,2)
        source = '\n\n'.join(replay.lines+blocks)+f'''
theorem bind_start : ResidualContinuation.begin {name} ⟨[],[]⟩ [] = hint_0 := by decide_cbv
theorem observed_start : banksAgree
    (privateBank (ResidualContinuation.run 0 (ResidualContinuation.begin {name} ⟨[],[]⟩ []))) {name} = true := by
  rw [bind_start,hint_step_0]
  decide_cbv
end FrozenObserverExport
'''
        needle = '⟨⟨49, 3840,'
        self.assertEqual(source.count(needle),1)
        with tempfile.TemporaryDirectory(prefix='verum-cursor-witness-') as directory:
            path = Path(directory)/'control.lean'; path.write_text(source)
            check(path)
            path.write_text(source.replace(needle,'⟨⟨50, 3840,'))
            result = check(path,FALSE)
            errors = [d for d in result['diagnostics'] if d['severity']=='error']
            self.assertEqual(len(errors),1)
            self.assertTrue(errors[0]['data'].startswith(FALSE))


if __name__ == '__main__': unittest.main()
