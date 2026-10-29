"""Portable structural laws for reserved evidence and ordered buffering.

The immutable declaration carrier contains the owner's source, origin, end,
signature, family and criterion. Exact rational arithmetic and computed source/
order/capacity guards are separate Lean extensions, not native term claims.
No runtime, statistical, allocation or single-use/affine refinement is implied.
"""
from portable_terms import (RECURSIVE, app, arrow, bind, congruence, construct,
    data, first, identity, induct, lam, local, lower, pair, pi, refl, second,
    sigma, universe, value_field, var)

N = data('Nat', [[], [RECURSIVE]])
successor = lambda n: construct(N, 1, n)
D, R, G, C = map(var, ('D', 'R', 'G', 'C'))
product = lambda a, b: sigma('_', a, b)
reservation = product(N, D)
rows = data('List', [[], [value_field(R), RECURSIVE]])
nil = construct(rows, 0)
cons = lambda x, xs: construct(rows, 1, x, xs)
buffer = product(reservation, rows)
option = lambda a: data('Option', [[], [value_field(a)]])
none = lambda a: construct(option(a), 0)
some = lambda a, x: construct(option(a), 1, x)
result = data('Except', [[value_field(buffer)], [value_field(buffer)]])
rejected = lambda b: construct(result, 0, b)
accepted = lambda b: construct(result, 1, b)
boolean = data('Bool', [[], []])
false, true = construct(boolean, 0), construct(boolean, 1)
definitions, contracts = [], []


def close(term):
    for name, definition in reversed(definitions):
        term = local(name, definition, term)
    return term


def certificate(term, claim):
    return dict(term=lower(close(term)), claimed_type=lower(close(claim)), metadata={})


def add(name, parameters, proof, claim):
    contracts.append((name, certificate(bind(lam, parameters, proof), bind(pi, parameters, claim)), True))


types = [('D', universe()), ('R', universe())]
n, d, ticket, outcome, grammar = map(var, ('next', 'declaration', 'reservation', 'outcome', 'grammar'))
reserve = pair(successor(n), pair(n, d))
complete = lambda next_, ticket_, outcome_: pair(next_, induct(outcome_,
    lam('result', option(G), option(product(reservation, G))), none(product(reservation, G)),
    lam('grammar', G, some(product(reservation, G), pair(ticket_, grammar)))))
reserved_params = [('D', universe()), ('next', N), ('declaration', D)]
add('reserved_ordinal', reserved_params, refl(n), identity(N, first(second(reserve)), n))
add('reserved_declaration', reserved_params, refl(d), identity(D, second(second(reserve)), d))
add('reservation_advance', reserved_params, refl(successor(n)), identity(N, first(reserve), successor(n)))
completion_params = [('D', universe()), ('G', universe()), ('next', N), ('reservation', reservation)]
add('completion_keeps_ledger', completion_params+[('outcome', option(G))], refl(n),
    identity(N, first(complete(n, ticket, outcome)), n))
add('refusal_cannot_refund', [('G', universe())]+reserved_params, refl(successor(n)),
    identity(N, first(complete(first(reserve), second(reserve), none(G))), successor(n)))
add('materialization_keeps_reservation', completion_params+[('grammar', G)],
    refl(some(product(reservation, G), pair(ticket, grammar))),
    identity(option(product(reservation, G)), second(complete(n, ticket, some(G, grammar))),
        some(product(reservation, G), pair(ticket, grammar))))

xs, ys, zs, x, tail, ih = map(var, ('xs', 'ys', 'zs', 'x', 'tail', 'ih'))
definitions.append(('append', bind(lam, [('R', universe()), ('xs', rows)],
    induct(xs, lam('list', rows, arrow(rows, rows)), lam('ys', rows, ys),
        bind(lam, [('x', R), ('tail', rows), ('ih', arrow(rows, rows)), ('ys', rows)],
            cons(x, app(ih, ys)))))))
append = lambda a, b: app(var('append'), R, a, b)
b = var('buffer')
batch = lambda b, additions: pair(first(b), append(second(b), additions))
params = types+[('buffer', buffer)]
add('pause_preserves_prefix', params, refl(b), identity(buffer, b, b))
add('append_keeps_reservation', params+[('xs', rows)], refl(first(b)),
    identity(reservation, first(batch(b, xs)), first(b)))
add('append_exact_prefix', params+[('xs', rows)], refl(append(second(b), xs)),
    identity(rows, second(batch(b, xs)), append(second(b), xs)))
assoc = lambda a: identity(rows, append(append(a, ys), zs), append(a, append(ys, zs)))
assoc_proof = induct(second(b), lam('xs', rows, assoc(xs)), refl(append(ys, zs)),
    bind(lam, [('x', R), ('tail', rows), ('ih', assoc(tail))],
        congruence(rows, rows, lam('xs', rows, cons(x, xs)),
            append(append(tail, ys), zs), append(tail, append(ys, zs)), ih)))
add('append_concatenation', params+[('ys', rows), ('zs', rows)],
    congruence(rows, buffer, lam('xs', rows, pair(first(b), xs)),
        append(append(second(b), ys), zs), append(second(b), append(ys, zs)), assoc_proof),
    identity(buffer, batch(batch(b, ys), zs), batch(b, append(ys, zs))))
admit = lambda allowed: induct(allowed, lam('allowed', boolean, result), rejected(b), accepted(batch(b, xs)))
add('refused_keeps_owner_and_prefix', params+[('xs', rows)], refl(rejected(b)),
    identity(result, admit(false), rejected(b)))
transfer = pair(first(b), pair(grammar, second(b)))
add('transfer_exact_prefix', params+[('G', universe()), ('grammar', G)], refl(second(b)),
    identity(rows, second(second(transfer)), second(b)))
selection, end = var('selected'), var('end')
validation = pair(selection, pair(end, nil))
add('validation_starts_empty', [('C', universe()), ('R', universe()), ('selected', C), ('end', N)],
    refl(nil), identity(rows, second(second(validation)), nil))

# Allocation order is independent of completion order. Addition is structural,
# and cancellation is proved by induction, not assumed as an arithmetic axiom.
start, i, k, same = map(var, ('start', 'i', 'k', 'same'))
definitions.append(('plus', lam('start', N, induct(start,
    lam('start', N, arrow(N, N)), lam('i', N, i),
    bind(lam, [('tail', N), ('ih', arrow(N, N)), ('i', N)], successor(app(ih, i)))))))
plus = lambda a, b: app(var('plus'), a, b)
pred = lam('number', N, induct(var('number'), lam('number', N, N), construct(N, 0),
    bind(lam, [('tail', N), ('ih', N)], tail)))
cancel_params = lambda n: [('i', N), ('k', N), ('same', identity(N, plus(n, i), plus(n, k)))]
cancel_goal = lambda n: bind(pi, cancel_params(n), identity(N, i, k))
cancel_proof = app(induct(start, lam('start', N, cancel_goal(start)),
    bind(lam, cancel_params(construct(N, 0)), same),
    bind(lam, [('tail', N), ('ih', cancel_goal(tail))]+cancel_params(successor(tail)),
        app(ih, i, k, congruence(N, N, pred, successor(plus(tail, i)), successor(plus(tail, k)), same)))),
    i, k, same)
add('reservation_indices_injective', [('start', N)]+cancel_params(start), cancel_proof, identity(N, i, k))

# Strict structural order: zero is below every successor; a successor is never
# below zero; comparing successors strips one constructor on each side.
empty = data('Empty', [])
unit = data('Unit', [[]])
definitions.append(('less', lam('i', N, induct(i, lam('i', N, arrow(N, universe())),
    lam('k', N, induct(k, lam('k', N, universe()), empty,
        bind(lam, [('tail', N), ('ih', universe())], unit))),
    bind(lam, [('tail', N), ('ih', arrow(N, universe())), ('k', N)],
        induct(k, lam('k', N, universe()), empty,
            bind(lam, [('right', N), ('unused', universe())], app(ih, var('right')))))))))
less = lambda a, b: app(var('less'), a, b)
h = var('before')
strict_goal = lambda n: less(plus(n, i), plus(n, k))
strict_proof = induct(start, lam('start', N, strict_goal(start)), h,
    bind(lam, [('tail', N), ('ih', strict_goal(tail))], ih))
add('reservation_indices_strict', [('start', N), ('i', N), ('k', N), ('before', less(i,k))],
    strict_proof, strict_goal(start))


def cases():
    cases = list(contracts)
    def reject(name, parameters, left, right, carrier):
        cases.append((name, certificate(bind(lam, parameters, refl(left)),
            bind(pi, parameters, identity(carrier, left, right))), False))
    reject('refusal_refunds_ordinal', reserved_params, first(reserve), n, N)
    reject('completion_reprices_ordinal', completion_params+[('grammar', G)],
        second(complete(n, ticket, some(G, grammar))), some(product(reservation,G), pair(pair(n, second(ticket)), grammar)),
        option(product(reservation,G)))
    reject('replacement_declaration', reserved_params+[('other', D)], second(second(reserve)), var('other'), D)
    reject('buffered_rows_erased', params+[('xs', rows)], second(batch(b, xs)), xs, rows)
    reject('buffer_order_reversed', params+[('xs', rows)], append(second(b), xs), append(xs, second(b)), rows)
    reject('refusal_changes_owner', params+[('xs', rows), ('other', buffer)], admit(false), rejected(var('other')), result)
    reject('training_reused_as_validation', [('C', universe()), ('R', universe()), ('selected', C), ('end', N), ('xs', rows)],
        second(second(validation)), xs, rows)
    return cases
