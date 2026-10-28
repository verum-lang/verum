"""Untrusted cursor witnesses, each checked before reuse by native replay.

No snapshot is accepted because Python calculated it. The emitted induction
proves every admitted step in Lean and audits the resulting shared lemmas.
"""
from observer_export_replay import array
from residual_trace_proofs import numeral


def cursor_trace(prefix, base, context, mask, target):
    residual = {(tuple(row['context']),i):list(counts)
        for row in base['residual'] for i,counts in enumerate(row['counts'])}
    reference = {i:list(counts) for i,counts in enumerate(base['reference'])}
    lengths = sorted((int(n),c) for n,c in base['lengths']['counts'].items())
    numerator,denominator = (int(base['ratio'][k]) for k in ('numerator','denominator'))
    samples,bits = base['samples'],base['bits']
    target_bits = [bool((byte>>bit)&1) for byte in target for bit in range(8)]
    mask_bits = [bool((byte>>bit)&1) for byte in mask for bit in range(8)]

    def ns(values): return array(str(v) for v in values)
    def bs(values): return array(str(v).lower() for v in values)
    def bank():
        child = array(f'(({ns(c)}, {i}), BinaryPrediction.fromCounts {a} {b})'
            for (c,i),(a,b) in residual.items())
        raw = array(f'({i}, BinaryPrediction.fromCounts {a} {b})'
            for i,(a,b) in reference.items())
        return (f'⟨⟨{samples}, {bits}, ⟨{numeral(numerator)}, {numeral(denominator)}, by decide⟩, '
                f'{child}, {raw}⟩, {array(f"({n}, {c})" for n,c in lengths)}⟩')
    def cursor(position):
        if position > len(target_bits): return f'ResidualContinuation.Cursor.ready {bank()}'
        return (f'ResidualContinuation.Cursor.pending ⟨{bank()}, {ns(context)}, '
                f'{bs(mask_bits[position:])}, {position}, {len(target)}⟩ {bs(target_bits[position:])}')

    start = cursor(0)
    blocks = [f'def {prefix}_0 : ResidualContinuation.Cursor := {start}',
        f'@[cbv_eval]\ntheorem {prefix}_step_0 : ResidualContinuation.run 0 {prefix}_0 = {prefix}_0 := rfl']
    for position in range(len(target_bits)+1):
        if position == len(target_bits):
            samples += 1; bits += len(target_bits); lengths.insert(0,(len(target),1))
        else:
            bit = int(target_bits[position])
            predicted = int(mask_bits[position]) if position < len(mask_bits) else 0
            child = residual.setdefault((tuple(context),position),[0,0])
            raw = reference.setdefault(position,[0,0])
            numerator *= (2*child[bit^predicted]+1)*(2*sum(raw)+2)
            denominator *= (2*sum(child)+2)*(2*raw[bit]+1)
            child[bit^predicted] += 1; raw[bit] += 1
        n = position+1
        blocks.append(f'def {prefix}_{n} : ResidualContinuation.Cursor := {cursor(n)}')
        blocks.append(f'@[cbv_eval]\ntheorem {prefix}_step_{n} : '
            f'ResidualContinuation.run {n} {prefix}_0 = {prefix}_{n} := by\n'
            f'  calc\n'
            f'    _ = ResidualContinuation.run 1 (ResidualContinuation.run {position} {prefix}_0) :=\n'
            f'      ResidualContinuation.run_add {position} 1 _\n'
            f'    _ = ResidualContinuation.run 1 {prefix}_{position} :=\n'
            f'      congrArg (ResidualContinuation.run 1) {prefix}_step_{position}\n'
            f'    _ = {prefix}_{n} := by decide_cbv')
    return blocks,len(target_bits)+2
