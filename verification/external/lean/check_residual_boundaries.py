#!/usr/bin/env python3
"""Replay captured partial-row banks and observed publication/refusal results.

This format covers one empty row followed by one nonempty row. It is finite
native correspondence, not a proof of arbitrary native lifecycle executions.
"""
from pathlib import Path

from check_residual_export import FORECAST_LAW, encode_banks, encode_lengths, ns, pair, wire
from check_residual_queries import MODULES, emit as emit_queries, fraction
from observer_export_replay import array, boolean, fields, natural, require, sequence, main as replay_main


def emit(data):
    fields(data, 'format queries partial committed')
    require(data['format'] == 'synarc-residual-boundaries-v2', 'Unsupported boundary export')
    packets = sequence(data['queries'])
    require(len(packets) == 3, 'Expected initial, empty-row and complete-row forecasts')
    source, counts = emit_queries({'packets': packets})
    rows = packets[2]['rows']
    require(len(rows) == 2 and packets[0]['rows'] == [] and packets[1]['rows'] == rows[:1],
            'Boundary histories do not share the declared completed prefix')
    require(rows[0]['target'] == [] and rows[1]['target'], 'Expected empty then nonempty row')
    context, mask, target = (wire(rows[1][key]) for key in ('context', 'mask', 'target'))
    width = 8 * len(target)
    partial = data['partial']
    fields(partial, 'before_refusal after_refusal after_last_bit positions length_total length_totals '
           'partial_finish_refused last_bit_finish_refused zero_work_refused '
           'empty_finish_refused empty_zero_work_refused')
    positions, totals = (sequence(partial[key]) for key in ('positions', 'length_totals'))
    require(len(positions) == len(totals) == 3, 'Incomplete partial boundary capture')
    positions = [natural(p) for p in positions]
    totals = [natural(t) for t in totals]
    claims = [f'theorem boundary_positions : ({array(positions)} : List Nat) = '
              f'[{width-1}, {width-1}, {width}] := by decide_cbv',
              f'theorem boundary_totals : ({array(totals)} : List Nat) = [1,1,1] := by decide_cbv',
              f'theorem boundary_length_total : ({natural(partial["length_total"])} : Nat) = 1 := by decide_cbv']
    for key in ('partial_finish_refused', 'last_bit_finish_refused', 'zero_work_refused',
                'empty_finish_refused', 'empty_zero_work_refused'):
        claims.append(f'theorem boundary_{key} : {boolean(partial[key])} = true := by decide_cbv')
    declarations = ['def boundary_base : State := fold packet_1_rows initial']
    for index, key in enumerate(('before_refusal', 'after_refusal', 'after_last_bit')):
        snapshot = partial[key]
        fields(snapshot, 'forecast_law prediction lengths reference residual')
        require(snapshot['forecast_law'] == FORECAST_LAW, 'Wrong partial forecast law')
        fields(snapshot['prediction'], 'context mask')
        require(wire(snapshot['prediction']['context']) == context and
                wire(snapshot['prediction']['mask']) == mask, 'Partial prediction changed')
        lengths = encode_lengths(snapshot['lengths'], 1, [[]])
        # Unlike complete rows, a private body can retain a non-byte-aligned
        # number of allocated cells. Preserve exactly those captured cells.
        residual = sequence(snapshot['residual'])
        require(len(residual) == 1, 'Expected the pending row context only')
        fields(residual[0], 'context counts')
        require(wire(residual[0]['context']) == context, 'Wrong pending context')
        child, raw = sequence(residual[0]['counts']), sequence(snapshot['reference'])
        require(len(child) == len(raw) == int(positions[index]), 'Partial position/cell mismatch')
        child = array(f'(({ns(context)}, {p}), {array(pair(c))})' for p,c in enumerate(child))
        raw = array(f'({p}, {array(pair(c))})' for p,c in enumerate(raw))
        state = f'boundary_partial_{index}'
        declarations.append(f'def {state} : State := body boundary_base {ns(context)} 0 '
            f'(wireBits {ns(mask)}) ((wireBits {ns(target)}).take {positions[index]})')
        claims.extend([
            f'theorem boundary_bank_{index} : AdaptiveResidual.forecastAgrees {state} '
            f'{child} {raw} = true := by decide_cbv',
            f'theorem boundary_lengths_{index} : AdaptiveResidual.histogramAgrees '
            f'packet_1_rows {lengths} = true := by decide_cbv',
        ])
    committed = data['committed']
    fields(committed, 'bits forecast_law lengths ratio reference residual samples')
    require(committed['forecast_law'] == FORECAST_LAW, 'Wrong committed law')
    lengths = encode_lengths(committed['lengths'], 2, [r['target'] for r in rows])
    residual, reference = encode_banks(committed, [(wire(r['context']),wire(r['target'])) for r in rows])
    snapshot = (f'⟨{natural(committed["samples"])}, {natural(committed["bits"])}, '
                f'{fraction(committed["ratio"])}, {residual}, {reference}⟩')
    claims.extend([
        f'theorem boundary_commit : agrees (fold packet_2_rows initial) {snapshot} = true := by decide_cbv',
        f'theorem boundary_commit_lengths : AdaptiveResidual.histogramAgrees packet_2_rows '
        f'{lengths} = true := by decide_cbv',
    ])
    ending = 'end FrozenObserverExport\n'
    require(source.endswith(ending), 'Missing query proof namespace')
    source = source[:-len(ending)] + '\n\n'.join(declarations + claims) + '\n\n' + ending
    return source, {**counts, 'claims': counts['claims'] + len(claims), 'boundary_claims': len(claims)}


if __name__ == '__main__':
    replay_main(emit=emit, modules=MODULES,
        producers=[Path(__file__).resolve(), Path(__file__).with_name('check_residual_queries.py'),
                   Path(__file__).with_name('check_residual_export.py'),
                   Path(__file__).with_name('check_joint_export.py'),
                   Path(__file__).with_name('residual_trace_proofs.py')],
        scope='Captured empty/pending/last-bit/row-end banks and exact frozen probabilities; '
              'observed refusal/publication flags checked against the required outcomes. '
              'Finite source-bound evidence, not universal native ownership or overflow correctness.',
        description=__doc__)
