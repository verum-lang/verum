#!/usr/bin/env python3
"""Check observed predictive ownership, completed VM values and private scoring.

Initial score frontiers and declared callback semantics are explicit premises.
Internal native VM continuations, poison/overflow paths and physical provenance
are outside this packet; native stop diagnostics are retained without promoting
them to a proved classification.
"""
import json
from collections import Counter
from pathlib import Path

from check_joint_export import decimal
from check_residual_export import FORECAST_LAW, ns, pair, wire
from check_residual_queries import MODULES as RESIDUAL_MODULES, fraction, load_jsonl
from observer_export_replay import (array, boolean, fields, natural, require,
    sequence, variant, main as replay_main)
from predictive_cursor_proofs import cursor_trace

SEMANTICS = ('u64 identity/checked_add with exact arity; overflow and foreign shapes '
             'refuse; Meaning.calls is observational only')
MODULES = list(dict.fromkeys(RESIDUAL_MODULES + [
    'TypedPrograms', 'FiniteCode', 'FiniteExecution', 'ObserverAdmission',
    'ObserverOwnership', 'ConservativeRefinement', 'ExactTransports', 'PartialObservers',
    'RetainedExecution', 'RetainedExecutionTrace', 'RetainedNative',
    'AtomicPublication', 'PredictiveReplay', 'ResidualContinuation', 'PredictiveObservation']))
BANK_FIELDS = 'forecast_law samples bits ratio lengths reference residual'


def key(value):
    return json.dumps(value, sort_keys=True, separators=(',', ':'))


def text(value):
    require(type(value) is str and value, 'Expected nonempty description')
    return value


def codec(item, source):
    fields(item, 'source encoding width_bytes padding')
    require(item['source'] == source and item['encoding'] == 'u64_le' and
            type(item['width_bytes']) is int and item['width_bytes'] == 8,
            'Unsupported declared wire codec')
    padding = item['padding']
    require(type(padding) is dict, 'Missing explicit padding rule')
    if padding.get('kind') == 'none':
        fields(padding, 'kind'); return 0
    fields(padding, 'kind modulus byte')
    require(padding['kind'] == 'value_mod' and type(padding['byte']) is int and
            padding['byte'] == 0, 'Unsupported padding rule')
    modulus = int(natural(padding['modulus']))
    require(modulus > 0, 'Zero padding modulus')
    return modulus


def program(item):
    fields(item, 'steps output library')
    steps = []
    for step in sequence(item['steps']):
        fields(step, 'operation arguments')
        kind, op = variant(step['operation'])
        if kind == 'primitive':
            require(type(op) is str and op in ('identity', 'checked_add'), 'Unknown primitive')
            operation = '.primitive .' + ('add' if op == 'checked_add' else 'identity')
        else:
            require(kind == 'call', 'Unknown operation'); operation = '.call ' + natural(op)
        arguments = []
        for argument in sequence(step['arguments']):
            kind, value = variant(argument)
            require(kind in ('input', 'register', 'literal'), 'Unknown operand')
            arguments.append('.' + kind + ' ' + natural(value))
        steps.append(f'⟨{operation}, {array(arguments)}, none⟩')
    output = 'none' if item['output'] is None else 'some ' + natural(item['output'])
    return f'.mk {array(steps)} ({output}) {array(program(p) for p in sequence(item["library"]))}'


def binding(item):
    fields(item, 'interface width slots')
    slots = []
    for slot in sequence(item['slots']):
        kind, value = variant(slot)
        if kind == 'state':
            require(value is True, 'Invalid state slot'); slots.append('.state')
        else:
            require(kind == 'external', 'Unknown binding slot')
            require(int(natural(value)) < int(natural(item['width'])), 'Invalid external slot')
            slots.append('.external ' + natural(value))
    return f'⟨{natural(item["interface"],255)}, {natural(item["width"])}, {array(slots)}⟩'


class Replay:
    def __init__(self):
        self.lines = ['import VerumExternalReplay.PredictiveObservation',
            'import VerumExternalReplay.AtomicPublication',
            'open PredictiveObservation PredictiveReplay',
            'set_option warningAsError true', 'set_option maxRecDepth 65536',
            'set_option maxHeartbeats 30000000', 'set_option cbv.maxSteps 10000000',
            'namespace FrozenObserverExport']
        self.claims = 0
        self.memo = set()
        self.banks = {}
        self.traces = {}
        self.trace_claims = 0
        self.counts = Counter()

    def claim(self, proposition, proof='decide_cbv'):
        if proposition not in self.memo:
            self.memo.add(proposition)
            self.lines.append(f'theorem c{self.claims} : {proposition} := by\n  {proof}')
            self.claims += 1

    def bank(self, item):
        fields(item, BANK_FIELDS)
        require(item['forecast_law'] == FORECAST_LAW, 'Wrong forecast law')
        encoded = key(item)
        if encoded in self.banks:
            return self.banks[encoded]
        fields(item['lengths'], 'total counts')
        require(type(item['lengths']['counts']) is dict, 'Expected sparse length map')
        lengths = []
        for length, count in item['lengths']['counts'].items():
            length = int(decimal(length)); natural(length)
            require(int(natural(count)) > 0, 'Empty allocated length cell')
            lengths.append((length, count))
        residual, contexts = [], []
        for entry in sequence(item['residual']):
            fields(entry, 'context counts')
            context = wire(entry['context']); counts = sequence(entry['counts'])
            require(context not in contexts and counts, 'Duplicate or empty context')
            contexts.append(context)
            for position, counts in enumerate(counts):
                a,b = pair(counts)
                residual.append(f'(({ns(context)}, {position}), BinaryPrediction.fromCounts {a} {b})')
        require(contexts == sorted(contexts), 'Noncanonical native context order')
        reference = []
        for position, counts in enumerate(sequence(item['reference'])):
            a,b = pair(counts)
            reference.append(f'({position}, BinaryPrediction.fromCounts {a} {b})')
        name = f'bank_{len(self.banks)}'; self.banks[encoded] = name
        self.lines.append(f'def {name} : Bank := ⟨⟨{natural(item["samples"])}, '
            f'{natural(item["bits"])}, {fraction(item["ratio"])}, {array(residual)}, '
            f'{array(reference)}⟩, {array(f"({n}, {c})" for n,c in sorted(lengths))}⟩')
        self.claim(f'AdaptiveLength.total {name}.lengths = {natural(item["lengths"]["total"])}')
        return name

    def metadata(self, data, index):
        """Decode shared source/identity tables without inventing lifecycle events."""
        fields(data, 'format case scope semantics wire_protocol definitions inputs scores models protocols events')
        require(data['format'] == 'synarc-residual-transactions-v1', 'Unknown transaction format')
        text(data['case']); text(data['scope'])
        require(data['semantics'] == SEMANTICS, 'Unsupported native semantic premise')
        protocol = data['wire_protocol']; fields(protocol, 'evidence prediction target')
        require(protocol['evidence'] == 'declared_fixture_callback_premise', 'Unbound callback semantics')
        fields(protocol['prediction'], 'context mask')
        context = wire(protocol['prediction']['context'])
        mask_mod = codec(protocol['prediction']['mask'], 'native_reading')
        target_mod = codec(protocol['target'], 'observed_outcome')
        self.prefix = f'p{index}'
        self.definitions, self.inputs, self.scores = {}, {}, {}
        self.owners, self.states, self.live = {}, {}, {}
        self.seen_scores, self.seen_models = set(), set()
        self.sources, self.tickets, self.updates, self.frozen = set(), [], Counter(), {}
        self.predicting = self.learning = None
        self.context, self.mask_mod, self.target_mod = context, mask_mod, target_mod
        for name in ('models', 'protocols'):
            table = set()
            for entry in sequence(data[name]):
                fields(entry, 'id'); identity = int(natural(entry['id']))
                require(identity not in table, 'Duplicate nominal identity'); table.add(identity)
            require(table, 'Empty nominal registry')
            setattr(self, name, table)
        model_banks = {}
        for entry in sequence(data['scores']):
            fields(entry, 'id model frontier')
            identity = int(natural(entry['id'])); model = int(natural(entry['model']))
            require(identity not in self.scores and model in self.models, 'Invalid score identity')
            b = self.bank(entry['frontier'])
            self.claim(f'Coherent {b}')
            parts = {k:entry['frontier'][k] for k in ('lengths', 'residual', 'reference')}
            require(model_banks.setdefault(model, parts) == parts, 'Shared model banks changed')
            self.scores[identity] = (b, entry['frontier'], model)
        require(set(model_banks) == self.models, 'Unobserved model registry entry')
        for entry in sequence(data['definitions']):
            fields(entry, 'id initial update read update_binding read_binding')
            identity = int(natural(entry['id'])); natural(entry['initial'])
            require(identity not in self.definitions, 'Duplicate definition')
            prefix = f'{self.prefix}_d{identity}'; self.definitions[identity] = prefix
            for phase in ('read', 'update'):
                self.lines.append(f'def {prefix}_{phase} : FiniteCode.Code RetainedNative.Primitive Nat := '
                                  + program(entry[phase]))
                self.lines.append(f'def {prefix}_{phase}_binding : ObserverAdmission.Binding Nat := '
                                  + binding(entry[phase+'_binding']))
        require(self.definitions and self.scores, 'Missing initial owners')
        for entry in sequence(data['inputs']):
            fields(entry, 'id interface values'); identity = int(natural(entry['id']))
            require(identity not in self.inputs, 'Duplicate input owner')
            natural(entry['interface'],255)
            for v in sequence(entry['values']): natural(v)
            self.inputs[identity] = entry
    def packet(self, data, index):
        self.metadata(data, index)
        events = sequence(data['events']); require(events, 'Empty transaction trace')
        for event in events:
            self.event(event)
            self.counts['events'] += 1
        require(self.predicting is None and self.learning is None, 'Unfinished observed service')
        require(not self.tickets, 'Unconsumed observed tickets')
        require(not self.frozen, 'Unpaired retained forecast observation')
        self.counts['packets'] += 1

    def cut(self, c):
        fields(c, 'definition owner state value')
        for v in c.values(): natural(v)
        d,o,s = (c[k] for k in ('definition','owner','state'))
        require(d in self.definitions, 'Unknown definition')
        require(self.owners.setdefault(o,d) == d, 'Owner definition changed')
        require(self.states.setdefault(s,(d,c['value'])) == (d,c['value']), 'Retained state changed')
        return f'⟨{d}, {o}, {s}⟩'

    def runtime(self, item, *, observe=True):
        fields(item, 'cut score protocol'); c = self.cut(item['cut'])
        require(int(natural(item['score'])) in self.scores, 'Unknown score')
        require(int(natural(item['protocol'])) in self.protocols, 'Unknown protocol')
        owner = item['cut']['owner']
        if observe:
            require(self.live.setdefault(owner,item) == item, 'Published runtime changed without commit')
        self.seen_scores.add(item['score'])
        self.seen_models.add(self.scores[item['score']][2])
        return f'(⟨{c}, {item["score"]}⟩ : AtomicPublication.Frontier Nat Nat Nat Nat)'

    def fresh(self, identities, identity, kind):
        self.claim(f'({ns(sorted(identities))} : List Nat).contains {natural(identity)} = false')
        self.counts['fresh_'+kind+'_allocations'] += 1

    def input(self, identity):
        require(int(natural(identity)) in self.inputs, 'Unknown retained input owner')
        return self.inputs[identity]

    def vm(self, c, identity, phase, value):
        self.cut(c); supplied = self.input(identity); natural(value)
        d = self.definitions[c['definition']]
        self.claim(f'evaluate {d}_{phase} {d}_{phase}_binding {c["value"]} '
            f'{supplied["interface"]} {ns(supplied["values"])} = some {value}')

    def source(self, sources, event=None):
        pairs = []
        for row in sequence(sources):
            fields(row, 'query event'); self.input(row['query']); self.input(row['event'])
            pair = (row['query'],row['event'])
            require(pair in self.sources and pair not in pairs, 'Unobserved or duplicate source association')
            require(event is None or row['event'] == event, 'Retained event owner changed')
            pairs.append(pair)
        require(pairs, 'Missing actual input association')
        return pairs

    def reading(self, reading, sources=None):
        fields(reading, 'cut value' + (' retained_sources' if sources is None else ''))
        if sources is None: sources = reading['retained_sources']
        self.cut(reading['cut']); natural(reading['value'])
        for query,_ in self.source(sources): self.vm(reading['cut'],query,'read',reading['value'])

    def forecast(self, item, score, reading):
        fields(item, 'forecast_law prediction lengths reference residual')
        fields(item['prediction'], 'context mask')
        c,m = wire(item['prediction']['context']), wire(item['prediction']['mask'])
        self.claim(f'({ns(c)} : List Nat) = {ns(self.context)}')
        self.claim(f'({ns(m)} : List Nat) = wire 8 {self.mask_mod} {natural(reading)}')
        base, raw, _ = self.scores[score]
        # Forecasts expose model counts, not a score or ratio. Supply the latter
        # from the separately measured Score solely to compare the model part.
        projection = {**raw, **{k:item[k] for k in ('forecast_law','lengths','reference','residual')}}
        self.claim(f'banksAgree {base} {self.bank(projection)} = true')

    def ticket(self, item, before):
        fields(item, 'reading event previous_score protocol forecast')
        require(item['event'] == before['event'] and item['previous_score'] == before['previous_score']
                and item['protocol'] == before['protocol'], 'Returned ticket changed owners')
        self.sources.add((before['query'],item['event']))
        self.source(item['reading']['retained_sources'],item['event'])
        self.reading(item['reading'])
        require(item['reading']['cut'] == before['cut'], 'Ticket read a different revision')
        self.forecast(item['forecast'],item['previous_score'],item['reading']['value'])
        self.tickets.append(item)

    def prepared(self, item, reading, event):
        fields(item, 'before after'); fields(item['after'], 'state value')
        before = self.cut(item['before'])
        require(item['before'] == reading['cut'], 'Prepared/read ownership mismatch')
        state,value = item['after']['state'],item['after']['value']
        self.cut({**item['before'],**item['after']})
        self.claim(f'({natural(state)} : Nat) ≠ {item["before"]["state"]}')
        self.vm(item['before'],event,'update',value)
        return f'⟨{before}, {state}⟩'

    def accumulator(self, item, ticket, target):
        fields(item, BANK_FIELDS+' pending poisoned invalid')
        boolean(item['poisoned'])
        require(not item['poisoned'] and item['invalid'] is None,
                'Terminal accumulator needs its own native failure correspondence')
        snapshot = self.bank({k:item[k] for k in BANK_FIELDS.split()})
        target = wire(target); prediction = ticket['forecast']['prediction']
        c,m = wire(prediction['context']),wire(prediction['mask'])
        if item['pending'] is None:
            position = 8*len(target)+1
        else:
            pending = item['pending']; fields(pending,'prediction target position bits')
            require(pending['prediction'] == prediction and wire(pending['target']) == target,
                    'Pending prediction or target changed')
            position = int(natural(pending['position']))
            require(int(natural(pending['bits'])) == 8*len(target) and position <= 8*len(target),
                    'Invalid pending bit frontier')
        base = self.scores[ticket['previous_score']][0]
        start = f'(ResidualContinuation.begin {base} ⟨{ns(c)}, {ns(m)}⟩ {ns(target)})'
        trace_key = (base,c,m,target)
        if trace_key not in self.traces:
            prefix = f'cursor_{len(self.traces)}'
            hints,count = cursor_trace(prefix,self.scores[ticket['previous_score']][1],c,m,target)
            self.lines.extend(hints)
            self.lines.append(f'@[cbv_eval]\ntheorem {prefix}_start : {start} = {prefix}_0 := by decide_cbv')
            self.trace_claims += count+1; self.traces[trace_key] = prefix
        # Apply the named, already checked prefix theorem directly. Automatic
        # CBV pattern matching on a whole large rational/counter bank can recurse
        # excessively even for run 0; explicit composition avoids that search.
        self.claim(f'banksAgree (privateBank (ResidualContinuation.run {position} {start})) {snapshot} = true',
                   f'rw [{self.traces[trace_key]}_start, {self.traces[trace_key]}_step_{position}]\n  decide_cbv')
        return position

    def learning_snapshot(self, item, ticket):
        fields(item,'reading preparing prepared retained_sources previous_score target accumulator done')
        fields(item['preparing'],'before event'); boolean(item['done'])
        require(item['previous_score'] == ticket['previous_score'], 'Learning changed previous score')
        require(item['preparing']['before'] == ticket['reading']['cut'] and
                item['preparing']['event'] == ticket['event'], 'Preparation changed owned cut/input')
        require(item['retained_sources'] == ticket['reading']['retained_sources'], 'Learning sources changed')
        wire(item['target'])
        if item['done']:
            require(all(item[k] is None for k in ('reading','prepared','accumulator')), 'Consumed learning retained output')
            return None
        require(item['reading'] == ticket['reading'], 'Learning changed reading')
        if item['prepared'] is not None: self.prepared(item['prepared'],item['reading'],ticket['event'])
        return self.accumulator(item['accumulator'],ticket,item['target'])

    def update(self, item, ticket=None):
        fields(item,'reading prepared retained_sources previous_score next_score target')
        self.reading(item['reading'])
        require(item['reading']['retained_sources'] == item['retained_sources'], 'Update sources differ')
        sources = self.source(item['retained_sources'])
        require(len({e for _,e in sources}) == 1, 'Ambiguous prepared input')
        prep = self.prepared(item['prepared'],item['reading'],sources[0][1])
        for k in ('previous_score','next_score'):
            require(int(natural(item[k])) in self.scores, 'Unknown update score')
        self.claim(f'({item["next_score"]} : Nat) ≠ {item["previous_score"]}')
        self.claim(f'({self.scores[item["next_score"]][2]} : Nat) ≠ {self.scores[item["previous_score"]][2]}')
        if ticket is not None:
            self.fresh(self.seen_scores,item['next_score'],'score')
            self.fresh(self.seen_models,self.scores[item['next_score']][2],'model')
            self.seen_scores.add(item['next_score'])
            self.seen_models.add(self.scores[item['next_score']][2])
            require(item['reading'] == ticket['reading'] and item['previous_score'] == ticket['previous_score'],
                    'Update changed retained authority')
            prediction = ticket['forecast']['prediction']
            self.claim(f'banksAgree (learn {self.scores[item["previous_score"]][0]} '
                f'⟨{ns(wire(prediction["context"]))}, {ns(wire(prediction["mask"]))}⟩ '
                f'{ns(wire(item["target"]))}) {self.scores[item["next_score"]][0]} = true')
        return (f'⟨{self.cut(item["reading"]["cut"])}, {prep}, {item["previous_score"]}, '
                f'{item["next_score"]}, {ns(wire(item["target"]))}, by decide_cbv⟩')

    def event(self, row):
        require(type(row) is dict, 'Expected transaction event')
        kind = row.get('kind')
        if kind == 'fork':
            fields(row,'kind before after branch')
            before = self.runtime(row['before']); self.runtime(row['after'])
            require(row['before'] == row['after'], 'Fork changed parent')
            require(row['branch']['cut']['owner'] not in self.live, 'Fork reused live owner')
            branch = self.runtime(row['branch'])
            require(row['branch']['protocol'] == row['before']['protocol'], 'Fork changed protocol')
            self.claim(f'AtomicPublication.fork {before} {row["branch"]["cut"]["owner"]} = {branch}')
            self.counts['forks'] += 1; return
        if kind in ('predicting_advance','learning_advance'):
            fields(row,'kind grant paid before after result')
            phase = 'predicting' if kind == 'predicting_advance' else 'learning'
            fields(row['before'],'runtime '+phase); fields(row['after'],'runtime '+phase)
            self.runtime(row['before']['runtime']); self.runtime(row['after']['runtime'])
            require(row['before']['runtime'] == row['after']['runtime'], 'Private work published a runtime')
            self.claim(f'decide ({natural(row["paid"])} ≤ {natural(row["grant"])}) = true')
            result = row['result']; require(type(result) is dict, 'Expected actual result arm')
            result_kind = result.get('kind')
            if result_kind in ('native','wire','work'):
                fields(result,'kind reason'); text(result['reason'])
                self.counts['native_stop_observations' if result_kind=='native' else 'scoring_refusals'] += 1
            elif result_kind in ('pending','already_returned'): fields(result,'kind')
            else: fields(result,'kind value')
            before,after = row['before'][phase],row['after'][phase]
            if phase == 'predicting':
                for snap in (before,after):
                    fields(snap,'cut query event previous_score protocol done')
                    self.cut(snap['cut']); self.input(snap['query']); boolean(snap['done'])
                require(not before['done'], 'Consumed predicting call needs guard observations')
                if self.predicting is None:
                    runtime = row['before']['runtime']
                    require(before['cut'] == runtime['cut'] and before['previous_score'] == runtime['score']
                            and before['protocol'] == runtime['protocol'], 'New prediction has foreign authority')
                    self.input(before['event'])
                else: require(before == self.predicting, 'Predicting snapshot chain changed')
                if result_kind == 'ticket':
                    require(after == {**before,'event':None,'done':True}, 'Ticket did not consume predicting owner')
                    self.ticket(result['value'],before); self.predicting = None
                else:
                    require(result_kind in ('pending','native'), 'Unmodeled predicting outcome')
                    require(after == before, 'Retained predicting ownership changed')
                    self.predicting = after
                return
            require(self.learning is not None, 'Learning without observed ticket consumption')
            ticket,last = self.learning
            require(before == last, 'Learning snapshot chain changed')
            a = self.learning_snapshot(before,ticket)
            retained_states = set(self.states)
            b = self.learning_snapshot(after,ticket)
            require(after['target'] == before['target'], 'Learning changed complete target')
            if before['prepared'] is None:
                require(result_kind in ('pending','native') and not after['done'], 'Unexpected preparation outcome')
                require(before['accumulator'] == after['accumulator'], 'Preparation changed scoring cursor')
                if after['prepared'] is not None:
                    self.fresh(retained_states,after['prepared']['after']['state'],'state')
            elif before['accumulator']['pending'] is not None:
                require(after['prepared'] == before['prepared'] and not after['done'], 'Scoring changed prepared owner')
                if result_kind == 'work':
                    require(after == before, 'Denied atomic scoring step changed cursor')
                    self.claim(f'({natural(row["paid"])} : Nat) = 0')
                else:
                    require(result_kind == 'pending', 'Unmodeled scoring failure')
                    self.claim(f'({natural(row["paid"])} : Nat) = 1')
                    self.claim(f'({b} : Nat) = {a}+1')
            else:
                require(result_kind == 'update' and after['done'], 'Completed scorer did not return update')
                update = result['value']; self.update(update,ticket)
                require(update['target'] == before['target'] and update['prepared'] == before['prepared'],
                        'Returned update changed target/prepared owner')
                self.updates[key(update)] += 1; self.learning = None
                self.counts['completed_updates'] += 1; return
            self.learning = (ticket,after); return
        if kind == 'observe':
            fields(row,'kind outcome runtime learning'); self.runtime(row['runtime']); natural(row['outcome'])
            require(self.learning is None, 'Overlapping unlabelled learning cursors')
            learning = row['learning']
            matches = [t for t in self.tickets if t['reading'] == learning['reading'] and
                t['previous_score'] == learning['previous_score'] and t['event'] == learning['preparing']['event']]
            require(matches, 'Observe did not consume a returned ticket')
            ticket = matches[0]; self.tickets.remove(ticket)
            self.claim(f'({ns(wire(learning["target"]))} : List Nat) = wire 8 {self.target_mod} {row["outcome"]}')
            require(not learning['done'] and learning['prepared'] is None, 'Observe started after preparation')
            self.claim(f'({self.learning_snapshot(learning,ticket)} : Nat) = 0')
            self.learning = (ticket,learning); return
        if kind == 'commit':
            fields(row,'kind before incoming_update result after')
            before = self.runtime(row['before']); update = row['incoming_update']
            require(self.updates[key(update)] > 0, 'Commit used an unobserved or consumed update')
            self.updates[key(update)] -= 1
            name = f'{self.prefix}_update_{self.counts["commits"]}'
            self.lines.append(f'def {name} : AtomicPublication.Update Nat Nat Nat Nat (List Nat) := {self.update(update)}')
            after = self.runtime(row['after'],observe=False); result = row['result']
            require(row['after']['protocol'] == row['before']['protocol'], 'Commit changed protocol')
            if result.get('kind') == 'refused':
                fields(result,'kind returned_update')
                require(result['returned_update'] == update, 'Refusal lost or changed retained update')
                self.updates[key(update)] += 1
                self.claim(f'AtomicPublication.admissible {before} {name} = false')
                self.counts['refused_commits'] += 1
            else:
                fields(result,'kind receipt'); require(result['kind']=='accepted','Unknown commit result')
                receipt = result['receipt']; fields(receipt,'reading committed retained_sources target previous_score next_score')
                fields(receipt['reading'],'cut value'); fields(receipt['committed'],'before after')
                require(receipt['reading'] == {k:update['reading'][k] for k in ('cut','value')} and
                    all(receipt[k] == update[k] for k in ('retained_sources','target','previous_score','next_score')),
                    'Receipt changed transaction data')
                a,b = self.cut(receipt['committed']['before']),self.cut(receipt['committed']['after'])
                self.claim(f'AtomicPublication.admissible {before} {name} = true')
                self.claim(f'(match (AtomicPublication.commit {before} {name}).receipt with '
                    f'| .ok r => some (r.committed.before,r.committed.after) | .error _ => none) = some ({a},{b})')
                self.counts['accepted_commits'] += 1
            self.claim(f'(AtomicPublication.commit {before} {name}).current = {after}')
            self.live[row['before']['cut']['owner']] = row['after']
            self.counts['commits'] += 1; return
        if kind == 'frozen_forecast':
            fields(row,'kind label frontier'); label = text(row['label'])
            stem,sep,phase = label.rpartition('-')
            require(sep and stem and phase in ('before','after'), 'Unpaired forecast label')
            if phase == 'before':
                require(stem not in self.frozen, 'Duplicate retained forecast label')
                matches = [t for t in self.tickets if t['forecast'] == row['frontier']]
                require(matches, 'Frozen forecast was not returned by a native ticket')
                self.frozen[stem] = matches[0]
            else:
                require(stem in self.frozen, 'Missing retained forecast before observation')
                ticket = self.frozen.pop(stem)
                require(row['frontier'] == ticket['forecast'], 'Retained forecast changed')
                self.forecast(row['frontier'],ticket['previous_score'],ticket['reading']['value'])
                self.counts['retained_forecasts'] += 1
            return
        raise ValueError('Unknown transaction event')


def emit(data):
    fields(data,'packets'); packets = sequence(data['packets'])
    require(packets, 'Empty transaction export')
    replay = Replay()
    for i,packet in enumerate(packets): replay.packet(packet,i)
    replay.lines.append('attribute [cbv_opaque] ResidualContinuation.run')
    replay.lines.append('end FrozenObserverExport')
    return '\n\n'.join(replay.lines)+'\n', {**replay.counts,
        'cursor_transitions':replay.trace_claims,'claims':replay.claims+replay.trace_claims}


if __name__ == '__main__':
    replay_main(emit=emit, modules=MODULES, producers=[Path(__file__).resolve(),
        Path(__file__).with_name('check_residual_export.py'), Path(__file__).with_name('check_residual_queries.py'),
        Path(__file__).with_name('predictive_cursor_proofs.py'),
        Path(__file__).with_name('check_joint_export.py'), Path(__file__).with_name('residual_trace_proofs.py')],
        scope=__doc__, description=__doc__, export_loader=load_jsonl)
