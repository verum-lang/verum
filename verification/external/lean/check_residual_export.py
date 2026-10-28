#!/usr/bin/env python3
"""Replay a complete native residual/raw KT stream and every supplied frontier."""
from pathlib import Path
from check_joint_export import decimal
from observer_export_replay import fields, natural, sequence, array, require, main as replay_main
from residual_trace_proofs import trace


def wire(value):
    return tuple(int(natural(x, 255)) for x in sequence(value))


def ns(values):
    return array(str(x) for x in values)


def pair(value):
    require(len(sequence(value)) == 2, 'Expected both binary counts')
    return [natural(x) for x in value]


def emit(data):
    fields(data, 'format bit_order length_law length_unit rows frontiers')
    require(data['format'] == 'synarc-residual-native-v1', 'Unsupported residual export')
    require(data['bit_order'] == 'lsb0' and data['length_unit'] == 'byte' and
            data['length_law'] == '2^-(n+1)', 'Wrong wire probability convention')
    rows = sequence(data['rows'])
    require(rows, 'Empty native stream')
    phases = {'training': 0, 'fresh': 1, 'runtime': 2}
    encoded_rows, parsed, ids, seen_phases = [], [], set(), []
    for item in rows:
        fields(item, 'context mask target native_read native_state ordinal origin phase')
        require(type(item['phase']) is str and item['phase'] in phases, 'Unknown phase')
        phase = phases[item['phase']]
        require(not seen_phases or phase >= seen_phases[-1], 'Phase order regressed')
        seen_phases.append(phase)
        identity = (natural(item['origin']), natural(item['ordinal']))
        require(identity not in ids, 'Duplicate physical observation identity')
        ids.add(identity)
        # These values bind the original capture; VM evaluation is a separate theorem.
        natural(item['native_read']); natural(item['native_state'])
        context, mask, target = (wire(item[k]) for k in ('context', 'mask', 'target'))
        parsed.append((context, target))
        encoded_rows.append(f'⟨{ns(context)}, {ns(mask)}, {ns(target)}⟩')
    require(set(seen_phases) == {0, 1, 2}, 'Training, fresh and runtime observations required')
    frontiers = sequence(data['frontiers'])
    encoded_frontiers, cuts = [], []
    for frontier in frontiers:
        fields(frontier, 'samples bits ratio reference residual')
        cut = int(natural(frontier['samples']))
        require(cut <= len(rows) and (not cuts or cut >= cuts[-1]), 'Invalid frontier prefix')
        cuts.append(cut)
        bits = natural(frontier['bits'])
        fields(frontier['ratio'], 'numerator denominator')
        numerator, denominator = (decimal(frontier['ratio'][k]) for k in ('numerator', 'denominator'))
        require(denominator != '0', 'Zero ratio denominator')
        # Reorder only storage: every supplied key/count pair must occur exactly once.
        expected, used = [], set()
        for context, target in parsed[:cut]:
            for position in range(8 * len(target)):
                key = (context, position)
                if key not in used:
                    used.add(key); expected.append(key)
        provided, contexts = {}, []
        for entry in sequence(frontier['residual']):
            fields(entry, 'context counts')
            context = wire(entry['context'])
            require(context not in contexts, 'Duplicate residual context')
            contexts.append(context)
            counts = sequence(entry['counts'])
            require(counts, 'Empty allocated residual context')
            for position, counts_pair in enumerate(counts):
                provided[(context, position)] = pair(counts_pair)
        require(contexts == sorted(contexts), 'Noncanonical native context order')
        require(set(provided) == used, 'Missing or extra residual cells')
        residual = array(f'(({ns(c)}, {p}), {array(provided[(c,p)])})' for c,p in expected)
        reference = sequence(frontier['reference'])
        require(len(reference) == max((8*len(target) for _,target in parsed[:cut]), default=0),
                'Missing or extra raw-reference cells')
        reference = array(f'({p}, {array(pair(counts))})' for p,counts in enumerate(reference))
        encoded_frontiers.append(f'⟨{cut}, {bits}, ⟨{numerator}, {denominator}, by decide⟩, {residual}, {reference}⟩')
    boundaries = {seen_phases.count(0), seen_phases.count(0)+seen_phases.count(1), len(rows)}
    require(boundaries <= set(cuts) and cuts[-1] == len(rows), 'Missing phase frontier or incomplete stream')
    blocks = [
        'import VerumExternalReplay.ResidualPrediction',
        'open ResidualPrediction',
        'set_option warningAsError true',
        'set_option maxRecDepth 65536',
        'set_option maxHeartbeats 30000000',
        'set_option cbv.maxSteps 10000000',
        'namespace FrozenObserverExport',
        *trace(rows, encoded_rows),
        'def native_rows : List Row := ' + array(encoded_rows),
        'def native_frontiers : List Snapshot := ' + array(encoded_frontiers),
        'theorem complete_stream : replay native_rows initial native_frontiers = true := by decide_cbv',
        'end FrozenObserverExport',
    ]
    return '\n\n'.join(blocks) + '\n', {'rows': len(rows), 'frontiers': len(cuts), 'claims': len(rows)+1,
        'training_rows': seen_phases.count(0), 'fresh_rows': seen_phases.count(1), 'runtime_rows': seen_phases.count(2)}


if __name__ == '__main__':
    replay_main(emit=emit,
        modules=['ExactMass','CausalCoding','CategoricalPrediction','BinaryPrediction',
                 'PositionPrediction','ResidualPrediction'],
        producers=[Path(__file__).resolve(), Path(__file__).with_name('check_joint_export.py'),
                   Path(__file__).with_name('residual_trace_proofs.py')],
        scope='Complete finite residual/raw KT row fold and all count/ratio frontiers. Native VM, physical causality, overflow/refusal behavior, resource charging and statistical null/stopping guarantees are separate.',
        description=__doc__)
