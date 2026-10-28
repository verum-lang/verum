#!/usr/bin/env python3
"""Replay native expressiveness witnesses against explicit programs and banks.

Snapshot owners, seed banks and declared codecs are premises, not a lifecycle
trace. Final values, complete learning, forecast projections and probability
queries are checked by Lean. Neither extension implementation nor acquisition,
physical provenance, resource admission or internal VM continuation is proved.
"""
import copy
from pathlib import Path
import re

from check_predictive_transactions import MODULES as TRANSACTION_MODULES, Replay
from check_residual_export import FORECAST_LAW, encode_banks, encode_lengths, ns, wire
from check_residual_queries import fraction, load_jsonl
from observer_export_replay import array, fields, natural, require, sequence, main as replay_main
from residual_trace_proofs import trace

MODULES = list(dict.fromkeys(TRANSACTION_MODULES + [
    'ConditionalExpressiveness', 'FrozenTraining', 'OutcomeTransitions']))


def fractions(values):
    result = []
    for value in sequence(values):
        require(len(sequence(value)) == 2, 'Expected rational pair')
        result.append(fraction(dict(numerator=value[0], denominator=value[1])))
    return result


def external(item):
    fields(item, 'interface values')
    return f'({natural(item["interface"],255)}, {array(natural(v) for v in sequence(item["values"]))})'


class WitnessReplay(Replay):
    def __init__(self):
        super().__init__()
        self.lines.insert(0, 'import VerumExternalReplay.OutcomeTransitions\n'
                            'import VerumExternalReplay.ConditionalExpressiveness')
        self.histories = {}

    def outcome_order(self, packet, index):
        fields(packet, 'kind scope query event states target_orders left_forecast right_forecast '
                       'native_owner_metadata returned_target_wires')
        require(type(packet['scope']) is str, 'Missing snapshot scope')
        metadata = packet['native_owner_metadata']
        fields(metadata, 'initial left_final right_final tables')
        tables = copy.deepcopy(metadata['tables'])
        fields(tables, 'format case scope semantics wire_protocol definitions inputs scores models protocols events')
        # This diagnostic snapshot uses null for absent padding. Translate the
        # declared codec, never a case label, to the transaction reader's schema.
        protocol = tables['wire_protocol']
        fields(protocol, 'evidence prediction target')
        fields(protocol['prediction'], 'context mask')
        for item in (protocol['prediction']['mask'], protocol['target']):
            fields(item, 'source encoding width_bytes padding')
            require(item['padding'] is None, 'Unsupported snapshot padding')
            item['padding'] = dict(kind='none')
        require(tables['inputs'] == [] and tables['events'] == [],
                'Snapshot reader cannot silently discard a lifecycle trace')
        self.metadata(tables, index)
        initial, left, right = (metadata[k] for k in ('initial','left_final','right_final'))
        for runtime in (initial,left,right): self.runtime(runtime, observe=False)
        require({r['cut']['definition'] for r in (initial,left,right)} == set(self.definitions),
                'Changed or unobserved VM definition')
        require({r['score'] for r in (initial,left,right)} == set(self.scores),
                'Unobserved score entry')
        require({r['protocol'] for r in (initial,left,right)} == self.protocols,
                'Changed or unobserved callback protocol')
        for name in ('owner','state'):
            identities = [r['cut'][name] for r in (initial,left,right)]
            self.claim(f'({ns(identities)} : List Nat).Nodup')
        for identities in ([r['score'] for r in (initial,left,right)],
                           [self.scores[r['score']][2] for r in (initial,left,right)]):
            self.claim(f'({ns(identities)} : List Nat).Nodup')
        states = sequence(packet['states'])
        require(len(states) == 3, 'Expected initial and two final VM values')
        self.claim(f'({array(natural(s) for s in states)} : List Nat) = '
                   f'{ns(r["cut"]["value"] for r in (initial,left,right))}')
        orders, returned = (sequence(packet[k]) for k in ('target_orders','returned_target_wires'))
        require(len(orders) == len(returned) == 2, 'Expected both outcome orders')
        require(all(sequence(o) for o in orders) and len(orders[0]) == len(orders[1]),
                'Expected two nonempty histories of equal length')
        for order in orders:
            for value in order: natural(value)
        self.claim(f'({ns(orders[0])} : List Nat) ≠ {ns(orders[1])}')
        prefix = self.prefix
        definition = self.definitions[initial['cut']['definition']]
        query, event = external(packet['query']), external(packet['event'])
        self.lines.extend([
            f'def {prefix}_update (state : Nat) (input : Nat × List Nat) : Option Nat := '
                f'evaluate {definition}_update {definition}_update_binding state input.1 input.2',
            f'def {prefix}_read (state : Nat) (input : Nat × List Nat) : Option Prediction := '
                f'(evaluate {definition}_read {definition}_read_binding state input.1 input.2).map '
                f'(fun reading => ⟨{ns(self.context)}, wire 8 {self.mask_mod} reading⟩)',
        ])
        seed = self.scores[initial['score']][0]
        for side, runtime, order, targets in zip(('left','right'), (left,right), orders, returned):
            require(len(sequence(targets)) == len(order), 'Missing returned outcome wire')
            encoded = []
            for outcome, target in zip(order, targets):
                target = wire(target)
                self.claim(f'({ns(target)} : List Nat) = wire 8 {self.target_mod} {outcome}')
                encoded.append(f'⟨{query}, {event}, {ns(target)}⟩')
            rows = f'{prefix}_{side}_rows'
            self.lines.append(f'def {rows} : List (Record (Nat × List Nat) (Nat × List Nat)) := '
                              + array(encoded))
            final = self.scores[runtime['score']][0]
            self.claim(f'(OutcomeTransitions.replay (OutcomeTransitions.fromPredictive '
                       f'{prefix}_read {prefix}_update) ({rows}.map OutcomeTransitions.predictiveRow) '
                       f'({initial["cut"]["value"]}, {seed})).map '
                       f'(fun s => (s.1, banksAgree s.2 {final})) = '
                       f'some ({runtime["cut"]["value"]}, true)',
                       f'rw [OutcomeTransitions.predictive_replay {prefix}_read {prefix}_update '
                       f'{rows} ⟨{initial["cut"]["value"]}, {seed}⟩]\n  decide_cbv')
            forecast = packet[side+'_forecast']
            fields(forecast, 'forecast_law prediction lengths reference residual')
            fields(forecast['prediction'], 'context mask')
            context, mask = (wire(forecast['prediction'][k]) for k in ('context','mask'))
            self.claim(f'({ns(context)} : List Nat) = {ns(self.context)}')
            self.claim(f'({prefix}_read {runtime["cut"]["value"]} {query}).map '
                       f'(fun p => p.mask) = some {ns(mask)}')
            raw = self.scores[runtime['score']][1]
            projection = {**raw, **{k:forecast[k] for k in ('forecast_law','lengths','reference','residual')}}
            self.claim(f'banksAgree {final} {self.bank(projection)} = true')
        a,b = (self.scores[r['score']][0] for r in (left,right))
        self.claim(f'banksAgree {a} {b} = true')
        self.claim(f'({left["cut"]["value"]} : Nat) = {right["cut"]["value"]}')
        # Tie the concrete program to the general result, including failures:
        # every history with the same pre-events has the same VM result.
        self.claim(f'∀ (first second : List (OutcomeTransitions.Row (Nat × List Nat) Nat)) '
                   f'(state : Nat), first.map OutcomeTransitions.Row.input = '
                   f'second.map OutcomeTransitions.Row.input → '
                   f'OutcomeTransitions.evolve (OutcomeTransitions.ignoreOutcome {prefix}_update) first state = '
                   f'OutcomeTransitions.evolve (OutcomeTransitions.ignoreOutcome {prefix}_update) second state',
                   f'intro first second state same\n  exact OutcomeTransitions.old_outcome_invariance '
                   f'{prefix}_update first second state same')
        self.counts['outcome_histories'] += 2
        self.counts['outcome_rows'] += sum(map(len, orders))

    def positional(self, packet, index):
        fields(packet, 'format kind targets rows query_targets conditioning forecast points conditionals strict_positive_witness')
        require(packet['format'] == 'synarc-conditional-expressiveness-v2', 'Unknown conditional format')
        conditioning = packet['conditioning']; fields(conditioning, 'wire_length high_six_bits')
        require(natural(conditioning['wire_length']) == '1', 'Unsupported pair-law wire length')
        high = natural(conditioning['high_six_bits'],63)
        rows, encoded, parsed = sequence(packet['rows']), [], []
        require(rows, 'Missing explicit training rows')
        for row in rows:
            fields(row, 'context mask target')
            c,m,t = (wire(row[k]) for k in ('context','mask','target'))
            parsed.append((c,t)); encoded.append(f'⟨{ns(c)}, {ns(m)}, {ns(t)}⟩')
        self.claim(f'({array(ns(t) for _,t in parsed)} : List (List Nat)) = '
                   f'{array("["+natural(t,255)+"]" for t in sequence(packet["targets"]))}')
        history = tuple(tuple(tuple(r[k]) for k in ('context','mask','target')) for r in rows)
        if history not in self.histories:
            name = 'training_'+str(len(self.histories))
            self.histories[history] = name
            for block in trace(rows, encoded):
                if block.startswith('attribute '): continue
                block = block.replace(': State :=', ': ResidualPrediction.State :=')
                block = block.replace('row (', 'ResidualPrediction.row (')
                self.lines.append(re.sub(r'\b(state|transition)_(\d+)\b',
                    lambda m: f'{name}_{m[1]}_{m[2]}', block))
            self.trace_claims += len(rows)
            self.lines.append(f'def {name}_rows : List ResidualPrediction.Row := {array(encoded)}')
            self.claim(f'ResidualPrediction.fold {name}_rows ResidualPrediction.initial = '
                       f'{name}_state_{len(rows)}')
        name = self.histories[history]
        state = f'{name}_state_{len(rows)}'
        forecast = packet['forecast']
        fields(forecast, 'forecast_law prediction lengths reference residual')
        require(forecast['forecast_law'] == FORECAST_LAW, 'Wrong forecast law')
        fields(forecast['prediction'], 'context mask')
        context, mask = (wire(forecast['prediction'][k]) for k in ('context','mask'))
        lengths = encode_lengths(forecast['lengths'],len(rows),[t for _,t in parsed])
        residual, reference = encode_banks(forecast, parsed)
        self.claim(f'AdaptiveResidual.histogramAgrees {name}_rows {lengths} = true')
        self.claim(f'AdaptiveResidual.forecastAgrees {state} {residual} {reference} = true')
        targets = [wire(t) for t in sequence(packet['query_targets'])]
        points, conditional, witness = (fractions(packet[k]) for k in
                                        ('points','conditionals','strict_positive_witness'))
        require(len(targets) == len(points) == len(conditional) == len(witness) == 4,
                'Expected four pair-law entries')
        require(len(set(targets)) == 4, 'Duplicate conditioning outcome')
        self.claim(f'({array(ns(t) for t in targets)} : List (List Nat)) = '
                   f'(List.range 4).map (fun low => [4*{high}+low])')
        mass = f'(ExactMass.sum {array(points)})'
        self.claim(f'0 < {mass}.numerator')
        for target, point, cond in zip(targets,points,conditional):
            self.claim(f'ExactMass.Equal (AdaptiveResidual.candidate {lengths} {state} '
                       f'{ns(context)} {ns(mask)} {ns(target)} false) {point}')
            self.claim(f'ExactMass.Equal (ExactMass.mul {cond} {mass}) {point}')
        for label, law in (('conditional',conditional),('witness',witness)):
            self.lines.append(f'def p{index}_{label} (x y : Bool) : ExactMass.Fraction := '
                              f'if x then (if y then {law[3]} else {law[1]}) '
                              f'else (if y then {law[2]} else {law[0]})')
            self.claim(f'ExactMass.Equal (ExactMass.sum {array(law)}) ExactMass.one')
        self.claim(f'ConditionalExpressiveness.RankOne p{index}_conditional')
        self.claim(f'¬ ConditionalExpressiveness.RankOne p{index}_witness')
        self.claim(f'∀ x y, 0 < (p{index}_witness x y).numerator',
                   'intro x y\n  cases x <;> cases y <;> decide_cbv')
        self.counts['probability_queries'] += len(targets)
        self.counts['training_rows'] += len(rows)


def emit(data):
    fields(data, 'packets'); packets = sequence(data['packets'])
    require(packets, 'Empty expressiveness export')
    replay = WitnessReplay()
    for index, packet in enumerate(packets):
        require(type(packet) is dict, 'Expected witness packet')
        if packet.get('kind') == 'pre_event_update_order_blindness': replay.outcome_order(packet,index)
        elif packet.get('kind') == 'positional_rank_one': replay.positional(packet,index)
        else: raise ValueError('Unknown expressiveness witness')
    replay.lines.append('end FrozenObserverExport')
    return '\n\n'.join(replay.lines)+'\n', {**replay.counts, 'packets':len(packets),
        'claims':replay.claims+replay.trace_claims, 'training_transitions':replay.trace_claims}


if __name__ == '__main__':
    replay_main(emit=emit, modules=MODULES, producers=[Path(__file__).resolve(),
        *[Path(__file__).with_name(name+'.py') for name in ('check_predictive_transactions',
        'check_residual_export','check_residual_queries','check_joint_export',
        'predictive_cursor_proofs','residual_trace_proofs')]],
        scope=__doc__, description=__doc__, export_loader=load_jsonl)
