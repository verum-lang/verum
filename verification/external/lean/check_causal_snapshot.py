#!/usr/bin/env python3
"""Replay outcome-aware scalar VM snapshots, complete histories and commits.

The declared callbacks, exported program/source association and native capture
fidelity remain premises. This checks completed observations, not internal VM
continuations, acquisition acceptance, physical dispatch or interface addresses.
"""
import copy
from pathlib import Path
import re

from check_predictive_transactions import Replay, SEMANTICS, key
from check_residual_export import ns, wire
from check_residual_queries import load_jsonl
from observer_export_replay import (array, fields, natural, require, sequence,
    module_closure, main as replay_main)
from residual_trace_proofs import trace

MODULES = module_closure(['CausalObservation','AtomicPublication'])


class CausalReplay(Replay):
    def __init__(self):
        super().__init__()
        self.lines.insert(0,'import VerumExternalReplay.CausalObservation')
        self.prefixes = {}

    def external(self, item):
        fields(item,'interface values value_wires_u64_le')
        interface = natural(item['interface'],255)
        values = [natural(v) for v in sequence(item['values'])]
        encoded = [wire(w) for w in sequence(item['value_wires_u64_le'])]
        require(len(values) == len(encoded),'Missing diagnostic input wire')
        for value,encoded in zip(values,encoded):
            self.claim(f'({ns(encoded)} : List Nat) = wire 8 0 {value}')
        return f'({interface}, {array(values)})'

    def named_input(self, identity):
        item = self.input(identity)
        return f'({item["interface"]}, {ns(item["values"])})'

    def score(self, rows, expected):
        encoded = [f'⟨{ns(r["context"])}, {ns(r["mask"])}, {ns(r["target"])}⟩' for r in rows]
        hints = trace(rows,encoded); history = ()
        state = 'ResidualPrediction.initial'
        for i,row in enumerate(rows):
            history += (tuple(tuple(row[k]) for k in ('context','mask','target')),)
            if history not in self.prefixes:
                n = len(self.prefixes); self.prefixes[history] = f'score_state_{n}'
                for block in hints[2*i:2*i+2]:
                    block = block.replace(': State :=',': ResidualPrediction.State :=')
                    block = block.replace('row (','ResidualPrediction.row (')
                    self.lines.append(re.sub(r'\b(state|transition)_(\d+)\b',lambda m:f'score_{m[1]}_{n}',block))
                self.trace_claims += 1
            state = self.prefixes[history]
        name = f'score_rows_{self.counts["scored_histories"]}'
        self.lines.append(f'def {name} : List ResidualPrediction.Row := {array(encoded)}')
        self.claim(f'ResidualPrediction.fold {name} ResidualPrediction.initial = {state}')
        self.claim(f'banksAgree ⟨{state}, AdaptiveResidual.lengths {name}⟩ {self.bank(expected)} = true')
        self.counts['scored_histories'] += 1

    def ticket_snapshot(self, ticket, runtime, query=None, event=None):
        fields(ticket,'reading event previous_score protocol forecast')
        require(ticket['previous_score']==runtime['score'] and ticket['protocol']==runtime['protocol'],
                'Ticket changed bank or protocol owner')
        reading = ticket['reading']; fields(reading,'cut value retained_sources')
        require(reading['cut']==runtime['cut'],'Ticket changed state owner')
        sources = sequence(reading['retained_sources']); require(len(sources)==1,'Expected one dispatch association')
        source = sources[0]; fields(source,'query event')
        require(source['event']==ticket['event'],'Retained event owner changed')
        self.input(source['query']); self.input(source['event'])
        self.sources.add((source['query'],source['event']))
        if query is not None:
            self.claim(f'({self.named_input(source["query"])} : CausalObservation.External) = {query}')
        if event is not None:
            self.claim(f'({self.named_input(source["event"])} : CausalObservation.External) = {event}')
        self.reading(reading)
        self.forecast(ticket['forecast'],runtime['score'],reading['value'])

    def packet(self, packet, index):
        fields(packet,'format scope source_binding histories training fresh initial initial_bank branches native_tables')
        require(packet['format']=='synarc-causal-outcome-update-snapshot-v1','Unknown causal snapshot format')
        require(all(type(packet[k]) is str and packet[k] for k in ('scope','source_binding')),'Missing scope')
        tables = copy.deepcopy(packet['native_tables']); protocol = tables['wire_protocol']
        fields(protocol,'evidence prediction outcome update_injection diagnostic_input_wires observational_counter')
        require(protocol['evidence']=='source-reviewed fixture callback premise, not a native purity proof',
                'Unspecified callback premise')
        fields(protocol['prediction'],'context mask'); context = wire(protocol['prediction']['context'])
        for k,source in (('mask','actual native reading'),('outcome','actual observed u64')):
            c = protocol['prediction']['mask'] if k=='mask' else protocol['outcome']
            require(c==dict(source=source,encoding='u64_le',width_bytes=8),'Unsupported scalar codec')
        require(protocol['update_injection']==dict(source='same encoded outcome wire',encoding='u64_from_le_bytes',
            required_width_bytes=8,on_wrong_width='Encoding',
            placement='append one value to retained pre-event external inputs; preserve interface owner'),
            'Unsupported outcome injection')
        require(protocol['diagnostic_input_wires']=='u64_le rendering of each actual typed u64 value; not a separate input protocol'
            and protocol['observational_counter']=='ENCODINGS only counts calls and cannot affect returned bytes',
            'Unknown diagnostic semantics')
        require(tables['semantics']=='u64 identity/checked_add with exact arity; overflow and foreign argument shapes refuse',
                'Unknown VM semantics')
        tables['semantics'] = SEMANTICS
        codec = dict(encoding='u64_le',width_bytes=8,padding=dict(kind='none'))
        tables['wire_protocol'] = dict(evidence='declared_fixture_callback_premise',
            prediction=dict(context=list(context),mask=dict(source='native_reading',**codec)),
            target=dict(source='observed_outcome',**codec))
        self.metadata(tables,index)
        require(len(self.definitions)==1,'Expected one exported acquired definition')
        initial = packet['initial']; origin = self.runtime(initial)
        definition = self.definitions[initial['cut']['definition']]
        initial_value = next(d['initial'] for d in tables['definitions'] if d['id']==initial['cut']['definition'])
        update = f'(CausalObservation.update {definition}_update {definition}_update_binding)'
        policy = f'{self.prefix}_causal'
        self.lines.append(f'def {policy} := CausalObservation.protocol {definition}_read {definition}_update '
                          f'{definition}_read_binding {definition}_update_binding {ns(context)}')
        self.claim(f'banksAgree {self.bank(packet["initial_bank"])} {self.scores[initial["score"]][0]} = true')
        histories = {}
        for history in sequence(packet['histories']):
            fields(history,'id origin events'); identity = natural(history['id']); natural(history['origin'])
            require(identity not in histories,'Duplicate history identity')
            rows = []
            for entry in sequence(history['events']):
                fields(entry,'kind pre_event target_wire'); require(entry['kind']=='completed','Incomplete causal history')
                rows.append(f'⟨{self.external(entry["pre_event"])}, {ns(wire(entry["target_wire"]))}⟩')
            name = f'{self.prefix}_history_{identity}'
            self.lines.append(f'def {name} : List (OutcomeTransitions.Row CausalObservation.External (List Nat)) := {array(rows)}')
            histories[identity] = (name,history)
        all_rows, physical_ids, used_histories = [], set(), set()
        candidates = []
        for phase in ('training','fresh'):
            assessment = packet[phase]; fields(assessment,'candidate rows score')
            candidates.append(natural(assessment['candidate'])); rows = []
            for row in sequence(assessment['rows']):
                fields(row,'history origin ordinal cut context query outcome_value recorded_target_wire state_before_read reading')
                h = natural(row['history']); require(h in histories,'Unknown history')
                name,history = histories[h]; used_histories.add(h)
                require(row['origin']==history['origin'],'Foreign history origin')
                identity = (natural(row['origin']),natural(row['ordinal']))
                require(identity not in physical_ids,'Repeated scored observation'); physical_ids.add(identity)
                cut = int(natural(row['cut'])); require(cut<len(history['events']),'Missing current completed row')
                self.claim(f'({natural(row["ordinal"])} : Nat) = {cut}')
                natural(row['context'])
                target = wire(row['recorded_target_wire']); actual = natural(row['outcome_value'])
                self.claim(f'({ns(target)} : List Nat) = {ns(wire(history["events"][cut]["target_wire"]))}')
                self.claim(f'({ns(target)} : List Nat) = wire 8 0 {actual}')
                query = self.external(row['query']); before = natural(row['state_before_read']); reading = natural(row['reading'])
                self.claim(f'OutcomeTransitions.evolve {update} ({name}.take {cut}) {initial_value} = some {before}')
                self.claim(f'evaluate {definition}_read {definition}_read_binding {before} '
                           f'{row["query"]["interface"]} {ns(row["query"]["values"])} = some {reading}')
                rows.append(dict(context=list(context),mask=list(int(row['reading']).to_bytes(8,'little')),target=list(target)))
            require(rows,'Empty assessment'); self.score(rows,assessment['score']); all_rows.extend(rows)
        require(len(set(candidates))==1,'Different acquisition candidates')
        require(used_histories==set(histories),'Unobserved history')
        self.score(all_rows,packet['initial_bank'])
        # This snapshot declares installation at the complete retained history.
        longest = max(histories.values(),key=lambda pair:len(pair[1]['events']))
        for name,history in histories.values():
            require(history['origin']==longest[1]['origin'],'Installation changed history origin')
            self.claim(f'{name} = {longest[0]}.take {len(history["events"])}')
        self.claim(f'OutcomeTransitions.evolve {update} {longest[0]} {initial_value} = some {initial["cut"]["value"]}')
        events = list(sequence(tables['events'])); owners = [initial['cut']['owner']]
        revisions = {initial['cut']['state']}; final_values = []
        for branch in sequence(packet['branches']):
            fields(branch,'label initial steps final final_read final_forecast'); require(type(branch['label']) is str,'Invalid label')
            current = branch['initial']; branch_start = self.runtime(current); owners.append(current['cut']['owner'])
            self.claim(f'AtomicPublication.fork {origin} {current["cut"]["owner"]} = {branch_start}')
            for step in sequence(branch['steps']):
                fields(step,'query pre_event observed_outcome_value returned_target_wire before ticket forecast completed_update_input after state_after_commit bank_before bank_after')
                require(step['before']==current,'Broken branch history')
                query,event = self.external(step['query']),self.external(step['pre_event'])
                target = wire(step['returned_target_wire']); outcome = natural(step['observed_outcome_value'])
                self.claim(f'({ns(target)} : List Nat) = wire 8 0 {outcome}')
                ticket = step['ticket']; self.ticket_snapshot(ticket,current,query,event)
                require(step['forecast']==ticket['forecast'],'Forecast replaced before learning')
                completed = step['completed_update_input']; fields(completed,'preparing prepared')
                fields(completed['preparing'],'before event')
                require(completed['preparing']['before']==current['cut'],'Completion changed cut owner')
                completed_id = completed['preparing']['event']; completed_input = self.named_input(completed_id)
                self.claim(f'CausalObservation.appendOutcome {event} {ns(target)} = some {completed_input}')
                self.prepared(completed['prepared'],ticket['reading'],completed_id)
                before = self.scores[current['score']][0]; after = step['after']; self.cut(after['cut'])
                require(after['score'] in self.scores,'Unknown result bank'); after_bank = self.scores[after['score']][0]
                self.claim(f'banksAgree {self.bank(step["bank_before"])} {before} = true')
                self.claim(f'banksAgree {self.bank(step["bank_after"])} {after_bank} = true')
                self.claim(f'(OutcomeTransitions.step {policy} ({current["cut"]["value"]}, {before}) '
                           f'⟨⟨{query},{event}⟩,{ns(target)}⟩).map (fun s => (s.1,banksAgree s.2 {after_bank})) = '
                           f'some ({natural(step["state_after_commit"])},true)')
                self.claim(f'({natural(step["state_after_commit"])} : Nat) = {after["cut"]["value"]}')
                require(events,'Missing actual commit'); commit = events.pop(0)
                require(commit['kind']=='commit' and commit['before']==current and commit['after']==after,
                        'Commit observations changed')
                incoming = commit['incoming_update']
                require(incoming['reading']==ticket['reading'] and incoming['prepared']==completed['prepared']
                        and wire(incoming['target'])==target,'Commit changed evidence')
                self.fresh(revisions,after['cut']['state'],'revision'); revisions.add(after['cut']['state'])
                self.fresh(self.seen_scores,after['score'],'score')
                self.fresh(self.seen_models,self.scores[after['score']][2],'model')
                self.updates[key(incoming)] += 1
                self.commit(commit,completed_event=completed_id)
                current = after
            require(branch['final']==current,'Final runtime changed')
            self.ticket_snapshot(branch['final_read'],current)
            require(branch['final_forecast']==branch['final_read']['forecast'],'Final forecast changed')
            final_values.append(current['cut']['value'])
        require(not events,'Unreplayed commit observation')
        require(len(final_values)==2,'Expected two outcome-order branches')
        self.claim(f'({ns(owners)} : List Nat).Nodup')
        self.claim(f'({final_values[0]} : Nat) ≠ {final_values[1]}')
        self.counts['assessment_rows'] += len(all_rows)
        self.counts['complete_history_rows'] += sum(len(h['events']) for _,h in histories.values())
        self.counts['packets'] += 1


def emit(data):
    fields(data,'packets'); packets = sequence(data['packets']); require(packets,'Empty causal snapshot')
    replay = CausalReplay()
    for index,packet in enumerate(packets): replay.packet(packet,index)
    replay.lines.append('end FrozenObserverExport')
    return '\n\n'.join(replay.lines)+'\n',{**replay.counts,'claims':replay.claims+replay.trace_claims,
        'checked_scoring_transitions':replay.trace_claims}


if __name__=='__main__':
    replay_main(emit=emit,modules=MODULES,producers=[Path(__file__).resolve(),
        *[Path(__file__).with_name(n+'.py') for n in ('check_predictive_transactions','check_residual_export',
        'check_residual_queries','check_joint_export','residual_trace_proofs','predictive_cursor_proofs')]],
        scope=__doc__,description=__doc__,export_loader=load_jsonl)
