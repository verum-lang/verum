"""Untrusted producers for constructive natural arithmetic.

Every law is an ordinary induction/identity term, not a kernel arithmetic
extension. Polynomial normalization below produces a path using these laws;
the producer's Python arithmetic has no proof authority.
"""
from dataclasses import dataclass

from portable_terms import (RECURSIVE, app, arrow, bind, congruence, construct,
    data, identity, induct, j, lam, local, lower, pi, refl, symm, transitive,
    tt, unit, universe, var, sigma, pair, first, second)


def names(term):
    if not isinstance(term, (tuple, list)):
        return set()
    if term and term[0] == 'Var':
        return {term[1]}
    return set().union(*(names(child) for child in term))


class Library:
    def __init__(self):
        self.definitions = []
        self.claims = {}

    def define(self, name, value):
        if any(name == key for key, _ in self.definitions):
            raise ValueError('duplicate proof definition: ' + name)
        self.definitions.append((name, value))
        return var(name)

    def prove(self, name, params, proof, claim):
        self.define(name, bind(lam, params, proof))
        self.claims[name] = bind(pi, params, claim)

    def close(self, term):
        used = names(term)
        selected = []
        for name, value in reversed(self.definitions):
            if name in used:
                if name in self.claims:
                    used.update(names(self.claims[name]))
                selected.append((name, value))
                used.update(names(value))
        selected.reverse()
        proof_names = {name for name, _ in selected if name in self.claims}
        # Arithmetic computations do not depend on proof inhabitants. Check
        # that boundary before separating computation from the proof DAG.
        for name, value in selected:
            if name not in proof_names and names(value) & proof_names:
                raise ValueError('computational definition depends on a proof')
        for name in proof_names:
            if names(self.claims[name]) & proof_names:
                raise ValueError('proof-dependent claim requires a dependent library')
        levels, groups = {}, {}
        for name, value in selected:
            if name in proof_names:
                dependencies = names(value) & proof_names
                level = 1 + max((levels[parent] for parent in dependencies), default=-1)
                levels[name] = level
                groups.setdefault(level, []).append((name, value))
        substitutions, bundles = {}, []
        for level, group in groups.items():
            binder = f'__arithmetic_library_{level}'
            if binder in used:
                raise ValueError('reserved proof-library binder')
            def bundle(items, projection):
                if len(items) == 1:
                    name, value = items[0]
                    return (substitute(self.claims[name], substitutions), substitute(value, substitutions),
                            {name: projection})
                split = len(items) // 2
                left_type, left, left_refs = bundle(items[:split], first(projection))
                right_type, right, right_refs = bundle(items[split:], second(projection))
                return sigma('_bundle_left', left_type, right_type), pair(left, right), left_refs | right_refs
            ty, value, refs = bundle(group, var(binder))
            bundles.append((binder, ty, value))
            substitutions.update(refs)
        term = substitute(term, substitutions)
        # One checked cut per dependency layer, with balanced products for
        # independent lemmas. This keeps proofs opaque while checking their
        # users, without a linear-depth tower or new trusted kernel syntax.
        for binder, ty, value in reversed(bundles):
            term = app(lam(binder, ty, term), value)
        for name, value in reversed(selected):
            if name not in proof_names:
                term = local(name, value, term)
        return term

    def certificate(self, name):
        return {'term': lower(self.close(var(name))),
                'claimed_type': lower(self.close(self.claims[name])), 'metadata': {}}

    def cases(self):
        return [(name, self.certificate(name), True) for name in self.claims]


def substitute(term, replacements):
    if not isinstance(term, (tuple, list)):
        return term
    if isinstance(term, tuple) and term:
        if term[0] == 'Var':
            return replacements.get(term[1], term)
        if term[0] in ('Pi', 'Lam', 'Sigma', 'Let'):
            kind, binder, value, body = term
            scoped = {name: replacement for name, replacement in replacements.items() if name != binder}
            return kind, binder, substitute(value, replacements), substitute(body, scoped)
    values = [substitute(value, replacements) for value in term]
    return tuple(values) if isinstance(term, tuple) else values


library = Library()
N = library.define('Nat', data('Nat', [[], [RECURSIVE]]))
Z = construct(N, 0)
S = lambda n: construct(N, 1, n)
ONE = S(Z)
EMPTY = data('Empty', [])
add = lambda a, b: app(var('nat_add'), a, b)
mul = lambda a, b: app(var('nat_mul'), a, b)
le = lambda a, b: app(var('nat_le'), a, b)
eq = lambda a, b: identity(N, a, b)
call = lambda name, *args: app(var(name), *args)

library.prove('nat_eq_trans', [('x', N), ('y', N), ('z', N),
    ('xy', eq(var('x'), var('y'))), ('yz', eq(var('y'), var('z')))],
    transitive(N, var('x'), var('y'), var('z'), var('xy'), var('yz')), eq(var('x'), var('z')))
library.prove('nat_eq_symm', [('x', N), ('y', N), ('xy', eq(var('x'), var('y')))],
    symm(N, var('x'), var('y'), var('xy')), eq(var('y'), var('x')))
library.prove('nat_congr', [('f', arrow(N, N)), ('x', N), ('y', N), ('xy', eq(var('x'), var('y')))],
    congruence(N, N, var('f'), var('x'), var('y'), var('xy')),
    eq(app(var('f'), var('x')), app(var('f'), var('y'))))


@dataclass(frozen=True)
class Path:
    left: tuple
    right: tuple
    proof: tuple
    carrier: tuple = N

    def then(self, other):
        # Definitional conversion is checked by the kernels; endpoints may
        # differ syntactically after unfolding a recursive equation.
        if self.left == self.right:
            return Path(self.left, other.right, other.proof, self.carrier)
        if other.left == other.right:
            return Path(self.left, other.right, self.proof, self.carrier)
        return Path(self.left, other.right,
                    call('nat_eq_trans', self.left, self.right, other.right, self.proof, other.proof)
                    if self.carrier == N else transitive(self.carrier, self.left, self.right, other.right, self.proof, other.proof),
                    self.carrier)

    def reverse(self):
        if self.left == self.right:
            return same(self.left, carrier=self.carrier)
        return Path(self.right, self.left,
                    call('nat_eq_symm', self.left, self.right, self.proof) if self.carrier == N
                    else symm(self.carrier, self.left, self.right, self.proof), self.carrier)

    def under(self, function, carrier=None):
        target = self.carrier if carrier is None else carrier
        if self.left == self.right:
            return same(function(self.left), carrier=target)
        fn = lam('_rewrite', self.carrier, function(var('_rewrite')))
        return Path(function(self.left), function(self.right),
                    call('nat_congr', fn, self.left, self.right, self.proof) if self.carrier == target == N
                    else congruence(self.carrier, target, fn, self.left, self.right, self.proof), target)


def same(left, right=None, carrier=N):
    return Path(left, left if right is None else right, refl(left), carrier)


def law(name, left, right, *args):
    return Path(left, right, call(name, *args))


def induction(name, goal, base, step):
    tail, ih = var(name + '_tail'), var(name + '_ih')
    return induct(var(name), lam(name + '_subject', N, goal(var(name + '_subject'))),
                  base, bind(lam, [(name + '_tail', N), (name + '_ih', goal(tail))], step(tail, ih)))


a, b, c = map(var, ('a', 'b', 'c'))
library.define('nat_add', bind(lam, [('a', N), ('b', N)],
    induction('a', lambda _: N, b, lambda _, total: S(total))))
library.define('nat_mul', bind(lam, [('a', N), ('b', N)],
    induction('a', lambda _: N, Z, lambda _, product: add(b, product))))
library.define('nat_le', lam('a', N, induction('a', lambda _: arrow(N, universe()),
    lam('b', N, unit()), lambda _, order: lam('b', N,
        induction('b', lambda _: universe(), EMPTY, lambda tail, _: app(order, tail))))))

library.prove('add_zero', [('a', N)],
    induction('a', lambda x: eq(add(x, Z), x), refl(Z),
        lambda t, ih: Path(add(t, Z), t, ih).under(S).proof), eq(add(a, Z), a))
library.prove('add_succ', [('a', N), ('b', N)],
    induction('a', lambda x: eq(add(x, S(b)), S(add(x, b))), refl(S(b)),
        lambda t, ih: Path(add(t, S(b)), S(add(t, b)), ih).under(S).proof),
    eq(add(a, S(b)), S(add(a, b))))
library.prove('add_assoc', [('a', N), ('b', N), ('c', N)],
    induction('a', lambda x: eq(add(add(x, b), c), add(x, add(b, c))), refl(add(b, c)),
        lambda t, ih: Path(add(add(t, b), c), add(t, add(b, c)), ih).under(S).proof),
    eq(add(add(a, b), c), add(a, add(b, c))))
library.prove('add_comm', [('a', N), ('b', N)],
    induction('a', lambda x: eq(add(x, b), add(b, x)),
        law('add_zero', add(b, Z), b, b).reverse().proof,
        lambda t, ih: Path(add(t, b), add(b, t), ih).under(S).then(
            law('add_succ', add(b, S(t)), S(add(b, t)), b, t).reverse()).proof),
    eq(add(a, b), add(b, a)))


def operation(term, name):
    if (isinstance(term, tuple) and len(term) == 3 and term[0] == 'App'
            and term[1][0] == 'App' and term[1][1] == var(name)):
        return term[1][2], term[2]
    return None


def sum_terms(terms):
    if not terms:
        return Z
    if len(terms) == 1:
        return terms[0]
    return add(terms[0], sum_terms(terms[1:]))


def join_sum(left, right):
    """Path from two right-associated sums to their concatenation."""
    l, r = sum_terms(left), sum_terms(right)
    if not left:
        return same(add(l, r), r)
    if not right:
        return law('add_zero', add(l, r), l, l)
    if len(left) == 1:
        return same(add(l, r))
    return law('add_assoc', add(l, r), add(left[0], add(sum_terms(left[1:]), r)),
               left[0], sum_terms(left[1:]), r).then(
        join_sum(left[1:], right).under(lambda x: add(left[0], x)))


def swap_sum(head, next_, tail):
    if not tail:
        return law('add_comm', add(head, next_), add(next_, head), head, next_)
    t = sum_terms(tail)
    return law('add_assoc', add(add(head, next_), t), add(head, add(next_, t)),
               head, next_, t).reverse().then(
        law('add_comm', add(head, next_), add(next_, head), head, next_).under(lambda x: add(x, t))).then(
        law('add_assoc', add(add(next_, head), t), add(next_, add(head, t)), next_, head, t))


def sort_sum(terms):
    if len(terms) < 2:
        return terms, same(sum_terms(terms))
    rest, path = sort_sum(terms[1:])
    path = path.under(lambda x: add(terms[0], x))
    def insert(head, sorted_):
        if not sorted_ or repr(head) <= repr(sorted_[0]):
            return [head] + sorted_, same(sum_terms([head] + sorted_))
        tail, after = insert(head, sorted_[1:])
        moved = swap_sum(head, sorted_[0], sorted_[1:])
        return [sorted_[0]] + tail, moved.then(after.under(lambda x: add(sorted_[0], x)))
    result, inserted = insert(terms[0], rest)
    return result, path.then(inserted)


def additive_normalize(term):
    parts = operation(term, 'nat_add')
    if parts is None:
        return ([] if term == Z else [term]), same(term)
    x, y = parts
    xs, xp = additive_normalize(x)
    ys, yp = additive_normalize(y)
    path = xp.under(lambda v: add(v, y)).then(yp.under(lambda v: add(sum_terms(xs), v)))
    joined = path.then(join_sum(xs, ys))
    sorted_, ordered = sort_sum(xs + ys)
    return sorted_, joined.then(ordered)


def additive(left, right):
    xs, p = additive_normalize(left)
    ys, q = additive_normalize(right)
    if xs != ys:
        raise ValueError('different additive polynomials')
    return p.then(q.reverse())


library.prove('mul_zero', [('a', N)],
    induction('a', lambda x: eq(mul(x, Z), Z), refl(Z), lambda _, ih: ih), eq(mul(a, Z), Z))
library.prove('mul_succ', [('a', N), ('b', N)],
    induction('a', lambda x: eq(mul(x, S(b)), add(x, mul(x, b))), refl(Z),
        lambda t, ih: Path(mul(t, S(b)), add(t, mul(t, b)), ih).under(lambda x: add(S(b), x)).then(
            additive(add(b, add(t, mul(t, b))), add(t, add(b, mul(t, b)))).under(S)).proof),
    eq(mul(a, S(b)), add(a, mul(a, b))))
library.prove('mul_one', [('a', N)],
    induction('a', lambda x: eq(mul(x, ONE), x), refl(Z),
        lambda t, ih: Path(mul(t, ONE), t, ih).under(S).proof), eq(mul(a, ONE), a))
library.prove('mul_comm', [('a', N), ('b', N)],
    induction('a', lambda x: eq(mul(x, b), mul(b, x)),
        law('mul_zero', mul(b, Z), Z, b).reverse().proof,
        lambda t, ih: Path(mul(t, b), mul(b, t), ih).under(lambda x: add(b, x)).then(
            law('mul_succ', mul(b, S(t)), add(b, mul(b, t)), b, t).reverse()).proof),
    eq(mul(a, b), mul(b, a)))
library.prove('mul_add_left', [('a', N), ('b', N), ('c', N)],
    induction('a', lambda x: eq(mul(add(x, b), c), add(mul(x, c), mul(b, c))), refl(mul(b, c)),
        lambda t, ih: Path(mul(add(t, b), c), add(mul(t, c), mul(b, c)), ih).under(lambda x: add(c, x)).then(
            law('add_assoc', add(add(c, mul(t, c)), mul(b, c)),
                add(c, add(mul(t, c), mul(b, c))), c, mul(t, c), mul(b, c)).reverse()).proof),
    eq(mul(add(a, b), c), add(mul(a, c), mul(b, c))))
library.prove('mul_assoc', [('a', N), ('b', N), ('c', N)],
    induction('a', lambda x: eq(mul(mul(x, b), c), mul(x, mul(b, c))), refl(Z),
        lambda t, ih: law('mul_add_left', mul(add(b, mul(t, b)), c),
            add(mul(b, c), mul(mul(t, b), c)), b, mul(t, b), c).then(
                Path(mul(mul(t, b), c), mul(t, mul(b, c)), ih).under(lambda x: add(mul(b, c), x))).proof),
    eq(mul(mul(a, b), c), mul(a, mul(b, c))))
library.prove('mul_add_right', [('a', N), ('b', N), ('c', N)],
    law('mul_comm', mul(a, add(b, c)), mul(add(b, c), a), a, add(b, c)).then(
        law('mul_add_left', mul(add(b, c), a), add(mul(b, a), mul(c, a)), b, c, a)).then(
        law('mul_comm', mul(b, a), mul(a, b), b, a).under(lambda x: add(x, mul(c, a)))).then(
        law('mul_comm', mul(c, a), mul(a, c), c, a).under(lambda x: add(mul(a, b), x))).proof,
    eq(mul(a, add(b, c)), add(mul(a, b), mul(a, c))))


def change_order(left, right, proof, left_path=None, right_path=None):
    if left_path is not None:
        proof = j(lam('_left', N, le(var('_left'), right)), proof, left_path.proof)
        left = left_path.right
    if right_path is not None:
        proof = j(lam('_right', N, le(left, var('_right'))), proof, right_path.proof)
    return proof


library.prove('le_refl', [('a', N)],
    induction('a', lambda x: le(x, x), tt(), lambda _, ih: ih), le(a, a))

# Nested induction is explicit: impossible successor-to-zero branches eliminate
# Empty. Neither transitivity nor cancellation is an ambient arithmetic axiom.
d = var('d')
def trans_goal(x):
    return bind(pi, [('b', N), ('c', N), ('ab', le(x, b)), ('bc', le(b, c))], le(x, c))
def trans_step(t, ih):
    goal_b = lambda y: bind(pi, [('c', N), ('ab', le(S(t), y)), ('bc', le(y, c))], le(S(t), c))
    def step_b(u, _):
        goal_c = lambda z: bind(pi, [('ab', le(S(t), S(u))), ('bc', le(S(u), z))], le(S(t), z))
        return lam('c', N, induction('c', goal_c,
            bind(lam, [('ab', le(S(t), S(u))), ('bc', le(S(u), Z))],
                induct(var('bc'), lam('_absurd', EMPTY, le(S(t), Z)))),
            lambda v, _: bind(lam, [('ab', le(S(t), S(u))), ('bc', le(S(u), S(v)))],
                app(ih, u, v, var('ab'), var('bc')))))
    return lam('b', N, induction('b', goal_b,
        bind(lam, [('c', N), ('ab', le(S(t), Z)), ('bc', le(Z, c))],
            induct(var('ab'), lam('_absurd', EMPTY, le(S(t), c)))), step_b))
library.prove('le_trans', [('a', N), ('b', N), ('c', N), ('ab', le(a, b)), ('bc', le(b, c))],
    app(induction('a', trans_goal,
        bind(lam, [('b', N), ('c', N), ('ab', le(Z, b)), ('bc', le(b, c))], tt()), trans_step),
        b, c, var('ab'), var('bc')), le(a, c))

def weaken_goal(x):
    return bind(pi, [('b', N), ('h', le(x, b))], le(x, S(b)))
library.prove('le_step', [('a', N), ('b', N), ('h', le(a, b))],
    app(induction('a', weaken_goal, bind(lam, [('b', N), ('h', le(Z, b))], tt()),
        lambda t, ih: lam('b', N, induction('b', lambda y: pi('h', le(S(t), y), le(S(t), S(y))),
            lam('h', le(S(t), Z), induct(var('h'), lam('_absurd', EMPTY, le(S(t), ONE)))),
            lambda u, _: lam('h', le(S(t), S(u)), app(ih, u, var('h')))))), b, var('h')),
    le(a, S(b)))

library.prove('add_le_left', [('c', N), ('a', N), ('b', N), ('h', le(a, b))],
    induction('c', lambda x: le(add(x, a), add(x, b)), var('h'), lambda _, ih: ih),
    le(add(c, a), add(c, b)))
library.prove('add_le_right', [('c', N), ('a', N), ('b', N), ('h', le(a, b))],
    change_order(add(c, a), add(c, b), call('add_le_left', c, a, b, var('h')),
        law('add_comm', add(c, a), add(a, c), c, a),
        law('add_comm', add(c, b), add(b, c), c, b)), le(add(a, c), add(b, c)))
library.prove('add_le', [('a', N), ('b', N), ('c', N), ('d', N), ('ab', le(a, b)), ('cd', le(c, d))],
    call('le_trans', add(a, c), add(b, c), add(b, d),
        call('add_le_right', c, a, b, var('ab')), call('add_le_left', b, c, d, var('cd'))),
    le(add(a, c), add(b, d)))
library.prove('add_le_cancel', [('c', N), ('a', N), ('b', N)],
    induction('c', lambda x: pi('h', le(add(x, a), add(x, b)), le(a, b)),
        lam('h', le(a, b), var('h')), lambda _, ih: ih),
    pi('h', le(add(c, a), add(c, b)), le(a, b)))
library.prove('mul_le_left', [('c', N), ('a', N), ('b', N), ('h', le(a, b))],
    induction('c', lambda x: le(mul(x, a), mul(x, b)), tt(),
        lambda t, ih: call('add_le', a, b, mul(t, a), mul(t, b), var('h'), ih)),
    le(mul(c, a), mul(c, b)))
library.prove('mul_le_right', [('c', N), ('a', N), ('b', N), ('h', le(a, b))],
    change_order(mul(c, a), mul(c, b), call('mul_le_left', c, a, b, var('h')),
        law('mul_comm', mul(c, a), mul(a, c), c, a),
        law('mul_comm', mul(c, b), mul(b, c), c, b)), le(mul(a, c), mul(b, c)))
library.prove('mul_le', [('a', N), ('b', N), ('c', N), ('d', N), ('ab', le(a, b)), ('cd', le(c, d))],
    call('le_trans', mul(a, c), mul(b, c), mul(b, d),
        call('mul_le_right', c, a, b, var('ab')), call('mul_le_left', b, c, d, var('cd'))),
    le(mul(a, c), mul(b, d)))

p = var('p')
def cancel_goal(x):
    return pi('b', N, pi('h', le(mul(x, S(p)), mul(b, S(p))), le(x, b)))
library.prove('mul_succ_le_cancel', [('p', N), ('a', N), ('b', N)],
    app(induction('a', cancel_goal,
        bind(lam, [('b', N), ('h', le(Z, mul(b, S(p))))], tt()),
        lambda t, ih: lam('b', N, induction('b',
            lambda y: pi('h', le(mul(S(t), S(p)), mul(y, S(p))), le(S(t), y)),
            lam('h', le(mul(S(t), S(p)), Z), induct(var('h'), lam('_absurd', EMPTY, le(S(t), Z)))),
            lambda u, _: lam('h', le(mul(S(t), S(p)), mul(S(u), S(p))),
                app(ih, u, call('add_le_cancel', S(p), mul(t, S(p)), mul(u, S(p)), var('h'))))))), b),
    pi('h', le(mul(a, S(p)), mul(b, S(p))), le(a, b)))
library.prove('mul_positive_le_cancel', [('p', N), ('a', N), ('b', N), ('positive', le(ONE, p)),
    ('h', le(mul(a, p), mul(b, p)))],
    app(induction('p', lambda x: bind(pi, [('positive', le(ONE, x)), ('h', le(mul(a, x), mul(b, x)))], le(a, b)),
        bind(lam, [('positive', le(ONE, Z)), ('h', le(mul(a, Z), mul(b, Z)))],
            induct(var('positive'), lam('_absurd', EMPTY, le(a, b)))),
        lambda t, _: bind(lam, [('positive', le(ONE, S(t))), ('h', le(mul(a, S(t)), mul(b, S(t))))],
            call('mul_succ_le_cancel', t, a, b, var('h')))), var('positive'), var('h')), le(a, b))
library.prove('mul_positive', [('a', N), ('b', N), ('ha', le(ONE, a)), ('hb', le(ONE, b))],
    call('mul_le', ONE, a, ONE, b, var('ha'), var('hb')), le(ONE, mul(a, b)))


def product_terms(terms):
    if not terms:
        return ONE
    if len(terms) == 1:
        return terms[0]
    return mul(terms[0], product_terms(terms[1:]))


def join_product(left, right):
    l, r = product_terms(left), product_terms(right)
    if not left:
        return same(mul(l, r), add(r, Z)).then(law('add_zero', add(r, Z), r, r))
    if not right:
        return law('mul_one', mul(l, r), l, l)
    if len(left) == 1:
        return same(mul(l, r))
    return law('mul_assoc', mul(l, r), mul(left[0], mul(product_terms(left[1:]), r)),
               left[0], product_terms(left[1:]), r).then(
        join_product(left[1:], right).under(lambda x: mul(left[0], x)))


def sort_product(terms):
    if len(terms) < 2:
        return terms, same(product_terms(terms))
    rest, path = sort_product(terms[1:])
    path = path.under(lambda x: mul(terms[0], x))
    def insert(head, sorted_):
        if not sorted_ or repr(head) <= repr(sorted_[0]):
            return [head] + sorted_, same(product_terms([head] + sorted_))
        next_, tail = sorted_[0], sorted_[1:]
        if not tail:
            return [next_, head], law('mul_comm', mul(head, next_), mul(next_, head), head, next_)
        t = product_terms(tail)
        moved = law('mul_assoc', mul(mul(head, next_), t), mul(head, mul(next_, t)), head, next_, t).reverse().then(
            law('mul_comm', mul(head, next_), mul(next_, head), head, next_).under(lambda x: mul(x, t))).then(
            law('mul_assoc', mul(mul(next_, head), t), mul(next_, mul(head, t)), next_, head, t))
        inserted, after = insert(head, tail)
        return [next_] + inserted, moved.then(after.under(lambda x: mul(next_, x)))
    result, inserted = insert(terms[0], rest)
    return result, path.then(inserted)


def polynomial(term):
    """Return canonical monomials and a kernel-checkable normalization path."""
    if term == Z:
        return [], same(Z)
    if term == ONE:
        return [[]], same(ONE)
    if term[:3] == ('Construct', N, 1):
        tail = term[3][0]
        poly, path = polynomial(add(ONE, tail))
        return poly, same(term, add(ONE, tail)).then(path)
    plus = operation(term, 'nat_add')
    times = operation(term, 'nat_mul')
    if plus is None and times is None:
        return [[term]], same(term)
    x, y = plus if plus is not None else times
    xs, xp = polynomial(x)
    ys, yp = polynomial(y)
    sx = sum_terms([product_terms(t) for t in xs])
    sy = sum_terms([product_terms(t) for t in ys])
    operation_ = add if plus is not None else mul
    path = xp.under(lambda v: operation_(v, y)).then(yp.under(lambda v: operation_(sx, v)))
    if plus is not None:
        terms, combined = additive_normalize(add(sx, sy))
        lookup = {repr(product_terms(t)): t for t in xs + ys}
        return [lookup[repr(t)] for t in terms], path.then(combined)

    def distribute(left, right):
        l = sum_terms([product_terms(t) for t in left])
        r = sum_terms([product_terms(t) for t in right])
        if not left:
            return [], same(mul(l, r), Z)
        if not right:
            return [], law('mul_zero', mul(l, r), Z, l)
        if len(left) > 1:
            h = product_terms(left[0]); t = sum_terms([product_terms(v) for v in left[1:]])
            first, fp = distribute(left[:1], right)
            rest, rp = distribute(left[1:], right)
            expanded = law('mul_add_left', mul(l, r), add(mul(h, r), mul(t, r)), h, t, r).then(
                fp.under(lambda v: add(v, mul(t, r)))).then(
                rp.under(lambda v: add(sum_terms([product_terms(v) for v in first]), v)))
            joined = join_sum([product_terms(v) for v in first], [product_terms(v) for v in rest])
            return first + rest, expanded.then(joined)
        if len(right) > 1:
            h = product_terms(right[0]); t = sum_terms([product_terms(v) for v in right[1:]])
            first, fp = distribute(left, right[:1])
            rest, rp = distribute(left, right[1:])
            expanded = law('mul_add_right', mul(l, r), add(mul(l, h), mul(l, t)), l, h, t).then(
                fp.under(lambda v: add(v, mul(l, t)))).then(
                rp.under(lambda v: add(sum_terms([product_terms(v) for v in first]), v)))
            return first + rest, expanded.then(join_sum([product_terms(v) for v in first], [product_terms(v) for v in rest]))
        joined = join_product(left[0], right[0])
        sorted_, ordered = sort_product(left[0] + right[0])
        return [sorted_], joined.then(ordered)

    expanded, ep = distribute(xs, ys)
    terms, sp = sort_sum([product_terms(t) for t in expanded])
    lookup = {repr(product_terms(t)): t for t in expanded}
    return [lookup[repr(t)] for t in terms], path.then(ep).then(sp)


def ring(left, right):
    if left == right:
        return same(left)
    xs, p = polynomial(left)
    ys, q = polynomial(right)
    if xs != ys:
        raise ValueError('different natural polynomials')
    return p.then(q.reverse())
