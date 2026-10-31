"""Synthetic protocol controls; these fixtures are not native execution evidence."""
import copy
import re
import unittest

from check_source_cycle_export import emit, MODELS
from lean_test_fixture import IsolatedLeanTest


def fixture(*, wrong_action=False, terminal=False, following=False):
    """A one-state semantic cycle with distinct retained state allocations."""
    value = {'kind':'Nat','value':0}
    owner = {'authority':1,'birth':2}
    floor = {'lineage':owner,'input_epoch':'3','output_epoch':'4'}
    event_source = {'epoch':'3','raw_space':'5','owner':owner,'shape':'Event'}
    query_source = {**event_source,'shape':'Query'}
    def supplied(query=False,action=0):
        payload = {'epoch':'3','raw_space':'5','owner':owner,'raw':7,'input':7,
                   'episode':0,'life_age':8,'action':None if query else action,'pose':255}
        return {'interface':query_source if query else event_source,
                'values':[{'kind':'Query' if query else 'Event','value':payload}]}
    code = {'steps':[{'operation':{'Primitive':'Identity'},'arguments':[{'Input':0}]}],
            'output':0,'library':[]}
    report = {'stop':'Complete','registers':[value],'dispatch_attempts':1,
              'expanded_attempts':1,'call_attempts':0,'halt':None}
    definition = {'identity':0,'initial':value,'update':code,'read':code,
                  'update_binding':{'interface':event_source,'width':1,'slots':['State']},
                  'read_binding':{'interface':query_source,'width':1,'slots':['State']},
                  'semantics':{'kind':'AcquiredV3','max_trace':100}}
    def cut(state, owner=0):
        return {'definition':0,'owner':owner,'state':state,'value':value}
    limits = {'states':4,'workspace':100,'execution':{'Unlimited':None}}
    certificate = {'identity':0,'definition':0,'autonomy':{'definition':0,'applies':True},
                   'limits':limits,'inputs':[supplied(),supplied(True)],'carrier':[cut(100,1)],
                   'readings':[{'cut':cut(100,1),'value':value,'execution':report}],
                   'transitions':[{'receipt':{'before':cut(100,1),'after':cut(101,1)},'execution':report}],
                   'closing_target':0,'period':1,'observation_labels':[0],'phase_codes':[0],
                   'scope':'synthetic autonomous carrier'}
    state = {'floor':floor,'clock':{'origin':0,'episode':0,'age':8,'phase':'ready'},
             'journal':{'pending_boundary':False,'history_known':True,'ordinal':10},
             'installed':{'cut':cut(1),'history':0,'origin_cut':0},
             'namespace':{'owner':0,'epoch':'4','capacity':100,'entries':[]},
             'certificate':0,'binding':None,'following_binding':None,'inspection':{'Ok':None}}
    data = {'protocol':'synarc-source-cycle-v1','case':'synthetic', 'maximum':2**64-1,
            'initialization':'synthetic checkpoint; no native provenance',
            'identity_scope':'synthetic allocation names','definitions':[definition],
            'histories':[{'identity':0,'origin':{'lineage':owner,'epoch':'3','ordinal':0}}],
            'certificates':[certificate],'samples':[{'identity':0,'lineage':owner,'ordinal':10}],'steps':[]}
    def public():
        b = state['binding']; p = state['clock']
        visible = b is not None and b['pending_sample'] is None and b['bound_cut']==state['installed']['cut'] and \
            p['phase']=='ready' and b['point']=={'origin':p['origin'],'time':{'episode':p['episode'],'age':p['age']}}
        state['inspection'] = {'Ok':None if not visible else {
            'floor':floor,'primitive_observable_period':1,'phase':0,'anchor_phase':0,
            'anchor_episode':b['anchor_point']['time']['episode'],'anchor_age':b['anchor_point']['time']['age'],
            'episode':b['point']['time']['episode'],'life_age':b['point']['time']['age']}}
    def push(op,arg,result,change=lambda:None):
        before = copy.deepcopy(state); change(); public()
        index = len(data['steps'])
        data['steps'].append(copy.deepcopy({'seq':index,'operation':op,'input':arg,'result':result,
                                          'before':before,'after':state}))
        return index
    def bind():
        p = state['clock']; point={'origin':p['origin'],'time':{'episode':p['episode'],'age':p['age']}}
        state['binding']={'certificate':0,'anchor_event':len(data['steps']),'anchor_point':point,'anchor_phase':0,
                          'point':point,'bound_cut':state['installed']['cut'],'state_index':0,'pending_sample':None}
        state['following_binding']=None
    def anchor(change):
        index=push('anchor',{'limits':limits,'quoted_work':100}, {'Ok':{'quoted':100,'allowed':100,'spent':1,'returned':99,
                   'strict_analysis':'Certified','source_cycle':None}},change)
        data['steps'][index]['result']['Ok']['source_cycle']=copy.deepcopy(state['inspection'])
    if following:
        anchor(lambda:state.update(following_binding={'certificate':0,'original_cut':cut(1),'latest_cut':cut(1),
            'history':0,'origin_cut':0,'comparison_cursor':0,'comparison_pending':True,'found_index':None,
            'shift':{'states':1,'start':0,'capped':0,'residue':0},'broken':False,'taken':False,'poisoned':False}))
    else: anchor(bind)
    prep=push('prepare',{'input':supplied(action=int(wrong_action)),'work_grant':10000,'execution_limit':{'Unlimited':None}},
        {'read':{'input':supplied(True),'cut':cut(1),'value':value,'execution':report,'name':0},
         'prepared':{'before':cut(1),'after':{'state':2,'value':value}},'execution':report,'remaining_work':9990},
        lambda:state['namespace']['entries'].append({'input':7,'value':value}))
    point={'origin':0,'time':{'episode':0,'age':8}}
    edge={'point':point,'sample':0,'transition':{'terminal':'Ended'} if terminal else {'tick':None},
          'successor':{'episode':1,'age':0} if terminal else {'episode':0,'age':9}}
    native_cut={'lineage':owner,'ordinal':10,'time':{'episode':0,'life_age':8},'raw':7,'state':7,'action':0,
                'epoch':'3','raw_space':'5','pose':255,'sensory':None,'history_end':10,'inputs':[['3',7],['4',0]]}
    def captured():
        state['clock']['phase']='inFlight';state['journal']['ordinal']=11
    dispatch=push('dispatch',{'preparation':prep,'point':point,'action':0},
        {'cut':native_cut,'receipt':edge,'capture':{'action':0,'outcome':{'terminal':{'kind':'Ended','resumption':'Stopped'}} if terminal else {'step':None}},
         'physical_point':point,'world_calls':1},captured)
    push('begin_run',None,{'Err':'PendingPhysicalTime'})
    def commit():
        state['installed']['cut']=cut(2)
        if following:
            state['following_binding']['latest_cut']=cut(2)
            state['following_binding']['shift']['capped']=1
    committed=push('native_commit',{'dispatch':dispatch,'preparation':prep},
        {'input':supplied(action=int(wrong_action)),'receipt':{'before':cut(1),'after':cut(2)},'execution':report},commit)
    def native():
        if state['binding'] is not None:
            if wrong_action: state['binding']=None
            else:
                state['binding']['bound_cut']=cut(2)
                state['binding']['pending_sample']={'lineage':owner,'ordinal':10}
    push('pair_native',{'dispatch':dispatch,'native_commit':committed},{'returned':'Unit'},native)
    def physical():
        state['clock']['age']=0 if terminal else 9
        state['clock']['phase']='terminalPending' if terminal else 'ready'
    applied=push('physical_commit',{'dispatch':dispatch},{'Ok':edge},physical)
    def paired():
        if state['binding'] is not None:
            if terminal: state['binding']=None
            else:
                state['binding']['pending_sample']=None
                state['binding']['point']={'origin':0,'time':{'episode':0,'age':9}}
    push('pair_physical',{'physical_commit':applied},{'returned':'Unit'},paired)
    if terminal:
        push('close_terminal',None,{'Ok':None},lambda:state['clock'].update(episode=1,phase='ready'))
    if following or terminal or wrong_action: anchor(bind)
    push('begin_run',None,{'Ok':None},lambda:state.update(clock={'origin':1,'episode':state['clock']['episode']+1,'age':0,'phase':'ready'},binding=None))
    anchor(bind)
    return data


class SourceCycleDecodeTests(unittest.TestCase):
    def test_disconnected_and_wrong_authority_rejected(self):
        changes=[
            ('native_commit',lambda r:r['result']['input']['values'][0]['value'].update(action=1),'Committed input replaced'),
            ('dispatch',lambda r:r['result']['cut']['inputs'].append(['3',99]),'Duplicate cut input epoch'),
            ('physical_commit',lambda r:r['result']['Ok']['point'].update(origin=9),'Applied edge differs'),
            ('pair_native',lambda r:r['after'].update(binding=None),'Native pairing disagrees'),
        ]
        for op,change,error in changes:
            with self.subTest(op=op):
                data=fixture();row=next(r for r in data['steps'] if r['operation']==op);change(row)
                with self.assertRaisesRegex(ValueError,error): emit(data)

    def test_restart_cannot_reuse_origin_or_reset_native_state(self):
        for change,error in (
            (lambda r:r['after']['clock'].update(origin=0),'Restart reused origin'),
            (lambda r:r['after']['installed']['cut'].update(state=99),'Unexpected mutation: installed'),
        ):
            data=fixture();row=next(r for r in data['steps'] if r['operation']=='begin_run' and 'Ok' in r['result'])
            change(row)
            with self.assertRaisesRegex(ValueError,error): emit(data)

    def test_following_count_is_derived_from_commits(self):
        data=fixture(following=True)
        row=next(r for r in data['steps'] if r['operation']=='native_commit')
        row['after']['following_binding']['shift']['capped']=0
        with self.assertRaisesRegex(ValueError,'Following owner did not consume exactly'):
            emit(data)

    def test_recorded_execution_cannot_end_before_pairing(self):
        data=fixture()
        end=next(i for i,r in enumerate(data['steps']) if r['operation']=='physical_commit')
        data['steps']=data['steps'][:end+1]
        with self.assertRaisesRegex(ValueError,'Unpaired physical application'):
            emit(data)

    def test_initial_ownership_cannot_be_assumed(self):
        data=fixture()
        data['steps']=data['steps'][1:]
        for i,row in enumerate(data['steps']): row['seq']=i
        with self.assertRaisesRegex(ValueError,'Initial source ownership requires'):
            emit(data)


class SourceCycleKernelTests(IsolatedLeanTest):
    MODULES = tuple(MODELS)

    def test_synthetic_complete_handoffs_and_axioms(self):
        for options in ({}, {'wrong_action':True}, {'terminal':True}, {'following':True}):
            with self.subTest(options=options):
                source,counts=emit(fixture(**options))
                names=['FrozenObserverExport.'+n for n in re.findall(r'^theorem (\w+)',source,re.M)]
                self.assertEqual(len(names),counts['claims'])
                self.assertGreater(counts['claims'],10)
                self.assert_axioms(self.prove(source+'\n'+'\n'.join('#print axioms '+n for n in names)),names)

    def test_false_actual_vm_values_fail_in_kernel(self):
        data=fixture()
        data['definitions'][0]['update']['steps'][0]['arguments']=[{'Literal':{'kind':'Nat','value':1}}]
        source,_=emit(data)
        # Several claims intentionally fail; require actual theorem failures,
        # not parser, unknown identifier or timeout acceptance.
        from observer_export_replay import batches
        failures=0
        for source,_ in batches(source,size=1):
            if 'observe (ObserverAdmission.evaluate' in source and '.update' in source:
                self.prove(source,failure=True,expected_error='Tactic `rfl` failed')
                failures+=1
        self.assertGreater(failures,0)

    def test_exported_autonomy_flag_cannot_hide_an_eager_external_read(self):
        data=fixture()
        definition=data['definitions'][0]
        definition['update_binding']['slots']=['State',{'External':0}]
        # The selected result stays State. An unused eager instruction still
        # observes the external port, so output-only dependency is insufficient.
        definition['update']=copy.deepcopy(definition['update'])
        definition['update']['steps'].append({'operation':{'Primitive':'EventAge'},'arguments':[{'Input':1}]})
        source,_=emit(data)
        from observer_export_replay import batches
        selected=[part for part,_ in batches(source,size=1) if '∀ (state : Value)' in part and 'd0_update' in part.split('theorem ')[-1]]
        self.assertEqual(len(selected),1)
        self.prove(selected[0],failure=True)


if __name__=='__main__': unittest.main()
