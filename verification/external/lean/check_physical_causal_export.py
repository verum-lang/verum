#!/usr/bin/env python3
"""Check physical capture, journal publication and observed ownership guards.

Root/pointer comparisons are native observations, not a memory-model proof.
Journal publication erases revision anchors to Unit; the full native bind guard
is checked separately from its scalar fields and observed root comparisons.
Outer preparation invariants do not certify the unexported AVL cursors.
"""
import json
from pathlib import Path
from acquired_export_codec import option, tag, wide
from check_action_capture_export import Encoder, TERMINALS
from check_residual_queries import load_jsonl
from observer_export_replay import (array, boolean, fields, natural, require, sequence,
    module_closure, main as replay_main)

MODULES = module_closure(['CausalJournal'])
CUT = 'action epoch history_end inputs lineage ordinal pose raw raw_space sensory state time'
EVENT = 'action captured epoch inputs lineage ordinal pose raw raw_space sensory state time'
CAPTURE_TYPE = 'Capture Nat (List UInt8 × List UInt8 × List UInt8)'


def key(value): return json.dumps(value,sort_keys=True,separators=(',',':'))

def opaque(value):
    # Preserve every strictly decoded metadata field. The journal contract is
    # parametric in input payloads; this encoding supplies no VM semantics.
    return '('+array(str(b) for b in key(value).encode())+' : List Nat)'

def origin(value):
    fields(value,'authority birth')
    return f'({natural(value["authority"])}, {natural(value["birth"])})'


class PhysicalReplay:
    def __init__(self):
        self.encoder=Encoder(2**16-1)
        self.definitions=[];self.claims=[];self.captures={};self.slots={};self.propositions=set()
        self.allocations=set()
        self.counts=dict(cases=0,capture_views=0,guards=0,preparation_visits=0,
                         captured_publications=0,legacy_publications=0,refused=0)

    def claim(self,proposition,proof='decide_cbv'):
        # Repeated snapshots reuse exactly the same proposition, never a claim
        # selected by case label or by a lossy hash of the observed payload.
        if (proposition,proof) in self.propositions:return
        self.propositions.add((proposition,proof))
        self.claims.append(f'theorem c{len(self.claims)} : {proposition} := by {proof}')

    def observed(self,value):
        if type(value) is str:return tag(value,['Step','Refused'])
        fields(value,'Terminal');t=value['Terminal'];fields(t,'kind resumption')
        return f'(.terminal {tag(t["kind"],TERMINALS)} {tag(t["resumption"],["Resumed","Stopped"])})'

    def result(self,value):
        require(type(value) is dict and len(value)==1,'Expected structured capture result')
        if 'Ok' in value:
            v=value['Ok'];self.observed(v)
            v=dict(kind=v) if type(v) is str else dict(kind='Terminal',terminal=v['Terminal']['kind'],resumption=v['Terminal']['resumption'])
            return dict(kind='Ok',value=v)
        require('Err' in value,'Unknown capture result')
        v=value['Err'];fields(v,'action cause context operation stage terminal terminal_scope')
        op=v['operation']
        if type(op) is dict:
            fields(op,'Act');natural(op['Act'],2**16-1)
            require(v['action']==op['Act'],'Failure action accessor disagrees with operation')
            op=dict(kind='Act',action=op['Act'])
        else:
            tag(op,['Begin','Checkpoint','Restore']);require(v['action'] is None,'Control failure invents an action')
            op=dict(kind=op)
        return dict(kind='Err',value={**{k:v[k] for k in ('cause','context','stage','terminal','terminal_scope')},'operation':op})

    def capture(self,value):
        fields(value,'action observation result resumed target')
        canonical=key(value)
        if canonical in self.captures:return self.captures[canonical]
        normalized={**value,'result':self.result(value['result'])};del normalized['target']
        name=f'capture_{len(self.captures)}';self.captures[canonical]=name
        self.definitions.append(f'def {name} : {CAPTURE_TYPE} := {self.encoder.capture(normalized)}')
        self.claim(f'ActionCapture.check {name} = .ok {name}','rfl')
        target=value['target'];require(type(target) is dict and len(target)==1,'Invalid target result')
        if 'Ok' in target:encoded='(none,'+self.encoder.wire(target['Ok'])+')'
        else:
            fields(target,'Err');encoded='(some '+tag(target['Err'],['Unobserved','Missing'])+',[])'
        self.claim(f'(match ActionTargetWire.materialize (2^64-1) {name} with '
                   f'| .ok bytes => (none,bytes) | .error reason => (some reason,[])) = {encoded}')
        self.counts['capture_views']+=1
        return name

    def cut(self,value,event=False):
        fields(value,EVENT if event else CUT)
        origin(value['lineage']);fields(value['time'],'episode life_age')
        for n in value['time'].values():natural(n)
        for k in ('ordinal','raw','state'):natural(value[k])
        natural(value['action'],2**16-1);wide(value['epoch']);wide(value['raw_space'])
        option(value['pose'],lambda n:natural(n,255));option(value['sensory'],self.encoder.sensory)
        for pair in sequence(value['inputs']):
            require(type(pair) is list and len(pair)==2,'Invalid layer input');wide(pair[0]);natural(pair[1])
        if not event:natural(value['history_end'])
        return opaque({k:v for k,v in value.items() if k not in ('history_end','captured')})

    def outcome(self,value):
        fields(value,'observed observation target')
        observed=self.observed(value['observed']);body=option(value['observation'],self.encoder.sensory)
        # A semantic view of the recorded outcome reuses the target codec. It
        # is not a fabricated dispatch or a claim of additional native calls.
        synthetic=dict(action=0,result=dict(Ok=value['observed']),observation=value['observation'],resumed=None,target=value['target'])
        self.capture(synthetic)
        return f'⟨{observed},{body}⟩'

    def history(self,value):
        fields(value,'end events lineage');o=origin(value['lineage']);events=sequence(value['events'])
        self.claim(f'({natural(value["end"])} : Nat) = {len(events)}')
        rows=[]
        for i,event in enumerate(events):
            payload=self.cut(event,True)
            self.claim(f'({origin(event["lineage"])} : Nat × Nat) = {o}')
            self.claim(f'({natural(event["ordinal"])} : Nat) = {i}')
            rows.append(f'({payload},{option(event["captured"],self.outcome)})')
        return '('+array(rows)+' : List (List Nat × Option CausalJournal.Completed))'

    def timeline(self,value):
        fields(value,'complete epoch highest_epoch history lineage ordinal pending_epoch raw_space');boolean(value['complete'])
        for k in ('epoch','highest_epoch','raw_space'):wide(value[k])
        option(value['pending_epoch'],wide)
        events=self.history(value['history'])
        self.claim(f'({origin(value["lineage"])} : Nat × Nat) = {origin(value["history"]["lineage"])}')
        self.claim(f'({natural(value["ordinal"])} : Nat) = {natural(value["history"]["end"])}')
        return events

    def reservation(self,value,fresh=False):
        fields(value,'before highest_epoch next_ordinal pending_epoch slot slot_filled unpublished_history')
        wide(value['highest_epoch']);option(value['pending_epoch'],wide)
        payload=self.cut(value['before']);natural(value['slot'])
        if fresh:
            self.claim(f'({array(str(n) for n in sorted(self.allocations))} : List Nat).contains {value["slot"]} = false')
            self.allocations.add(value['slot'])
        require(value['slot'] in self.allocations,'Reservation has no observed allocation')
        self.claim(f'({natural(value["next_ordinal"])} : Nat) = {natural(value["before"]["ordinal"])}+1')
        self.claim(f'{boolean(value["slot_filled"])} = false')
        require(self.slots.setdefault(value['slot'],value['before'])==value['before'],'Slot changed reservation payload')
        self.history(value['unpublished_history'])
        return payload

    def guard(self,value):
        fields(value,'before comparisons highest_epoch native_result owner pending_epoch')
        before=value['before'];self.cut(before);owner=value['owner']
        fields(owner,'complete epoch highest_epoch history lineage ordinal pending_epoch raw_space')
        self.timeline(owner)
        c=value['comparisons'];fields(c,'complete epoch highest_epoch history_end history_native_root layers_pointer lineage ordinal pending_epoch raw_native_root raw_space')
        scalar=dict(complete=boolean(owner['complete']))
        for k,encode in (('epoch',wide),('raw_space',wide),('lineage',origin),('ordinal',natural)):
            scalar[k]=f'decide ({encode(before[k])} = {encode(owner[k])})'
        scalar['history_end']=f'decide ({natural(before["history_end"])} = {natural(owner["history"]["end"])})'
        scalar['highest_epoch']=f'decide ({wide(value["highest_epoch"])} = {wide(owner["highest_epoch"])})'
        scalar['pending_epoch']=f'decide (({option(value["pending_epoch"],wide)} : Option Nat) = {option(owner["pending_epoch"],wide)})'
        for k,expression in scalar.items():self.claim(f'({expression}) = {boolean(c[k])}')
        self.claim(f'({array(boolean(c[k]) for k in sorted(c))} : List Bool).all id = {boolean(value["native_result"])}')
        self.counts['guards']+=1

    def consume_binding(self,reservation,owner,accepted):
        matches=[i for i,b in enumerate(self.bindings) if b['guard']['before']==reservation['before']
            and b['guard']['owner']==owner and b['guard']['highest_epoch']==reservation['highest_epoch']
            and b['guard']['pending_epoch']==reservation['pending_epoch']]
        require(len(matches)==1,'Missing or ambiguous actual bind observation')
        b=self.bindings.pop(matches[0]);self.claim(f'{boolean(b["accepted"])} = {str(accepted).lower()}')

    def invocation(self,value,after,accepted,fresh=False):
        fields(value,'capture pending requested');natural(value['requested'],2**16-1)
        pending=value['pending'];fields(pending,'owner reservation');r=pending['reservation']
        payload=self.reservation(r,fresh);prior=self.timeline(pending['owner']);actual=self.timeline(after)
        self.consume_binding(r,{**pending['owner'],'complete':True},True)
        self.claim(f'{boolean(pending["owner"]["complete"])} = false')
        self.claim(f'({natural(value["requested"])} : Nat) = {natural(r["before"]["action"])}')
        self.claim(f'{self.history(r["unpublished_history"])} = {prior}++[({payload},none)]')
        self.claim(f'({natural(pending["owner"]["ordinal"])} : Nat) = {natural(r["before"]["ordinal"])}')
        self.claim(f'({origin(pending["owner"]["lineage"])} : Nat × Nat) = {origin(r["before"]["lineage"])}')
        c=self.capture(value['capture']);name=f'invocation_{self.counts["captured_publications"]+self.counts["refused"]}'
        self.definitions.append(f'def {name} : CausalJournal.Invocation Unit Nat (List Nat) Nat (List UInt8 × List UInt8 × List UInt8) := '
            f'⟨⟨⟨(),false,{prior}⟩,⟨(),(),{r["slot"]},{payload},{value["requested"]}⟩⟩,{c}⟩')
        if accepted:
            self.claim(f'(CausalJournal.observe {name}).toOption.map (fun p => (p.journal.events,p.journal.complete,p.slot)) = '
                       f'some ({actual},{boolean(after["complete"])},{r["slot"]})')
            self.claim(f'({natural(after["ordinal"])} : Nat) = {natural(r["next_ordinal"])}')
            self.claim(f'({origin(after["lineage"])} : Nat × Nat) = {origin(r["before"]["lineage"])}')
            self.claim(f'{opaque(after)} = {opaque({**pending["owner"],"complete":True,"history":after["history"],"ordinal":r["next_ordinal"]})}')
            self.counts['captured_publications']+=1
        else:
            self.claim(f'(CausalJournal.observe {name}).toOption.isNone = true')
            self.claim(f'{opaque(after)} = {opaque(pending["owner"])}')
            self.counts['refused']+=1
        return r

    def progress(self,value):
        fields(value,'before bump highest_epoch history_ready history_update_pending next_ordinal pending_epoch poisoned raw_ready slot slot_filled stage')
        wide(value['highest_epoch']);option(value['pending_epoch'],wide)
        self.cut(value['before']);natural(value['slot']);natural(value['next_ordinal'])
        for k in ('poisoned','slot_filled'):self.claim(f'{boolean(value[k])} = false')
        stage=value['stage'];layers=len(value['before']['inputs'])
        if type(stage) is str:
            require(stage in ('History','Raw','Complete'),'Unknown reservation stage')
            rank={'History':0,'Raw':1,'Complete':2+2*layers}[stage]
        else:
            fields(stage,'Layer');fields(stage['Layer'],'index action')
            index=int(natural(stage['Layer']['index']));boolean(stage['Layer']['action'])
            require(index<=layers,'Layer cursor beyond input count')
            rank=2+2*index+int(stage['Layer']['action'])
        self.claim(f'{boolean(value["history_update_pending"])} = {str(stage=="History").lower()}')
        self.claim(f'{boolean(value["history_ready"])} = {str(stage!="History").lower()}')
        self.claim(f'{boolean(value["raw_ready"])} = {str(type(stage) is dict or stage=="Complete").lower()}')
        if value['bump'] is not None:
            b=value['bump'];fields(b,'lookup_pending ordinal update_pending')
            self.claim(f'{boolean(b["lookup_pending"])} != {boolean(b["update_pending"])}')
            self.claim(f'({natural(b["ordinal"])} : Nat) = {natural(value["before"]["ordinal"])}')
        return rank

    def visits(self,case):
        visits=sequence(case['visits']);require(visits,'Missing preparation observations')
        prior=None;slot=case['ready']['slot']
        for v in visits:
            fields(v,'after before grant guard owner paid result same_published_root same_slot')
            if prior is not None:require(v['before']==prior,'Broken preparation chain')
            self.guard(v['guard']);self.claim(f'{boolean(v["guard"]["native_result"])} = true')
            require(v['guard']['owner']==v['owner'],'Advance guard checked another owner')
            self.claim(f'{opaque(v["guard"]["before"])} = {opaque(v["before"]["before"])}')
            a,b=self.progress(v['before']),self.progress(v['after'])
            self.claim(f'({a} : Nat) ≤ {b} ∧ {b} ≤ {a}+1')
            for snap in (v['before'],v['after']):
                self.claim(f'({natural(snap["slot"])} : Nat) = {slot}')
                self.claim(f'{opaque(snap["before"])} = {opaque(case["ready"]["before"])}')
                self.claim(f'({natural(snap["next_ordinal"])} : Nat) = {natural(case["ready"]["next_ordinal"])}')
                for k in ('highest_epoch','pending_epoch'):
                    self.claim(f'{opaque(snap[k])} = {opaque(case["ready"][k])}')
            self.claim(f'({natural(v["paid"])} : Nat) ≤ {natural(v["grant"])}')
            self.claim(f'{boolean(v["same_slot"])} = true')
            self.claim(f'{boolean(v["same_published_root"])} = true')
            self.timeline(v['owner'])
            self.claim(f'{opaque(v["owner"]["history"])} = {opaque(case["original"])}')
            self.claim(f'{boolean(v["owner"]["complete"])} = true')
            require(v['result'] in ('Ok(true)','Ok(false)','Err(History(Store(Work)))'),'Unsupported preparation result')
            self.claim(f'{str(v["result"]=="Ok(true)").lower()} = {str(v["after"]["stage"]=="Complete").lower()}')
            prior=v['after'];self.counts['preparation_visits']+=1
        require(prior['stage']=='Complete','Incomplete reservation')

    def packet(self,packet):
        fields(packet,'bindings cases format scope')
        require(packet['format']=='synarc-physical-causal-lifecycle-v2','Unsupported physical lifecycle schema')
        fields(packet['scope'],'observed premises')
        require(type(packet['scope']['observed']) is str and all(type(p) is str for p in sequence(packet['scope']['premises'])),'Invalid scope')
        bindings=sequence(packet['bindings']);require(bindings,'Missing bind guards')
        self.bindings=list(bindings)
        for binding in bindings:
            fields(binding,'accepted guard');self.guard(binding['guard'])
            self.claim(f'{boolean(binding["accepted"])} = {boolean(binding["guard"]["native_result"])}')
        cases=sequence(packet['cases']);kinds=[c.get('kind') for c in cases]
        expected={'zero_unit_then_terminal','foreign_action','unobserved','missing_body','owned_terminal_failure',
            'episode_terminal_failure','foreign_terminal_failure','control_failure','stale_reservation',
            'independent_reverse_completion','legacy_confirmation'}
        require(len(kinds)==len(expected) and set(kinds)==expected,'Missing, repeated or unknown physical case')
        for c in cases:
            kind=c['kind'];self.counts['cases']+=1
            if kind=='zero_unit_then_terminal':
                fields(c,'after callback_actions direct invocation kind old_history_after original published_slot ready returned_capture returned_cut same_observation_body visits')
                self.reservation(c['direct'],True);self.reservation(c['ready'],True);self.visits(c)
                self.claim(f'({natural(c["direct"]["slot"])} : Nat) ≠ {natural(c["ready"]["slot"])}')
                self.claim(f'{opaque({k:v for k,v in c["direct"].items() if k!="slot"})} = '
                           f'{opaque({k:v for k,v in c["ready"].items() if k!="slot"})}')
                r=self.invocation(c['invocation'],c['after'],True)
                require(r==c['ready'],'Dispatch changed prepared reservation')
                require(c['returned_capture']==c['invocation']['capture'],'Completion replaced retained capture')
                self.claim(f'{opaque(c["returned_cut"])} = {opaque(r["before"])}')
                self.claim(f'{opaque(c["old_history_after"])} = {opaque(c["original"])}')
                self.claim(f'({natural(c["published_slot"])} : Nat) = {r["slot"]}')
                self.claim(f'{boolean(c["same_observation_body"])} = true')
                self.claim(f'({array(natural(a,2**16-1) for a in sequence(c["callback_actions"]))} : List Nat) = [{r["before"]["action"]}]')
            elif kind=='stale_reservation':
                fields(c,'after_bind before_bind callback_count concurrent_invocation kind reservation returned same_slot')
                self.reservation(c['reservation'],True);self.invocation(c['concurrent_invocation'],c['before_bind'],True,True)
                require(c['returned']==c['reservation'],'Refused bind changed reservation')
                self.claim(f'{opaque(c["after_bind"])} = {opaque(c["before_bind"])}')
                self.claim(f'{boolean(c["same_slot"])} = true')
                self.claim(f'({natural(c["callback_count"])} : Nat) = 1')
                self.consume_binding(c['reservation'],c['before_bind'],False)
            elif kind=='independent_reverse_completion':
                fields(c,'after callback_actions completion_order invocations kind')
                for k in ('after','completion_order','invocations'):require(len(sequence(c[k]))==2,'Missing independent invocation')
                a,b=[self.invocation(inv,after,True,True) for inv,after in zip(c['invocations'],c['after'])]
                self.claim(f'({natural(a["slot"])} : Nat) ≠ {natural(b["slot"])}')
                self.claim(f'({origin(a["before"]["lineage"])} : Nat × Nat) ≠ {origin(b["before"]["lineage"])}')
                self.claim(f'{opaque(c["completion_order"])} = {opaque([b["before"],a["before"]])}')
                self.claim(f'({array(natural(n,2**16-1) for n in sequence(c["callback_actions"]))} : List Nat) = [{a["before"]["action"]},{b["before"]["action"]}]')
            elif kind=='legacy_confirmation':
                fields(c,'after kind reservation returned_cut');r=c['reservation'];self.reservation(r,True)
                matches=[b for b in self.bindings if b['guard']['before']==r['before']]
                require(len(matches)==1,'Unbound legacy publication')
                owner=matches[0]['guard']['owner'];prior=self.timeline(owner)
                self.consume_binding(r,matches[0]['guard']['owner'],True)
                actual=self.timeline(c['after']);self.claim(f'{actual} = {self.history(r["unpublished_history"])}')
                pending=f'(⟨⟨(),false,{prior}⟩,⟨(),(),{r["slot"]},{self.cut(r["before"])},{r["before"]["action"]}⟩⟩ : CausalJournal.Pending Unit Nat (List Nat) Nat)'
                self.claim(f'(CausalJournal.publish {pending} none).events = {actual}')
                self.claim(f'{opaque(c["after"])} = {opaque({**owner,"complete":True,"history":c["after"]["history"],"ordinal":r["next_ordinal"]})}')
                self.claim(f'(CausalJournal.completedRows {actual}).isNone = true')
                self.claim(f'{boolean(c["after"]["complete"])} = true')
                self.claim(f'({natural(c["after"]["ordinal"])} : Nat) = {natural(r["next_ordinal"])}')
                self.claim(f'{opaque(c["returned_cut"])} = {opaque(r["before"])}')
                self.counts['legacy_publications']+=1
            else:
                fields(c,'after before callback_actions invocation kind reservation result same_root slot_filled')
                result=c['result'];require(type(result) is dict and len(result)==1 and set(result)<= {'Ok','Err'},'Invalid completion result')
                accepted='Ok' in result
                r=self.invocation(c['invocation'],c['after'],accepted,True)
                require(r==c['reservation'],'Invocation changed reservation')
                self.claim(f'{opaque(c["before"])} = {opaque(c["invocation"]["pending"]["owner"]["history"])}')
                if accepted:
                    fields(result['Ok'],'capture cut');require(result['Ok']['capture']==c['invocation']['capture'],'Completion replaced capture')
                    self.claim(f'{opaque(result["Ok"]["cut"])} = {opaque(r["before"])}')
                else:require(result['Err']==c['invocation'],'Refusal lost or replaced invocation')
                self.claim(f'{boolean(c["same_root"])} = {str(not accepted).lower()}')
                self.claim(f'{boolean(c["slot_filled"])} = {str(accepted).lower()}')
                self.claim(f'({array(natural(a,2**16-1) for a in sequence(c["callback_actions"]))} : List Nat) = [{r["before"]["action"]}]')
        require(not self.bindings,'Unreplayed binding observation')


def emit(data):
    fields(data,'packets');packets=sequence(data['packets']);require(len(packets)==1,'Expected one physical export')
    r=PhysicalReplay();r.packet(packets[0])
    lines=['import VerumExternalReplay.CausalJournal','open AcquiredObserver ActionCapture',
        'set_option warningAsError true','set_option maxRecDepth 4096','namespace FrozenObserverExport',
        *r.encoder.definitions,*r.definitions,*r.claims,'end FrozenObserverExport']
    return '\n\n'.join(lines)+'\n',{**r.counts,'claims':len(r.claims)}


if __name__=='__main__':
    replay_main(emit=emit,modules=MODULES,producers=[Path(__file__).resolve(),
        *[Path(__file__).with_name(n+'.py') for n in ('check_action_capture_export','acquired_export_codec',
            'check_lifecycle_export','check_residual_queries','check_residual_export','check_joint_export',
            'residual_trace_proofs')]],
        scope=__doc__,description=__doc__,export_loader=load_jsonl)
