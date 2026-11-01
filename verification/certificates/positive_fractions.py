"""Unreduced natural fractions with explicit denominator-positivity proofs.

Order is cross multiplication. No division, float, quotient axiom or trusted
rational primitive is added to the portable calculus.
"""
import peano_arithmetic as nat
from peano_arithmetic import (Library, N, Z, ONE, S, Path, add, mul, le, eq,
    call, change_order, ring, law, induction)
from portable_terms import (app, bind, first, second, pair, sigma, var, lam,
    pi, tt, j, refl)

library = Library()
library.definitions = nat.library.definitions.copy()
library.claims = nat.library.claims.copy()
FOUNDATIONS = set(library.claims)
Q = library.define('Fraction', sigma('_numerator', N, N))
num = lambda x: x[1] if x[0] == 'Pair' else first(x)
den = lambda x: x[2] if x[0] == 'Pair' else second(x)
fraction = pair
one = pair(ONE, ONE)
times = lambda x, y: pair(mul(num(x), num(y)), mul(den(x), den(y)))
positive = lambda x: le(ONE, den(x))
order = lambda x, y: le(mul(num(x), den(y)), mul(num(y), den(x)))
equal = lambda x, y: eq(mul(num(x), den(y)), mul(num(y), den(x)))


def reframe(left, right, proof, new_left, new_right):
    return change_order(left, right, proof, ring(left, new_left), ring(right, new_right))


def equal_proof(left, right):
    return ring(mul(num(left), den(right)), mul(num(right), den(left))).proof


a, b, c, d = map(var, ('a', 'b', 'c', 'd'))
library.prove('fraction_refl', [('a', Q)], call('le_refl', mul(num(a), den(a))), order(a, a))
library.prove('fraction_equal_le', [('a', Q), ('b', Q), ('h', equal(a, b))],
    j(lam('_rhs', N, le(mul(num(a), den(b)), var('_rhs'))),
      call('le_refl', mul(num(a), den(b))), var('h')), order(a, b))
library.prove('fraction_trans', [('a', Q), ('b', Q), ('c', Q), ('positive', positive(b)),
    ('ab', order(a, b)), ('bc', order(b, c))],
    call('mul_positive_le_cancel', den(b), mul(num(a), den(c)), mul(num(c), den(a)), var('positive'),
        call('le_trans', mul(mul(num(a), den(c)), den(b)), mul(mul(num(b), den(a)), den(c)),
             mul(mul(num(c), den(a)), den(b)),
             reframe(mul(mul(num(a), den(b)), den(c)), mul(mul(num(b), den(a)), den(c)),
                call('mul_le_right', den(c), mul(num(a), den(b)), mul(num(b), den(a)), var('ab')),
                mul(mul(num(a), den(c)), den(b)), mul(mul(num(b), den(a)), den(c))),
             reframe(mul(mul(num(b), den(c)), den(a)), mul(mul(num(c), den(b)), den(a)),
                call('mul_le_right', den(a), mul(num(b), den(c)), mul(num(c), den(b)), var('bc')),
                mul(mul(num(b), den(a)), den(c)), mul(mul(num(c), den(a)), den(b))))), order(a, c))
library.prove('fraction_mul_le', [('a', Q), ('b', Q), ('c', Q), ('d', Q),
    ('ab', order(a, b)), ('cd', order(c, d))],
    reframe(mul(mul(num(a), den(b)), mul(num(c), den(d))),
            mul(mul(num(b), den(a)), mul(num(d), den(c))),
        call('mul_le', mul(num(a), den(b)), mul(num(b), den(a)),
             mul(num(c), den(d)), mul(num(d), den(c)), var('ab'), var('cd')),
        mul(num(times(a, c)), den(times(b, d))), mul(num(times(b, d)), den(times(a, c)))),
    order(times(a, c), times(b, d)))
library.prove('fraction_mul_positive', [('a', Q), ('b', Q), ('ha', positive(a)), ('hb', positive(b))],
    call('mul_positive', den(a), den(b), var('ha'), var('hb')), positive(times(a, b)))

e = var('e')
left, right = times(times(a, b), d), times(c, e)
target = times(times(a, d), b)
library.prove('fraction_mul_le_rotate', [('a', Q), ('b', Q), ('c', Q), ('d', Q), ('e', Q),
    ('ab', order(times(a, b), c)), ('de', order(d, e))],
    reframe(mul(num(left), den(right)), mul(num(right), den(left)),
        call('fraction_mul_le', times(a, b), c, d, e, var('ab'), var('de')),
        mul(num(target), den(right)), mul(num(right), den(target))), order(target, right))

f = var('f')
left, right = times(times(a, b), times(c, d)), times(e, f)
target = times(times(a, c), times(b, d))
library.prove('fraction_mul_le_interleave', [('a', Q), ('b', Q), ('c', Q), ('d', Q), ('e', Q), ('f', Q),
    ('ab', order(times(a, b), e)), ('cd', order(times(c, d), f))],
    reframe(mul(num(left), den(right)), mul(num(right), den(left)),
        call('fraction_mul_le', times(a, b), e, times(c, d), f, var('ab'), var('cd')),
        mul(num(target), den(right)), mul(num(right), den(target))), order(target, right))

for name, params, left, right in [
    ('fraction_one_left', [('a', Q)], times(one, a), a),
    ('fraction_one_right', [('a', Q)], times(a, one), a),
    ('fraction_mul_assoc', [('a', Q), ('b', Q), ('c', Q)], times(times(a, b), c), times(a, times(b, c))),
    ('fraction_mul_comm', [('a', Q), ('b', Q)], times(a, b), times(b, a)),
    ('fraction_mul_shuffle', [('a', Q), ('b', Q), ('c', Q), ('d', Q)],
        times(times(a, b), times(c, d)), times(times(a, c), times(b, d))),
]:
    library.prove(name, params, equal_proof(left, right), equal(left, right))

# Equality composition stays in cross products; there is no hidden reduction
# or gcd oracle for rational values.
left, right = times(a, c), times(b, d)
l = mul(mul(num(a), den(b)), mul(num(c), den(d)))
r = mul(mul(num(b), den(a)), mul(num(d), den(c)))
path = ring(mul(num(left), den(right)), l).then(
    Path(mul(num(a), den(b)), mul(num(b), den(a)), var('ab')).under(lambda z: mul(z, mul(num(c), den(d))))).then(
    Path(mul(num(c), den(d)), mul(num(d), den(c)), var('cd')).under(lambda z: mul(mul(num(b), den(a)), z))).then(
    ring(r, mul(num(right), den(left))))
library.prove('fraction_mul_equal', [('a', Q), ('b', Q), ('c', Q), ('d', Q),
    ('ab', equal(a, b)), ('cd', equal(c, d))], path.proof, equal(left, right))

def antisym_goal(x):
    return bind(pi, [('b', N), ('ab', le(x, b)), ('ba', le(b, x))], eq(x, b))
from portable_terms import induct
from peano_arithmetic import EMPTY
library.prove('le_antisymmetric', [('a', N), ('b', N), ('ab', le(a, b)), ('ba', le(b, a))],
    app(induction('a', antisym_goal,
        lam('b', N, induction('b', lambda y: bind(pi, [('ab', le(Z, y)), ('ba', le(y, Z))], eq(Z, y)),
            bind(lam, [('ab', le(Z, Z)), ('ba', le(Z, Z))], refl(Z)),
            lambda t, _: bind(lam, [('ab', le(Z, S(t))), ('ba', le(S(t), Z))],
                induct(var('ba'), lam('_absurd', EMPTY, eq(Z, S(t))))))),
        lambda t, ih: lam('b', N, induction('b',
            lambda y: bind(pi, [('ab', le(S(t), y)), ('ba', le(y, S(t)))], eq(S(t), y)),
            bind(lam, [('ab', le(S(t), Z)), ('ba', le(Z, S(t)))],
                induct(var('ab'), lam('_absurd', EMPTY, eq(S(t), Z)))),
            lambda u, _: bind(lam, [('ab', le(S(t), S(u))), ('ba', le(S(u), S(t)))],
                Path(t, u, app(ih, u, var('ab'), var('ba'))).under(S).proof)))),
        b, var('ab'), var('ba')), eq(a, b))

def equal_order(x, y, h):
    return call('fraction_equal_le', x, y, h)

def reverse_equal(x, y, h):
    return Path(mul(num(x), den(y)), mul(num(y), den(x)), h).reverse().proof

library.prove('fraction_equal_trans', [('a', Q), ('b', Q), ('c', Q), ('positive', positive(b)),
    ('ab', equal(a, b)), ('bc', equal(b, c))],
    call('le_antisymmetric', mul(num(a), den(c)), mul(num(c), den(a)),
        call('fraction_trans', a, b, c, var('positive'),
             equal_order(a, b, var('ab')), equal_order(b, c, var('bc'))),
        call('fraction_trans', c, b, a, var('positive'),
             equal_order(c, b, reverse_equal(b, c, var('bc'))),
             equal_order(b, a, reverse_equal(a, b, var('ab'))))), equal(a, c))
