#!/usr/bin/env python3
"""Check frozen numeral code and full native VM observations against Lean."""
from pathlib import Path
import acquired_export_codec
from acquired_export_codec import AcquiredDecoder
from observer_export_replay import (MAX64, array, fields, natural, require,
                                   sequence, variant, module_closure, main as replay_main)

DECODER = AcquiredDecoder(primitive_extensions=('Succ',))
MODELS = module_closure(['CheckedNumerals', 'FiniteExecution'])
PRODUCERS = [Path(__file__).resolve(), Path(acquired_export_codec.__file__).resolve()]
SCOPE = ('Finite supplied code and VM reports, including registers, stop, counters and typed halt. '
         'Original and decoded ASTs are compared; portable wire decoding, bindings, syntax cell '
         'accounting, enumeration, allocation, external work meters and acquisition are not proved.')


def report(item):
    """The numeral export uses serde's externally tagged primitive failure."""
    normalized = dict(item)
    if item.get('halt') is not None:
        halt = item['halt']; fields(halt, 'path reason')
        kind, error = variant(halt['reason'])
        require(kind == 'Primitive', 'Unsupported numeral halt representation')
        normalized['halt'] = dict(path=halt['path'], reason=dict(kind=kind, error=error))
    return DECODER.report(normalized)


def emit(data):
    fields(data, 'format scope payload')
    require(data['format'] == 'synarc-native-numeral-court-v1', 'Unknown numeral schema')
    require(type(data['scope']) is str, 'Expected scope text')
    payload = data['payload']; require(type(payload) is dict, 'Expected numeral payload')
    lines = ['import VerumExternalReplay.CheckedNumerals',
             'import VerumExternalReplay.FiniteExecution', 'open AcquiredObserver',
             'set_option warningAsError true', 'namespace FrozenObserverExport']
    names = []; executions = 0

    def claim(proposition, proof='decide_cbv'):
        name = 'c' + str(len(names)); names.append(name)
        lines.append(f'theorem {name} : {proposition} := by\n  {proof}')

    def execution(row, original=None, max_trace=None):
        nonlocal executions
        fields(row, 'code inputs semantics limit report')
        fields(row['semantics'], 'kind max_trace')
        require(row['semantics']['kind'] == 'AcquiredV2', 'Unsupported numeral semantics')
        cap = natural(row['semantics']['max_trace'])
        if max_trace is not None:
            require(row['semantics']['max_trace'] == max_trace, 'Trace bound changed during decoding')
        require(row['limit'] == 'Unlimited', 'Unsupported numeral execution limit')
        p = 'code' + str(executions); executions += 1
        lines.append(f'def {p} : FiniteCode.Code Op Value := {DECODER.code(row["code"])}')
        if original is not None:
            claim(f'{p} = {DECODER.code(original)}', 'rfl')
        inputs = array(DECODER.value(v) for v in sequence(row['inputs']))
        # Compare only exposed observations. MAX64 cannot charge-refuse these
        # finite executions; no native work-meter observation is invented.
        claim(f'(FiniteExecution.execute (apply {cap}) {p} ⟨none, {MAX64}⟩ '
              f'{inputs}).observation = {report(row["report"])}')

    if 'closed' in payload:
        fields(payload, 'closed boundary')
        closed = sequence(payload['closed']); boundary = sequence(payload['boundary'])
        require(closed and boundary, 'Missing closed or boundary executions')
        for row in closed + boundary:
            execution(row)
    else:
        fields(payload, 'bindings cells decoded_parts max_trace original_parts widths wire')
        originals = sequence(payload['original_parts']); decoded = sequence(payload['decoded_parts'])
        require(len(originals) == len(decoded) == 3, 'Expected three actual portable program roots')
        natural(payload['max_trace']); natural(payload['cells'])
        require(len(sequence(payload['widths'])) == len(sequence(payload['bindings'])) == 2,
                'Expected two portable binding descriptions')
        for width in payload['widths']: natural(width)
        for slots in payload['bindings']: sequence(slots)
        wire = sequence(payload['wire']); require(wire, 'Missing retained wire bytes')
        for byte in wire: natural(byte, 255)
        for original, decoded in zip(originals, decoded):
            execution(decoded, original, payload['max_trace'])

    lines += ['#print axioms ' + name for name in names] + ['end FrozenObserverExport']
    return '\n\n'.join(lines) + '\n', dict(executions=executions, claims=len(names))


if __name__ == '__main__':
    replay_main(emit=emit, modules=MODELS, producers=PRODUCERS, scope=SCOPE, description=__doc__)
