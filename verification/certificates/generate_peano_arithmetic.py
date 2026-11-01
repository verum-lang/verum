"""Constructive arithmetic laws and corrupted universal/concrete controls."""
from peano_arithmetic import (library, N, Z, ONE, S, a, b, c, add, mul, eq, le,
    ring, call)
from portable_terms import bind, lam, pi, var, lower, refl, tt


def cases():
    results = library.cases()
    identities = [
        ('polynomial_distributivity', mul(add(a, b), add(b, c)),
         add(add(mul(a, b), mul(a, c)), add(mul(b, b), mul(b, c)))),
        ('kt_factor_cross_difference',
         add(mul(a, add(add(a, b), c)), mul(c, b)),
         mul(add(a, c), add(a, b))),
    ]
    params = [('a', N), ('b', N), ('c', N)]
    for name, left, right in identities:
        cert = {'term': lower(library.close(bind(lam, params, ring(left, right).proof))),
                'claimed_type': lower(library.close(bind(pi, params, eq(left, right)))), 'metadata': {}}
        results.append((name, cert, True))
    invalid = [
        ('zero_cancellation', call('le_refl', Z), le(ONE, Z)),
        ('strict_order_reversed', tt(), le(S(ONE), ONE)),
        ('false_successor_identity', refl(ONE), eq(ONE, Z)),
        ('missing_positive_multiplier', bind(lam, [('a', N), ('b', N)], call('le_refl', Z)),
         bind(pi, [('a', N), ('b', N)], le(a, b))),
        ('wrong_distributivity', bind(lam, params, ring(identities[0][1], identities[0][2]).proof),
         bind(pi, params, eq(identities[0][1], add(mul(a, b), mul(b, c))))),
    ]
    for name, term, claim in invalid:
        results.append((name, {'term': lower(library.close(term)),
                              'claimed_type': lower(library.close(claim)), 'metadata': {}}, False))
    return results
