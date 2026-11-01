"""Exact fixed-alphabet KT inequalities, built from constructive arithmetic.

The parameter k denotes K-1, so the declared alphabet K is always positive.
R(a,n) = product(i<n, a+2*i). Ratios of these integer products are exactly
the rising-half ratios; no floating point or logarithms enter the proof.
"""
import positive_fractions as fractions
from peano_arithmetic import (Library, N, Z, ONE, S, Path, add, mul, le, eq,
    call, change_order, ring, law, induction)
from positive_fractions import Q, fraction, num, den, times, one, order, equal, positive, reframe
from portable_terms import (app, bind, lam, pi, var, induct, tt, refl, pair,
    list_type, nil, cons, j, identity, congruence, sigma, first, second, unit, universe)

library = Library()
library.definitions = fractions.library.definitions.copy()
library.claims = fractions.library.claims.copy()
FOUNDATIONS = set(library.claims)
k, a, n, u, v = map(var, ('k', 'a', 'n', 'u', 'v'))
double = lambda n: add(n, n)
factor = lambda a, n: add(a, double(n))
rise = lambda a, n: call('rise2', a, n)
pure = lambda k, n: fraction(rise(ONE, n), rise(S(k), n))
step = lambda k, n: fraction(factor(ONE, n), factor(S(k), n))

library.define('rise2', bind(lam, [('a', N), ('n', N)],
    induction('n', lambda _: N, ONE, lambda tail, ih: mul(ih, factor(a, tail)))))
library.prove('le_add', [('a', N), ('n', N)],
    change_order(add(a, Z), add(a, n), call('add_le_left', a, Z, n, tt()),
        law('add_zero', add(a, Z), a, a)), le(a, add(a, n)))
library.prove('rise2_positive', [('a', N), ('n', N), ('ha', le(ONE, a))],
    induction('n', lambda x: le(ONE, rise(a, x)), tt(),
        lambda tail, ih: call('mul_positive', rise(a, tail), factor(a, tail), ih,
            call('le_trans', ONE, a, factor(a, tail), var('ha'), call('le_add', a, double(tail))))),
    le(ONE, rise(a, n)))

def rise_merge_step(t, ih):
    left = mul(mul(rise(a, t), rise(a, v)), factor(a, t))
    right = mul(rise(a, add(t, v)), factor(a, add(t, v)))
    factor_le = call('add_le_left', a, double(t), double(add(t, v)),
        call('add_le', t, add(t, v), t, add(t, v), call('le_add', t, v), call('le_add', t, v)))
    proof = call('mul_le', mul(rise(a, t), rise(a, v)), rise(a, add(t, v)),
                 factor(a, t), factor(a, add(t, v)), ih, factor_le)
    return reframe(left, right, proof, mul(mul(rise(a, t), factor(a, t)), rise(a, v)), right)

library.prove('rise2_supermultiplicative', [('a', N), ('u', N), ('v', N)],
    induction('u', lambda x: le(mul(rise(a, x), rise(a, v)), rise(a, add(x, v))),
        change_order(rise(a, v), rise(a, v), call('le_refl', rise(a, v)),
            ring(rise(a, v), mul(ONE, rise(a, v)))), rise_merge_step),
    le(mul(rise(a, u), rise(a, v)), rise(a, add(u, v))))

# x/(x+d) <= (x+y)/(x+d+y), witnessed by the nonnegative gap y*d.
# Factor the scalar identity before substitution to avoid expanding repeated
# KT factors throughout the polynomial certificate.
x, d, y = map(var, ('x', 'd', 'y'))
left = mul(x, add(add(x, d), y))
right = mul(add(x, y), add(x, d))
library.prove('ratio_step_cross', [('x', N), ('d', N), ('y', N)],
    change_order(left, add(left, mul(y, d)), call('le_add', left, mul(y, d)),
        right_path=ring(add(left, mul(y, d)), right)), le(left, right))

def step_cross(k, u, v):
    x, y = factor(ONE, u), double(v)
    dl = ring(factor(S(k), u), add(x, k))
    nr = ring(factor(ONE, add(u, v)), add(x, y))
    dr = ring(factor(S(k), add(u, v)), add(add(x, k), y))
    return change_order(mul(x, add(add(x, k), y)), mul(add(x, y), add(x, k)),
        call('ratio_step_cross', x, k, y),
        dr.reverse().under(lambda z: mul(x, z)),
        nr.reverse().under(lambda z: mul(z, add(x, k))).then(
            dl.reverse().under(lambda z: mul(factor(ONE, add(u, v)), z))))

library.prove('pure_step_monotone', [('k', N), ('u', N), ('v', N)],
    step_cross(k, u, v), order(step(k, u), step(k, add(u, v))))
library.prove('pure_positive', [('k', N), ('n', N)],
    call('rise2_positive', S(k), n, tt()), positive(pure(k, n)))

def pure_merge_step(t, ih):
    p, q, r = pure(k, t), pure(k, v), pure(k, add(t, v))
    s, z = step(k, t), step(k, add(t, v))
    return call('fraction_mul_le_rotate', p, q, r, s, z, ih,
                call('pure_step_monotone', k, t, v))

library.prove('pure_merge', [('k', N), ('u', N), ('v', N)],
    induction('u', lambda x: order(times(pure(k, x), pure(k, v)), pure(k, add(x, v))),
        call('fraction_equal_le', times(one, pure(k, v)), pure(k, v),
             call('fraction_one_left', pure(k, v))), pure_merge_step),
    order(times(pure(k, u), pure(k, v)), pure(k, add(u, v))))

L = list_type(N)
xs = var('xs')
sum_ = lambda xs: call('count_sum', xs)
numerator = lambda a, xs: call('rising_product', a, xs)
pure_product = lambda k, xs: call('pure_product', k, xs)
cell = lambda k, xs: fraction(numerator(ONE, xs), rise(S(k), sum_(xs)))

def list_induction(name, goal, base, step):
    head, tail, ih = map(var, (name + '_head', name + '_tail', name + '_ih'))
    return induct(var(name), lam(name + '_subject', L, goal(var(name + '_subject'))), base,
        bind(lam, [(name + '_head', N), (name + '_tail', L), (name + '_ih', goal(tail))], step(head, tail, ih)))

library.define('count_sum', lam('xs', L,
    list_induction('xs', lambda _: N, Z, lambda h, _, ih: add(h, ih))))
library.define('rising_product', bind(lam, [('a', N), ('xs', L)],
    list_induction('xs', lambda _: N, ONE, lambda h, _, ih: mul(rise(a, h), ih))))
library.define('pure_product', bind(lam, [('k', N), ('xs', L)],
    list_induction('xs', lambda _: Q, one, lambda h, _, ih: times(pure(k, h), ih))))
library.prove('cell_positive', [('k', N), ('xs', L)],
    call('rise2_positive', S(k), sum_(xs), tt()), positive(cell(k, xs)))
library.prove('pure_product_positive', [('k', N), ('xs', L)],
    list_induction('xs', lambda x: positive(pure_product(k, x)), tt(),
        lambda h, t, ih: call('fraction_mul_positive', pure(k, h), pure_product(k, t),
                              call('pure_positive', k, h), ih)), positive(pure_product(k, xs)))

library.prove('cell_split_head', [('k', N), ('n', N), ('xs', L)],
    call('mul_le_left', mul(rise(ONE, n), numerator(ONE, xs)),
         mul(rise(S(k), n), rise(S(k), sum_(xs))), rise(S(k), add(n, sum_(xs))),
         call('rise2_supermultiplicative', S(k), n, sum_(xs))),
    order(cell(k, cons(N, n, xs)), times(pure(k, n), cell(k, xs))))

def split_step(h, t, ih):
    a, b, c = cell(k, cons(N, h, t)), times(pure(k, h), cell(k, t)), times(pure(k, h), pure_product(k, t))
    return call('fraction_trans', a, b, c,
        call('fraction_mul_positive', pure(k, h), cell(k, t), call('pure_positive', k, h), call('cell_positive', k, t)),
        call('cell_split_head', k, h, t),
        call('fraction_mul_le', pure(k, h), pure(k, h), cell(k, t), pure_product(k, t),
             call('fraction_refl', pure(k, h)), ih))

library.prove('cell_split', [('k', N), ('xs', L)],
    list_induction('xs', lambda x: order(cell(k, x), pure_product(k, x)), tt(), split_step),
    order(cell(k, xs), pure_product(k, xs)))

ys = var('ys')
merge = lambda xs, ys: call('merge_counts', xs, ys)
library.define('merge_counts', lam('xs', L,
    list_induction('xs', lambda _: pi('ys', L, L), lam('ys', L, ys),
        lambda h, t, ih: lam('ys', L,
            list_induction('ys', lambda _: L, cons(N, h, t),
                lambda y, rest, _: cons(N, add(h, y), app(ih, rest)))))))

def merge_product_step(h, t, ih):
    def step_y(y, rest, _):
        return call('fraction_mul_le_interleave', pure(k, h), pure(k, y),
            pure_product(k, t), pure_product(k, rest), pure(k, add(h, y)), pure_product(k, merge(t, rest)),
            call('pure_merge', k, h, y), app(ih, rest))
    return lam('ys', L, list_induction('ys',
        lambda y: order(times(pure_product(k, cons(N, h, t)), pure_product(k, y)), pure_product(k, merge(cons(N, h, t), y))),
        call('fraction_equal_le', times(pure_product(k, cons(N, h, t)), one), pure_product(k, cons(N, h, t)),
             call('fraction_one_right', pure_product(k, cons(N, h, t)))), step_y))

library.prove('pure_product_merge', [('k', N), ('xs', L), ('ys', L)],
    app(list_induction('xs', lambda x: pi('ys', L,
            order(times(pure_product(k, x), pure_product(k, ys)), pure_product(k, merge(x, ys)))),
        lam('ys', L, call('fraction_equal_le', times(one, pure_product(k, ys)), pure_product(k, ys),
                         call('fraction_one_left', pure_product(k, ys)))), merge_product_step), ys),
    order(times(pure_product(k, xs), pure_product(k, ys)), pure_product(k, merge(xs, ys))))

zeros = lambda n: call('zero_counts', n)
library.define('zero_counts', lam('n', N,
    induction('n', lambda _: L, nil(N), lambda _, ih: cons(N, Z, ih))))
library.prove('pure_product_zeros', [('k', N), ('n', N)],
    induction('n', lambda x: identity(Q, pure_product(k, zeros(x)), one), refl(one),
        lambda t, ih: congruence(Q, Q, lam('_product', Q, times(one, var('_product'))),
                                pure_product(k, zeros(t)), one, ih)),
    identity(Q, pure_product(k, zeros(n)), one))

Rows = list_type(L)
rows = var('rows')
product = lambda k, rows: call('cell_product', k, rows)
histogram = lambda width, rows: call('histogram', width, rows)

def rows_induction(name, goal, base, step):
    head, tail, ih = map(var, (name + '_head', name + '_tail', name + '_ih'))
    return induct(var(name), lam(name + '_subject', Rows, goal(var(name + '_subject'))), base,
        bind(lam, [(name + '_head', L), (name + '_tail', Rows), (name + '_ih', goal(tail))], step(head, tail, ih)))

library.define('cell_product', bind(lam, [('k', N), ('rows', Rows)],
    rows_induction('rows', lambda _: Q, one, lambda head, _, ih: times(cell(k, head), ih))))
width = var('width')
library.define('histogram', bind(lam, [('width', N), ('rows', Rows)],
    rows_induction('rows', lambda _: L, zeros(width), lambda head, _, ih: merge(head, ih))))

def partition_step(h, t, ih):
    left = times(cell(k, h), product(k, t))
    middle = times(pure_product(k, h), pure_product(k, histogram(width, t)))
    right = pure_product(k, merge(h, histogram(width, t)))
    return call('fraction_trans', left, middle, right,
        call('fraction_mul_positive', pure_product(k, h), pure_product(k, histogram(width, t)),
             call('pure_product_positive', k, h), call('pure_product_positive', k, histogram(width, t))),
        call('fraction_mul_le', cell(k, h), pure_product(k, h), product(k, t), pure_product(k, histogram(width, t)),
             call('cell_split', k, h), ih), call('pure_product_merge', k, h, histogram(width, t)))

# The width is independent of K for the algebra. A K-outcome interpretation
# must additionally check each row's width and the declared alphabet identity.
library.prove('partition_bound', [('k', N), ('width', N), ('rows', Rows)],
    rows_induction('rows', lambda r: order(product(k, r), pure_product(k, histogram(width, r))),
        j(lam('_value', Q, order(var('_value'), pure_product(k, zeros(width)))),
          call('fraction_refl', pure_product(k, zeros(width))), call('pure_product_zeros', k, width)),
        partition_step),
    order(product(k, rows), pure_product(k, histogram(width, rows))))

library.prove('fixed_histogram_bound', [('k', N), ('width', N), ('rows', Rows), ('xs', L),
    ('same_counts', identity(L, histogram(width, rows), xs))],
    j(lam('_counts', L, order(product(k, rows), pure_product(k, var('_counts')))),
      call('partition_bound', k, width, rows), var('same_counts')),
    order(product(k, rows), pure_product(k, xs)))

length = lambda xs: call('count_length', xs)
prefix = lambda rows: call('prefix_zero_rows', rows)
pure_rows = lambda xs: call('pure_rows', xs)
library.define('count_length', lam('xs', L,
    list_induction('xs', lambda _: N, Z, lambda _, __, ih: S(ih))))
library.define('prefix_zero_rows', lam('rows', Rows,
    rows_induction('rows', lambda _: Rows, nil(L), lambda h, _, ih: cons(L, cons(N, Z, h), ih))))
library.define('pure_rows', lam('xs', L,
    list_induction('xs', lambda _: Rows, nil(L),
        lambda h, t, ih: cons(L, cons(N, h, zeros(length(t))), prefix(ih)))))
library.prove('merge_zero_left', [('xs', L)],
    list_induction('xs', lambda x: identity(L, merge(zeros(length(x)), x), x), refl(nil(N)),
        lambda h, t, ih: Path(merge(zeros(length(t)), t), t, ih, L).under(lambda z: cons(N, h, z)).proof),
    identity(L, merge(zeros(length(xs)), xs), xs))
library.prove('histogram_prefix_zero', [('width', N), ('rows', Rows)],
    rows_induction('rows',
        lambda r: identity(L, histogram(S(width), prefix(r)), cons(N, Z, histogram(width, r))),
        refl(cons(N, Z, zeros(width))),
        lambda h, t, ih: Path(histogram(S(width), prefix(t)), cons(N, Z, histogram(width, t)), ih, L).under(
            lambda z: merge(cons(N, Z, h), z)).proof),
    identity(L, histogram(S(width), prefix(rows)), cons(N, Z, histogram(width, rows))))

def histogram_pure_step(h, t, ih):
    w = length(t); p = pure_rows(t); row = cons(N, h, zeros(w))
    return Path(histogram(S(w), prefix(p)), cons(N, Z, histogram(w, p)),
                call('histogram_prefix_zero', w, p), L).under(lambda z: merge(row, z)).then(
        Path(histogram(w, p), t, ih, L).under(lambda z: cons(N, add(h, Z), merge(zeros(w), z)))).then(
        Path(merge(zeros(w), t), t, call('merge_zero_left', t), L).under(lambda z: cons(N, add(h, Z), z))).then(
        Path(add(h, Z), h, call('add_zero', h)).under(lambda z: cons(N, z, t), L)).proof

library.prove('histogram_pure_rows', [('xs', L)],
    list_induction('xs', lambda x: identity(L, histogram(length(x), pure_rows(x)), x),
        refl(nil(N)), histogram_pure_step), identity(L, histogram(length(xs), pure_rows(xs)), xs))

library.prove('sum_zero_counts', [('n', N)],
    induction('n', lambda x: eq(sum_(zeros(x)), Z), refl(Z), lambda _, ih: ih), eq(sum_(zeros(n)), Z))
library.prove('rising_product_zeros', [('a', N), ('n', N)],
    induction('n', lambda x: eq(numerator(a, zeros(x)), ONE), refl(ONE),
        lambda t, ih: Path(numerator(a, zeros(t)), ONE, ih).under(lambda z: mul(ONE, z)).proof),
    eq(numerator(a, zeros(n)), ONE))
library.prove('cell_product_positive', [('k', N), ('rows', Rows)],
    rows_induction('rows', lambda r: positive(product(k, r)), tt(),
        lambda h, t, ih: call('fraction_mul_positive', cell(k, h), product(k, t), call('cell_positive', k, h), ih)),
    positive(product(k, rows)))

cell_left, cell_right = cell(k, cons(N, Z, xs)), cell(k, xs)
library.prove('cell_prefix_zero', [('k', N), ('xs', L)],
    ring(mul(mul(ONE, numerator(ONE, xs)), den(cell_right)), mul(num(cell_right), den(cell_right))).proof,
    equal(cell_left, cell_right))
library.prove('cell_product_prefix_zero', [('k', N), ('rows', Rows)],
    rows_induction('rows', lambda r: equal(product(k, prefix(r)), product(k, r)), refl(ONE),
        lambda h, t, ih: call('fraction_mul_equal', cell(k, cons(N, Z, h)), cell(k, h),
            product(k, prefix(t)), product(k, t), call('cell_prefix_zero', k, h), ih)),
    equal(product(k, prefix(rows)), product(k, rows)))

row = cons(N, n, zeros(width))
num_path = Path(numerator(ONE, zeros(width)), ONE, call('rising_product_zeros', ONE, width)).under(
    lambda z: mul(rise(ONE, n), z)).then(law('mul_one', mul(rise(ONE, n), ONE), rise(ONE, n), rise(ONE, n)))
den_path = Path(sum_(zeros(width)), Z, call('sum_zero_counts', width)).under(lambda z: add(n, z)).then(
    law('add_zero', add(n, Z), n, n)).under(lambda z: rise(S(k), z))
library.prove('single_outcome_cell', [('k', N), ('n', N), ('width', N)],
    num_path.under(lambda z: mul(z, rise(S(k), n))).then(
        den_path.under(lambda z: mul(rise(ONE, n), z)).reverse()).proof,
    equal(cell(k, row), pure(k, n)))

def attained_step(h, t, ih):
    p = pure_rows(t)
    tail_path = call('fraction_equal_trans', product(k, prefix(p)), product(k, p), pure_product(k, t),
        call('cell_product_positive', k, p), call('cell_product_prefix_zero', k, p), ih)
    return call('fraction_mul_equal', cell(k, cons(N, h, zeros(length(t)))), pure(k, h),
        product(k, prefix(p)), pure_product(k, t), call('single_outcome_cell', k, h, length(t)), tail_path)

library.prove('pure_rows_attains', [('k', N), ('xs', L)],
    list_induction('xs', lambda x: equal(product(k, pure_rows(x)), pure_product(k, x)), refl(ONE), attained_step),
    equal(product(k, pure_rows(xs)), pure_product(k, xs)))

from positive_fractions import reverse_equal
library.prove('fixed_histogram_maximum', [('k', N), ('xs', L), ('rows', Rows),
    ('same_counts', identity(L, histogram(length(xs), rows), xs))],
    call('fraction_trans', product(k, rows), pure_product(k, xs), product(k, pure_rows(xs)),
        call('pure_product_positive', k, xs), call('fixed_histogram_bound', k, length(xs), rows, xs, var('same_counts')),
        call('fraction_equal_le', pure_product(k, xs), product(k, pure_rows(xs)),
            reverse_equal(product(k, pure_rows(xs)), pure_product(k, xs), call('pure_rows_attains', k, xs)))),
    order(product(k, rows), product(k, pure_rows(xs))))

row_width = lambda w, rows: call('rows_width', w, rows)
library.define('rows_width', bind(lam, [('width', N), ('rows', Rows)],
    rows_induction('rows', lambda _: universe(), unit(),
        lambda h, _, ih: sigma('_head_width', eq(length(h), width), ih))))
library.prove('length_zero_counts', [('n', N)],
    induction('n', lambda x: eq(length(zeros(x)), x), refl(Z),
        lambda t, ih: Path(length(zeros(t)), t, ih).under(S).proof), eq(length(zeros(n)), n))
library.prove('rows_width_prefix', [('width', N), ('rows', Rows), ('widths', row_width(width, rows))],
    app(rows_induction('rows', lambda r: pi('widths', row_width(width, r), row_width(S(width), prefix(r))),
        lam('widths', unit(), tt()),
        lambda h, t, ih: lam('widths', row_width(width, cons(L, h, t)),
            pair(Path(length(h), width, first(var('widths'))).under(S).proof,
                 app(ih, second(var('widths')))))), var('widths')),
    row_width(S(width), prefix(rows)))
library.prove('pure_rows_width', [('xs', L)],
    list_induction('xs', lambda x: row_width(length(x), pure_rows(x)), tt(),
        lambda _, t, ih: pair(
            Path(length(zeros(length(t))), length(t), call('length_zero_counts', length(t))).under(S).proof,
            call('rows_width_prefix', length(t), pure_rows(t), ih))),
    row_width(length(xs), pure_rows(xs)))

zero_num = Path(numerator(ONE, zeros(width)), ONE, call('rising_product_zeros', ONE, width))
zero_den = Path(sum_(zeros(width)), Z, call('sum_zero_counts', width)).under(lambda z: rise(S(k), z))
library.prove('cell_zero_counts', [('k', N), ('width', N)],
    zero_num.under(lambda z: mul(z, ONE)).then(zero_den.under(lambda z: mul(ONE, z)).reverse()).proof,
    equal(cell(k, zeros(width)), one))
library.prove('cell_product_zero_head', [('k', N), ('width', N), ('rows', Rows)],
    call('fraction_equal_trans', times(cell(k, zeros(width)), product(k, rows)), times(one, product(k, rows)), product(k, rows),
        call('fraction_mul_positive', one, product(k, rows), tt(), call('cell_product_positive', k, rows)),
        call('fraction_mul_equal', cell(k, zeros(width)), one, product(k, rows), product(k, rows),
             call('cell_zero_counts', k, width), refl(mul(num(product(k, rows)), den(product(k, rows))))),
        call('fraction_one_left', product(k, rows))),
    equal(product(k, cons(L, zeros(width), rows)), product(k, rows)))

before, after = var('before'), var('after')
append = lambda before, after: call('append_rows', before, after)
library.define('append_rows', bind(lam, [('before', Rows), ('after', Rows)],
    rows_induction('before', lambda _: Rows, after, lambda h, _, ih: cons(L, h, ih))))
library.prove('cell_product_delete_zero', [('k', N), ('width', N), ('before', Rows), ('after', Rows)],
    rows_induction('before',
        lambda b: equal(product(k, append(b, cons(L, zeros(width), after))), product(k, append(b, after))),
        call('cell_product_zero_head', k, width, after),
        lambda h, t, ih: call('fraction_mul_equal', cell(k, h), cell(k, h),
            product(k, append(t, cons(L, zeros(width), after))), product(k, append(t, after)),
            refl(mul(num(cell(k, h)), den(cell(k, h)))), ih)),
    equal(product(k, append(before, cons(L, zeros(width), after))), product(k, append(before, after))))

power = lambda a, n: call('natural_power', a, n)
self_product = lambda xs: call('self_power_product', xs)
parent = lambda xs: fraction(self_product(xs), power(sum_(xs), sum_(xs)))
inverse_parent = lambda xs: fraction(den(parent(xs)), num(parent(xs)))
library.define('natural_power', bind(lam, [('a', N), ('n', N)],
    induction('n', lambda _: N, ONE, lambda _, ih: mul(ih, a))))
library.prove('power_positive', [('a', N), ('n', N), ('ha', le(ONE, a))],
    induction('n', lambda x: le(ONE, power(a, x)), tt(),
        lambda t, ih: call('mul_positive', power(a, t), a, ih, var('ha'))), le(ONE, power(a, n)))
library.prove('self_power_positive', [('n', N)],
    induction('n', lambda x: le(ONE, power(x, x)), tt(),
        lambda t, _: call('power_positive', S(t), S(t), tt())), le(ONE, power(n, n)))
library.define('self_power_product', lam('xs', L,
    list_induction('xs', lambda _: N, ONE, lambda h, _, ih: mul(power(h, h), ih))))
library.prove('self_product_positive', [('xs', L)],
    list_induction('xs', lambda x: le(ONE, self_product(x)), tt(),
        lambda h, t, ih: call('mul_positive', power(h, h), self_product(t), call('self_power_positive', h), ih)),
    le(ONE, self_product(xs)))
library.prove('parent_ml_positive', [('xs', L)], call('self_power_positive', sum_(xs)), positive(parent(xs)))
library.prove('parent_ml_inverse_positive', [('xs', L)], call('self_product_positive', xs), positive(inverse_parent(xs)))
library.prove('fixed_histogram_parent_ml_ratio', [('k', N), ('width', N), ('rows', Rows), ('xs', L),
    ('same_counts', identity(L, histogram(width, rows), xs))],
    call('fraction_mul_le', product(k, rows), pure_product(k, xs), inverse_parent(xs), inverse_parent(xs),
        call('fixed_histogram_bound', k, width, rows, xs, var('same_counts')), call('fraction_refl', inverse_parent(xs))),
    order(times(product(k, rows), inverse_parent(xs)), times(pure_product(k, xs), inverse_parent(xs))))

# Counter correspondence: each symbol starts occurrence at zero while the
# denominator cursor continues across symbols. This is an exact factor-range
# theorem, independent of the order in which the frozen frequencies are read.
shifted = lambda a, offset, n: call('shifted_factors', a, offset, n)
offset = var('offset')
library.define('shifted_factors', bind(lam, [('a', N), ('offset', N), ('n', N)],
    induction('n', lambda _: N, ONE, lambda t, ih: mul(ih, factor(a, add(offset, t))))))
base_range = law('mul_one', mul(rise(a, offset), ONE), rise(a, offset), rise(a, offset)).then(
    law('add_zero', add(offset, Z), offset, offset).under(lambda z: rise(a, z)).reverse())
def range_step(t, ih):
    f = factor(a, add(offset, t))
    return law('mul_assoc', mul(mul(rise(a, offset), shifted(a, offset, t)), f),
        mul(rise(a, offset), mul(shifted(a, offset, t), f)), rise(a, offset), shifted(a, offset, t), f).reverse().then(
        Path(mul(rise(a, offset), shifted(a, offset, t)), rise(a, add(offset, t)), ih).under(lambda z: mul(z, f))).then(
        law('add_succ', add(offset, S(t)), S(add(offset, t)), offset, t).under(lambda z: rise(a, z)).reverse()).proof
library.prove('rising_factor_range', [('a', N), ('offset', N), ('n', N)],
    induction('n', lambda t: eq(mul(rise(a, offset), shifted(a, offset, t)), rise(a, add(offset, t))),
        base_range.proof, range_step), eq(mul(rise(a, offset), shifted(a, offset, n)), rise(a, add(offset, n))))
library.define('counter_denominator', bind(lam, [('a', N), ('xs', L)],
    list_induction('xs', lambda _: pi('offset', N, N), lam('offset', N, ONE),
        lambda h, _, ih: lam('offset', N, mul(shifted(a, offset, h), app(ih, add(offset, h)))))))

# The producer orders arguments as (a, xs, offset), matching the list fold.
counter_den = lambda a, offset, xs: call('counter_denominator', a, xs, offset)
def counter_range_step(h, t, ih):
    r, s, d = rise(a, offset), shifted(a, offset, h), counter_den(a, add(offset, h), t)
    path = law('mul_assoc', mul(mul(r, s), d), mul(r, mul(s, d)), r, s, d).reverse().then(
        Path(mul(r, s), rise(a, add(offset, h)), call('rising_factor_range', a, offset, h)).under(lambda z: mul(z, d))).then(
        Path(mul(rise(a, add(offset, h)), d), rise(a, add(add(offset, h), sum_(t))), app(ih, add(offset, h)))).then(
        law('add_assoc', add(add(offset, h), sum_(t)), add(offset, add(h, sum_(t))), offset, h, sum_(t)).under(lambda z: rise(a, z)))
    return lam('offset', N, path.proof)
library.prove('counter_denominator_range', [('a', N), ('xs', L), ('offset', N)],
    app(list_induction('xs', lambda x: pi('offset', N,
        eq(mul(rise(a, offset), counter_den(a, offset, x)), rise(a, add(offset, sum_(x))))),
        lam('offset', N, base_range.proof), counter_range_step), offset),
    eq(mul(rise(a, offset), counter_den(a, offset, xs)), rise(a, add(offset, sum_(xs)))))
library.prove('counter_zero_origin', [('a', N), ('xs', L)],
    law('add_zero', add(counter_den(a, Z, xs), Z), counter_den(a, Z, xs), counter_den(a, Z, xs)).reverse().then(
        Path(mul(ONE, counter_den(a, Z, xs)), rise(a, sum_(xs)), call('counter_denominator_range', a, xs, Z))).proof,
    eq(counter_den(a, Z, xs), rise(a, sum_(xs))))


def cases():
    from portable_terms import lower
    result = [(name, library.certificate(name), True) for name in library.claims if name not in FOUNDATIONS]
    def number(n):
        result = Z
        for _ in range(n):
            result = S(result)
        return result
    def counts(values):
        result = nil(N)
        for value in reversed(values):
            result = cons(N, number(value), result)
        return result
    false_equalities = [
        ('changed_declared_alphabet', pure(ONE, number(2)), pure(number(2), number(2))),
        ('equal_totals_replace_histogram', pure_product(ONE, counts([3, 0])), pure_product(ONE, counts([2, 1]))),
        ('mixed_cell_attains', cell(ONE, counts([1, 1])), pure_product(ONE, counts([1, 1]))),
    ]
    invalid = [(name, refl(mul(num(left), den(right))), equal(left, right)) for name, left, right in false_equalities]
    invalid += [
        ('reverse_pure_merge', tt(), order(pure(ONE, number(2)), times(pure(ONE, ONE), pure(ONE, ONE)))),
        ('zero_alphabet_positive', tt(), le(ONE, rise(Z, ONE))),
    ]
    params = [('k', N), ('width', N), ('rows', Rows), ('xs', L), ('same_counts', identity(L, histogram(width, rows), xs))]
    invalid.append(('ignore_histogram_identity', bind(lam, params, call('partition_bound', k, width, rows)),
                    bind(pi, params, order(product(k, rows), pure_product(k, xs)))))
    for name, proof, claim in invalid:
        result.append((name, {'term': lower(library.close(proof)), 'claimed_type': lower(library.close(claim)), 'metadata': {}}, False))
    return result
