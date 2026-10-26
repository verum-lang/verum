#!/usr/bin/env python3
"""Replay general indexed-vector induction and reject changed indices/evidence."""
from pathlib import Path
import certificate_runner
import portable_terms
from portable_terms import (
    RECURSIVE, app, bind, congruence, construct, data, dependent_field,
    identity, indexed_constructor, induct, lam, local, lower, pi,
    recursive_field, refl, universe, var,
)


def cases():
    nat, zero, succ, vector, length = map(var, ('Nat', 'zero', 'succ', 'Vector', 'length'))
    a, n, xs, k, head, tail, ih = map(var, ('A', 'n', 'xs', 'k', 'head', 'tail', 'ih'))
    vec = lambda element, size: app(vector, element, size)
    family = data('Vector', [
        indexed_constructor([], [zero]),
        indexed_constructor([
            dependent_field('n', nat), dependent_field('head', var('Element')),
            recursive_field('tail', n),
        ], [app(succ, n)]),
    ], indices=[('size', nat)])
    length_parameters = [('A', universe()), ('n', nat), ('xs', vec(a, n))]
    branch_parameters = [('k', nat), ('head', a), ('tail', vec(a, k))]
    definitions = [
        ('Nat', data('Nat', [[], [RECURSIVE]])),
        ('zero', construct(nat, 0)),
        ('succ', lam('n', nat, construct(nat, 1, n))),
        ('Vector', lam('Element', universe(), family)),
        ('length', bind(lam, length_parameters, induct(xs,
            bind(lam, [('k', nat), ('ys', vec(a, k))], nat), zero,
            bind(lam, branch_parameters + [('ih', nat)], app(succ, ih))))),
    ]

    def close(term):
        for name, value in reversed(definitions):
            term = local(name, value, term)
        return term

    property_at = lambda size, value: identity(nat, app(length, a, size, value), size)
    motive = bind(lam, [('k', nat), ('ys', vec(a, k))], property_at(k, var('ys')))
    branch = bind(lam, branch_parameters + [('ih', property_at(k, tail))],
        congruence(nat, nat, succ, app(length, a, k, tail), k, ih))
    theorem = bind(lam, length_parameters, induct(xs, motive, refl(zero), branch))
    goal = bind(pi, length_parameters, property_at(n, xs))
    forged_branch = bind(lam, branch_parameters + [('ih', property_at(k, tail))], refl(app(succ, k)))
    forged = bind(lam, length_parameters, induct(xs, motive, refl(zero), forged_branch))
    nil = construct(app(vector, a), 0)
    one = construct(app(vector, a), 1, zero, var('x'), nil)
    inputs = [('A', universe()), ('x', a)]
    missing_index_family = data('MissingIndex', [[]], indices=[('n', nat)])
    malformed_index_family = data('WrongIndex', [indexed_constructor([], [universe()])], indices=[('n', nat)])
    rows = [
        ('general-indexed-length-preservation', theorem, goal, True),
        ('indexed-polymorphic-construction', bind(lam, inputs, one), bind(pi, inputs, vec(a, app(succ, zero))), True),
        ('replace-indexed-induction-hypothesis', forged, goal, False),
        ('forge-constructor-result-index', bind(lam, inputs, one), bind(pi, inputs, vec(a, zero)), False),
        ('wrong-recursive-index', bind(lam, inputs, construct(app(vector, a), 1, zero, var('x'), one)),
            bind(pi, inputs, vec(a, app(succ, zero))), False),
        ('missing-constructor-result-index', missing_index_family, pi('n', nat, universe()), False),
        ('wrong-constructor-result-type', malformed_index_family, pi('n', nat, universe()), False),
    ]
    return [(name, {'term': lower(close(term)), 'claimed_type': lower(close(claim)), 'metadata': {}}, accepted)
            for name, term, claim, accepted in rows]


if __name__ == '__main__':
    certificate_runner.run(cases(), __doc__, [Path(__file__),
        Path(portable_terms.__file__), Path(certificate_runner.__file__)])
