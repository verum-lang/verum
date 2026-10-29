#!/usr/bin/env python3
"""Check frozen registration identities and invocation checkpoints.

Snapshot identities, row bindings and refusal preservation are checked. Pending
receiver payloads, roster scheduling, VM bodies and score arithmetic are not
exported here; checkpoint consistency does not prove those transitions.
"""
import json
from collections import Counter
from pathlib import Path
from acquired_export_codec import SENSORY, lineage, tag
from check_residual_queries import load_jsonl
from observer_export_replay import (array, boolean, fields, natural, require,
    sequence, module_closure, main as replay_main)

MODULES = module_closure(['InvocationOwnership', 'AtomicPublication'])
PHASES = 'Prepared Armed Invoked Completed Cancelled Unknown'.split()
SCOPE = ('native registration/invocation/receiver observations; '
         'no numeric pre-action evaluation or action-ranking claim')
RECEIVER = ('owner source origin next verified event pending runtime registration '
            'evaluation last_evaluation last_registration')
METHOD_FIELDS = {
    'Automatic::register_before': 'after grant remaining result',
    'cancel_before_callback': 'after invocation',
    'before_bind': 'invocation sample',
    'Prepared::dispatch': 'invocation',
    'physical_callback_entry': 'action invocation',
    'physical_callback_returned': 'invocation',
    'CapturedDispatch::observed': 'actual_target invocation journal_matches_invocation target_result tracked_cut',
    'Receiver::advance': 'after before grant remaining result typed_result',
    'Receiver::register_before': 'after before result',
    'duplicate_receive': 'after before result',
    'native_runtime_fork_replacement': 'after before',
    'receive_actual_completed_capture': 'after before result',
    'untracked_observed': 'actual_target after before invocation tracked_cut',
    'unknown_callback_entry': 'action invocation',
    'unknown_callback_returned': 'invocation',
    'CapturedDispatch::observed_refused': 'invocation',
    'drop_unresolved_invocation': 'after invocation',
}


def canonical(value): return json.dumps(value, sort_keys=True, separators=(',', ':'))


class Replay:
    def __init__(self):
        self.definitions = []; self.claims = {}; self.values = {}; self.counts = Counter()

    def claim(self, expression): self.claims.setdefault(expression, None)

    def opaque(self, value):
        # Exact field equality only: no semantic meaning is attributed to JSON.
        key = canonical(value)
        if key not in self.values:
            name = f'v{len(self.values)}'; self.values[key] = name
            self.definitions.append(f'def {name} : List Nat := {array(str(b) for b in key.encode())}')
        return self.values[key]

    def equal(self, left, right):
        self.claim(f'decide ({self.opaque(left)} = {self.opaque(right)})')

    def sample(self, value):
        fields(value, 'lineage ordinal'); lineage(value['lineage']); natural(value['ordinal'])
        return canonical(value)

    def invocation(self, value):
        fields(value, 'id phase'); identity = natural(value['id']); phase = tag(value['phase'], PHASES)
        previous = self.phases.get(identity)
        if previous is None:
            self.claim(f'decide (({phase} : InvocationOwnership.Phase) = .prepared)')
        else:
            self.claim(f'InvocationOwnership.follows {previous} {phase}')
        self.phases[identity] = phase; self.counts['phase_observations'] += 1
        return phase

    def frontier(self, cut, score):
        fields(cut, 'definition owner state value')
        d,o,s = (natural(cut[k]) for k in ('definition','owner','state'))
        require(int(d) in self.tables['definitions'] and score in self.tables['scores'],
                'Undeclared revision/score identity')
        SENSORY.value(cut['value'])
        if s in self.states: self.equal(cut['value'], self.states[s])
        else: self.states[s] = cut['value']
        return f'(⟨⟨{d},{o},{s}⟩,{natural(score)}⟩ : AtomicPublication.Frontier Nat Nat Nat Nat)'

    def receiver(self, value):
        fields(value, RECEIVER); natural(value['owner']); SENSORY.source(value['source'])
        self.sample(value['origin']); natural(value['next']); natural(value['verified'])
        if value['event'] is not None: self.sample(value['event'])
        tag(value['pending'], 'Idle Predicting Observe Learning Commit Failed'.split())
        for key in ('evaluation','last_evaluation'):
            ev = value[key]
            if ev is None: continue
            fields(ev, 'sample timing resolved_after committed'); self.sample(ev['sample'])
            require(ev['timing'] in ('registered_before','late_replay'), 'Unknown evaluation timing')
            actual = self.completed.get(self.sample(ev['sample']))
            require(actual is not None, 'Evaluation has no observed completed invocation')
            registered = sorted(self.registered.get((value['owner'],self.sample(ev['sample'])),set()))
            self.claim(f'decide (({array(natural(i) for i in registered)} : List Nat).contains '
                       f'{natural(actual)} = {str(ev["timing"]=="registered_before").lower()})')
            self.claim(f'decide ({boolean(ev["committed"])} = true → {boolean(ev["resolved_after"])} = true)')
            self.claim(f'decide ({natural(ev["sample"]["ordinal"])} ≤ {natural(value["next"])})')
        if value['last_registration'] is not None:
            fields(value['last_registration'], 'sample result'); self.sample(value['last_registration']['sample'])
            require(type(value['last_registration']['result']) is str, 'Invalid registration diagnostic')
        runtime = value['runtime']; current = None
        if runtime is not None:
            fields(runtime, 'cut protocol score'); current = self.frontier(runtime['cut'],runtime['score'])
            require(int(natural(runtime['protocol'])) in self.tables['protocols'], 'Unknown runtime protocol')
        r = value['registration']
        if r is None: return
        fields(r, 'current event invocation predicting query sample'); self.invocation(r['invocation']); self.sample(r['sample'])
        p = r['predicting']; require(p is not None and runtime is not None, 'Missing retained registration projection')
        fields(p, 'cut done event previous_score protocol query')
        retained = self.frontier(p['cut'],p['previous_score'])
        # The native registration snapshot is not separately serialized. Its
        # constructor begins this predicting owner from that same snapshot;
        # this check compares the exposed owner with the native current guard.
        self.claim(f'decide (decide ({current} = {retained}) = {boolean(r["current"])})')
        self.claim(f'!{boolean(p["done"])}')
        self.equal(p['protocol'],runtime['protocol'])
        self.equal(r['sample']['lineage'],value['origin']['lineage'])
        self.equal(r['sample']['ordinal'],value['next'])
        for name in ('query','event'):
            row = r[name]; fields(row, 'interface values'); SENSORY.source(row['interface'])
            for item in sequence(row['values']): SENSORY.value(item)
            identity = int(natural(p[name])); require(identity in self.tables['inputs'], 'Unknown input allocation')
            native = self.tables['inputs'][identity]; fields(native, 'id interface values')
            self.equal(row, {k:native[k] for k in ('interface','values')})
        self.equal(r['event']['interface'],value['source'])
        event,query = r['event'],r['query']
        self.equal(query['interface'],{**event['interface'],'shape':'Query'})
        self.equal(query['values'],[{**v,'kind':'Query'} for v in event['values']])
        if r['invocation']['phase']=='Prepared':
            self.registered.setdefault((value['owner'],self.sample(r['sample'])),set()).add(r['invocation']['id'])
        self.counts['registrations'] += 1

    def snapshot(self, value):
        if 'receivers' not in value: self.receiver(value); return
        fields(value, 'history_nodes receivers records registration_cells')
        natural(value['history_nodes']); natural(value['registration_cells'])
        owners=[]
        for receiver in sequence(value['receivers']): self.receiver(receiver); owners.append(receiver['owner'])
        require(len(set(owners))==len(owners), 'Duplicate receiver ownership')
        for r in sequence(value['records']):
            fields(r, 'entries invocation sample'); self.invocation(r['invocation']); self.sample(r['sample'])
            roster=[]
            for entry in sequence(r['entries']):
                fields(entry, 'owner result'); roster.append(int(natural(entry['owner'])))
                require(entry['result'] is None or type(entry['result']) is str,'Invalid roster diagnostic')
            require(len(set(roster))==len(roster), 'Duplicate registered owner')

    def packet(self, packet):
        fields(packet, 'case events format native scope')
        require(packet['format']=='synarc-preaction-lifecycle-v1' and packet['scope']==SCOPE,'Unknown registration format/scope')
        native = packet['native']; fields(native, 'case definitions events format inputs models protocols scope scores semantics wire_protocol')
        require(native['format']=='synarc-residual-transactions-v1' and native['case']==packet['case'], 'Mismatched native table')
        require(native['events']==[], 'Separate native transaction events need their own replay')
        self.tables={}; self.phases={}; self.states={}; self.completed={}; self.registered={}
        for name in ('definitions','inputs','scores','models','protocols'):
            entries=sequence(native[name]); table={}
            for item in entries:
                require(type(item) is dict and 'id' in item,'Missing identity table entry')
                identity=int(natural(item['id'])); require(identity not in table,'Duplicate identity')
                table[identity]=item
            self.tables[name]=table
        for event in sequence(packet['events']):
            require(type(event) is dict and event.get('method') in METHOD_FIELDS,'Unknown observed method')
            method=event['method']; expected='method '+METHOD_FIELDS[method]
            if method=='Automatic::register_before' and 'before' in event: expected+=' before'
            fields(event,expected)
            if 'result' in event: require(type(event['result']) is str,'Invalid native diagnostic')
            # Before snapshots precede the recorded operation. Afterwards every
            # shared identity must follow its earlier observed phase.
            if 'before' in event: self.snapshot(event['before'])
            if 'invocation' in event: phase=self.invocation(event['invocation'])
            if 'after' in event: self.snapshot(event['after'])
            if 'grant' in event:
                self.claim(f'decide ({natural(event["remaining"])} ≤ {natural(event["grant"])})')
            expected_phase={
                'before_bind':'prepared','Prepared::dispatch':'armed',
                'physical_callback_entry':'invoked','physical_callback_returned':'invoked',
                'unknown_callback_entry':'invoked','unknown_callback_returned':'invoked',
                'CapturedDispatch::observed_refused':'invoked',
                'cancel_before_callback':'cancelled','drop_unresolved_invocation':'unknown',
                'CapturedDispatch::observed':'completed',
            }.get(method)
            if expected_phase: self.claim(f'decide (({phase} : InvocationOwnership.Phase) = .{expected_phase})')
            if 'sample' in event: self.sample(event['sample'])
            if 'action' in event: natural(event['action'],255)
            if 'actual_target' in event and event['actual_target'] is not None:
                for byte in sequence(event['actual_target']): natural(byte,255)
            if method=='CapturedDispatch::observed':
                # This is the combined native predicate, not separately
                # exported slot identity/presence. Do not invent either input.
                self.claim(boolean(event['journal_matches_invocation']))
                target=event['target_result']
                require(type(target) is dict and len(target)==1,'Invalid target result')
                if 'Ok' in target:
                    require(target['Ok']=='wire' and event['actual_target'] is not None,
                            'Missing materialized target')
                else:
                    fields(target,'Err');tag(target['Err'],['Unobserved','Missing'])
                    self.equal(event['actual_target'],None)
                sample=self.sample(event['tracked_cut'])
                require(sample not in self.completed,'Multiple publications for one sample')
                self.completed[sample]=event['invocation']['id']
            if method in ('duplicate_receive','untracked_observed'):
                self.equal(event['before'],event['after'])
            if method=='untracked_observed': self.equal(event['tracked_cut'],None)
            if method=='native_runtime_fork_replacement':
                before,after=event['before'],event['after']
                self.claim(f'decide ({natural(before["runtime"]["cut"]["owner"])} ≠ {natural(after["runtime"]["cut"]["owner"])})')
                self.equal(before['runtime']['cut']['value'],after['runtime']['cut']['value'])
                self.equal(before['runtime']['score'],after['runtime']['score'])
            if method=='Receiver::advance':
                result=event['typed_result']; require(type(result) is dict,'Missing typed result')
                if result.get('kind') in ('pending','ready'): fields(result,'kind')
                else:
                    fields(result,'kind detail resumable'); require(result['kind']=='refused','Invalid typed result');boolean(result['resumable'])
                    detail=result['detail']; require(type(detail) is dict and type(detail.get('kind')) is str,'Missing refusal detail')
                    detail_fields={'invocation':'kind reason','target':'kind reason',
                        'capacity':'kind','yielded':'kind','work':'kind',
                        'missing_row':'kind ordinal','conflicting_row':'kind ordinal',
                        'outside_typed_export_scope':'kind diagnostic'}
                    require(detail['kind'] in detail_fields,'Unknown refusal projection')
                    fields(detail,detail_fields[detail['kind']])
                    if 'ordinal' in detail: natural(detail['ordinal'])
                    if 'diagnostic' in detail: require(type(detail['diagnostic']) is str,'Invalid diagnostic')
                    if detail['kind'] in ('invocation','target'):
                        fields(detail,'kind reason'); before,after=event['before'],event['after']
                        for k in ('runtime','registration','next','evaluation','last_evaluation'): self.equal(before[k],after[k])
                        self.claim(f'!{boolean(result["resumable"])}')
                        if detail['kind']=='invocation':
                            r=after['registration'];require(r is not None,'Missing refused registration')
                            if detail['reason']=='Stale': self.claim(f'!{boolean(r["current"])}')
                            elif detail['reason']=='ForeignInvocation':
                                actual=self.completed.get(self.sample(after['event']))
                                require(actual is not None,'No observed completed capture for refusal')
                                self.claim(f'decide ({natural(r["invocation"]["id"])} ≠ {natural(actual)})')
                            else: require(False,'Uncovered invocation refusal')
                        self.counts['preserved_refusals'] += 1
            self.counts['events'] += 1
        self.counts['cases'] += 1


def emit(data):
    fields(data,'packets'); packets=sequence(data['packets']); require(packets,'Missing native packets')
    replay=Replay();names=set()
    for packet in packets:
        require(type(packet.get('case')) is str and packet['case'] not in names,'Duplicate/invalid case')
        names.add(packet['case']);replay.packet(packet)
    lines=['import VerumExternalReplay.InvocationOwnership','import VerumExternalReplay.AtomicPublication',
           'set_option warningAsError true','set_option maxRecDepth 65536',
           'set_option maxHeartbeats 30000000','set_option cbv.maxSteps 10000000',
           'namespace FrozenObserverExport',*replay.definitions]
    claims=list(replay.claims)
    for i in range(0,len(claims),32):
        lines.append(f'theorem c{i} : ({array(claims[i:i+32])} : List Bool).all id = true := by decide_cbv')
    lines.append('end FrozenObserverExport')
    return '\n\n'.join(lines)+'\n',dict(replay.counts,conditions=len(claims),claims=(len(claims)+31)//32)


if __name__=='__main__':
    replay_main(emit=emit, modules=MODULES,
        producers=[Path(__file__).resolve(),Path(__file__).with_name('acquired_export_codec.py'),
                   Path(__file__).with_name('check_residual_queries.py')],
        scope='Supplied registration revision/score identities, query/event bindings, phase checkpoint '
              'consistency, and refusal preservation. Allocation classes are native observations. '
              'No complete pending ownership, roster scheduling, VM execution, score arithmetic, '
              'unobserved transitions or universal native memory correctness.',
        description=__doc__,export_loader=load_jsonl)
