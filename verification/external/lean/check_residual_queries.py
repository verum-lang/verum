#!/usr/bin/env python3
"""Check frozen JSONL residual forecasts and exact point/prefix probabilities.

Each packet supplies its complete row history. Phase labels are descriptive;
these packets do not establish native execution or lifecycle ownership.
"""
import json
from pathlib import Path
import re

from check_joint_export import decimal
from check_residual_export import FORECAST_LAW, encode_banks, encode_lengths, ns, wire
from observer_export_replay import (
    array, boolean, fields, require, sequence, unique_object, main as replay_main,
)
from residual_trace_proofs import trace


FORMAT = 'synarc-residual-queries-v2'
MODULES = ['ExactMass', 'CausalCoding', 'EnumerableMass', 'FiniteWire',
           'AdaptiveLength', 'CategoricalPrediction', 'BinaryPrediction',
           'PositionPrediction', 'ResidualPrediction', 'AdaptiveResidual']


def load_jsonl(path):
    """Decode every line; never discard blank, malformed or duplicate-key rows."""
    require(path.stat().st_size <= 16 * 1024 * 1024, 'Export exceeds 16 MiB limit')
    lines = path.read_text().splitlines()
    require(lines, 'Empty probability-query export')
    packets = []
    def reject_constant(value):
        raise ValueError(f'Non-JSON constant: {value}')
    for number, line in enumerate(lines, 1):
        require(line.strip(), f'Blank JSONL packet at line {number}')
        packets.append(json.loads(line, object_pairs_hook=unique_object,
                                  parse_constant=reject_constant))
    return {'packets': packets}


def fraction(value):
    fields(value, 'numerator denominator')
    numerator, denominator = (decimal(value[k]) for k in ('numerator', 'denominator'))
    require(denominator != '0', 'Zero probability denominator')
    return f'⟨{numerator}, {denominator}, by decide⟩'


def emit(data):
    fields(data, 'packets')
    packets = sequence(data['packets'])
    require(packets, 'Empty probability-query export')
    blocks = [
        'import VerumExternalReplay.AdaptiveResidual',
        'open ExactMass ResidualPrediction',
        'set_option warningAsError true',
        'set_option maxRecDepth 65536',
        'set_option maxHeartbeats 30000000',
        'set_option cbv.maxSteps 10000000',
        'namespace FrozenObserverExport',
    ]
    # All row hints are checked before making row opaque. Packet histories may
    # overlap, but each supplied packet gets its own complete checked fold.
    declarations, claims = [], []
    row_count = query_count = 0
    phases = []
    for index, packet in enumerate(packets):
        fields(packet, 'format phase rows forecast queries')
        require(packet['format'] == FORMAT, 'Unsupported probability-query export')
        require(type(packet['phase']) is str, 'Expected descriptive phase string')
        phases.append(packet['phase'])
        prefix = f'packet_{index}'
        rows = sequence(packet['rows'])
        encoded_rows, parsed = [], []
        for item in rows:
            fields(item, 'context mask target')
            context, mask, target = (wire(item[k]) for k in ('context', 'mask', 'target'))
            parsed.append((context, target))
            encoded_rows.append(f'⟨{ns(context)}, {ns(mask)}, {ns(target)}⟩')
        for block in trace(rows, encoded_rows):
            if block.startswith('attribute '):
                continue
            blocks.append(re.sub(r'\b(state|transition)_(\d+)\b',
                                 lambda m: f'{prefix}_{m[1]}_{m[2]}', block))
        state = f'{prefix}_state_{len(rows)}' if rows else 'initial'
        forecast = packet['forecast']
        fields(forecast, 'forecast_law prediction lengths reference residual')
        require(forecast['forecast_law'] == FORECAST_LAW, 'Wrong forecast probability law')
        fields(forecast['prediction'], 'context mask')
        context, mask = (wire(forecast['prediction'][k]) for k in ('context', 'mask'))
        counts = encode_lengths(forecast['lengths'], len(rows), [target for _, target in parsed])
        residual, reference = encode_banks(forecast, parsed)
        declarations.extend([
            f'def {prefix}_rows : List Row := {array(encoded_rows)}',
            f'def {prefix}_lengths : AdaptiveLength.Counts := {counts}',
        ])
        claims.extend([
            f'theorem {prefix}_complete : fold {prefix}_rows initial = {state} := by decide_cbv',
            f'theorem {prefix}_histogram : AdaptiveResidual.histogramAgrees '
            f'{prefix}_rows {prefix}_lengths = true := by decide_cbv',
            f'theorem {prefix}_banks : AdaptiveResidual.forecastAgrees '
            f'{state} {residual} {reference} = true := by decide_cbv',
        ])
        queries = sequence(packet['queries'])
        require(queries, 'Packet has no probability queries')
        for qindex, query in enumerate(queries):
            fields(query, 'prefix target candidate reference')
            is_prefix = boolean(query['prefix'])
            target = wire(query['target'])
            arguments = f'{prefix}_lengths {state}'
            claims.extend([
                f'theorem {prefix}_query_{qindex}_candidate : Equal '
                f'(AdaptiveResidual.candidate {arguments} {ns(context)} {ns(mask)} '
                f'{ns(target)} {is_prefix}) {fraction(query["candidate"])} := by decide_cbv',
                f'theorem {prefix}_query_{qindex}_reference : Equal '
                f'(AdaptiveResidual.reference {arguments} {ns(target)} {is_prefix}) '
                f'{fraction(query["reference"])} := by decide_cbv',
            ])
        row_count += len(rows)
        query_count += len(queries)
    blocks.extend(['attribute [cbv_opaque] ResidualPrediction.row', *declarations,
                   *claims, 'end FrozenObserverExport'])
    return '\n\n'.join(blocks) + '\n', {
        'packets': len(packets), 'rows': row_count, 'queries': query_count,
        'claims': row_count + len(claims), 'phases': phases,
    }


if __name__ == '__main__':
    replay_main(emit=emit, export_loader=load_jsonl, modules=MODULES,
        producers=[Path(__file__).resolve(), Path(__file__).with_name('check_residual_export.py'),
                   Path(__file__).with_name('check_joint_export.py'),
                   Path(__file__).with_name('residual_trace_proofs.py')],
        scope='Finite complete-row KT and shared adaptive byte-length forecast replay, with exact '
              'point and all-wire prefix probabilities. Phase labels do not prove native lifecycle '
              'ownership. Native VM, physical causality, overflow/refusal, resource charging and '
              'statistical stopping guarantees remain separate.',
        description=__doc__)
