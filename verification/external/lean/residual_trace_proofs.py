"""Untrusted row witnesses: every transition is proved, then reused by replay.

These are evaluator hints, never native evidence. The original supplied native
frontiers remain separate and must agree with the complete checked row chain.
"""
from observer_export_replay import array


def numeral(value):
    # Avoid process-wide Python decimal limits for unreduced exact products.
    if value < 10**1000:
        return str(value)
    high, low = divmod(value, 10**1000)
    return numeral(high) + str(low).zfill(1000)


def trace(rows, encoded):
    residual, reference = {}, {}
    numerator, denominator, bits = 1, 1, 0
    blocks = []
    before = '⟨0, 0, ⟨1, 1, by decide⟩, [], []⟩'
    for i, (r, code) in enumerate(zip(rows, encoded)):
        for position in range(8 * len(r['target'])):
            byte, offset = divmod(position, 8)
            target = (r['target'][byte] >> offset) & 1
            mask = (r['mask'][byte] >> offset) & 1 if byte < len(r['mask']) else 0
            bit = target ^ mask
            child = residual.setdefault((tuple(r['context']), position), [0, 0])
            raw = reference.setdefault(position, [0, 0])
            numerator *= (2*child[bit]+1) * (2*sum(raw)+2)
            denominator *= (2*sum(child)+2) * (2*raw[target]+1)
            child[bit] += 1
            raw[target] += 1
        bits += 8 * len(r['target'])
        child = array(f'(({array(map(str, c))}, {p}), ⟨⟨[{a}, {b}], by decide⟩, rfl⟩)'
                      for (c, p), (a, b) in residual.items())
        raw = array(f'({p}, ⟨⟨[{a}, {b}], by decide⟩, rfl⟩)'
                    for p, (a, b) in reference.items())
        after = (f'⟨{i+1}, {bits}, ⟨{numeral(numerator)}, {numeral(denominator)}, by decide⟩, '
                 f'{child}, {raw}⟩')
        blocks.append(f'def state_{i+1} : State := {after}')
        # CBV rewrites after normalizing arguments: index the rule by concrete
        # constructors, not by transparent aliases that have already unfolded.
        blocks.append(f'@[cbv_eval]\ntheorem transition_{i} : '
                      f'row ({before}) ({code}) = state_{i+1} := by decide_cbv')
        before = after
    # If a hint does not match, replay fails instead of silently redoing the
    # entire computation. All hints above have already been kernel checked.
    blocks.append('attribute [cbv_opaque] ResidualPrediction.row')
    return blocks
