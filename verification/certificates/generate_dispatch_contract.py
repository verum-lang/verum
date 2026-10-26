#!/usr/bin/env python3
"""Untrusted proof producer for the pure owned-dispatch settlement policy.

The generic continuation operations and lawful preflight judgments are explicit
parameters. No certificate asserts Rust refinement, adapter fidelity, affine
usage, durable delivery, or that an independently supplied observation belongs
to a receipt. The result algebra models unwind cleanup, not Rust unwinding.
"""
import argparse
import json
from pathlib import Path

from portable_terms import (
    app, arrow, bind, construct, data, first, identity, induct, j, lam, local,
    lower, pair, pi, refl, sigma, tt, unit, universe, value_field, var, factor,
)

BOOL = data('Bool', [[], []])
FALSE = construct(BOOL, 0)
TRUE = construct(BOOL, 1)
EMPTY = data('Empty', [])


def choose(condition, target, no, yes):
    return induct(condition, lam('choice', BOOL, target), no, yes)


def result_type():
    return data('DispatchResult', [[value_field(var('Observation'))],
        [value_field(var('Error'))], []])


def ok(value): return construct(result_type(), 0, value)
def error(value): return construct(result_type(), 1, value)
def unwind(): return construct(result_type(), 2)
def settlement_type(): return sigma('credited', BOOL, var('Queue'))


def settlement_function(mutation=None):
    q = var('queue')
    kept = pair(FALSE, q)
    stopped = pair(FALSE, app(var('stop'), q))
    advanced = pair(TRUE, app(var('advance'), q))
    delivered = advanced if mutation == 'ignore-interruption' else choose(var('interrupted'), settlement_type(), advanced, stopped)
    if mutation != 'ignore-action':
        delivered = choose(var('unchanged'), settlement_type(), stopped, delivered)
    completed = induct(var('result'), lam('reply', result_type(), settlement_type()),
        lam('observation', var('Observation'), delivered),
        lam('error', var('Error'), advanced if mutation == 'credit-error' else stopped),
        advanced if mutation == 'credit-unwind' else stopped)
    body = completed if mutation == 'ignore-owner' else choose(var('owned'), settlement_type(),
        stopped if mutation == 'stop-foreign' else kept, completed)
    return bind(lam, [('owned', BOOL), ('unchanged', BOOL), ('interrupted', BOOL),
        ('result', result_type())], body)


def confirmed(result):
    return induct(result, lam('confirmation', result_type(), BOOL),
        lam('confirmed_value', var('Observation'), TRUE),
        lam('unconfirmed_error', var('Error'), FALSE), FALSE)


def settled(owned, unchanged, interrupted, result):
    return app(var('settle'), owned, unchanged, interrupted, result)


def no_false_true(path, target):
    # Transport the inhabitant of Unit along false=true into Empty, then
    # eliminate Empty. No axioms, admitted branches or Boolean host oracle.
    family = lam('discriminate', BOOL, choose(var('discriminate'), universe(), unit(), EMPTY))
    impossible = j(family, tt(), path)
    return induct(impossible, lam('impossible', EMPTY, target))


def authenticity_target(result, interrupted):
    return sigma('owner_identity', identity(var('Token'), var('selected'), var('current')),
        sigma('action_identity', identity(var('Action'), var('proposed'), var('actual')),
            sigma('not_interrupted', identity(BOOL, interrupted, FALSE),
                sigma('observed', var('Observation'), identity(result_type(), result, ok(var('observed')))))))


def authenticity_premises(owned, unchanged, interrupted, result):
    return [
        ('ownerLaw', arrow(identity(BOOL, owned, TRUE), identity(var('Token'), var('selected'), var('current')))),
        ('actionLaw', arrow(identity(BOOL, unchanged, TRUE), identity(var('Action'), var('proposed'), var('actual')))),
        ('credit', identity(BOOL, first(settled(owned, unchanged, interrupted, result)), TRUE)),
    ]


def authenticity_goal(owned, unchanged, interrupted, result):
    return bind(pi, authenticity_premises(owned, unchanged, interrupted, result), authenticity_target(result, interrupted))


def authenticity_leaf(owned, unchanged, interrupted, result, observed=None):
    if observed is not None:
        body = pair(app(var('ownerLaw'), refl(TRUE)),
            pair(app(var('actionLaw'), refl(TRUE)), pair(refl(FALSE), pair(observed, refl(result)))))
    else:
        body = no_false_true(var('credit'), authenticity_target(result, interrupted))
    return bind(lam, authenticity_premises(owned, unchanged, interrupted, result), body)


def authenticity_proof(owned, unchanged, interrupted, result):
    observed = var('observed_value')
    successful_result = ok(observed)
    interrupted_split = induct(interrupted,
        lam('interruption_case', BOOL, authenticity_goal(TRUE, TRUE, var('interruption_case'), successful_result)),
        authenticity_leaf(TRUE, TRUE, FALSE, successful_result, observed),
        authenticity_leaf(TRUE, TRUE, TRUE, successful_result))
    action_split = induct(unchanged,
        lam('action_case', BOOL, authenticity_goal(TRUE, var('action_case'), interrupted, successful_result)),
        authenticity_leaf(TRUE, FALSE, interrupted, successful_result), interrupted_split)
    result_split = induct(result,
        lam('result_case', result_type(), authenticity_goal(TRUE, unchanged, interrupted, var('result_case'))),
        lam('observed_value', var('Observation'), action_split),
        lam('error_value', var('Error'), authenticity_leaf(TRUE, unchanged, interrupted, error(var('error_value')))),
        authenticity_leaf(TRUE, unchanged, interrupted, unwind()))
    return induct(owned,
        lam('owner_case', BOOL, authenticity_goal(var('owner_case'), unchanged, interrupted, result)),
        authenticity_leaf(FALSE, unchanged, interrupted, result), result_split)


def parameters():
    return [(name, universe()) for name in ('Queue', 'Action', 'Token', 'Observation', 'Error')] + [
        ('queue', var('Queue')), ('stop', arrow(var('Queue'), var('Queue'))),
        ('advance', arrow(var('Queue'), var('Queue'))),
        ('proposed', var('Action')), ('actual', var('Action')),
        ('selected', var('Token')), ('current', var('Token')),
    ]


def certificate(kind='credit_requires_action_owner_and_result', mutation=None):
    o, u, i, r = (var(name) for name in ('owned', 'unchanged', 'interrupted', 'result'))
    tail = [('owned', BOOL), ('unchanged', BOOL), ('interrupted', BOOL), ('result', result_type())]
    if kind == 'credit_requires_action_owner_and_result':
        goal = authenticity_goal(o, u, i, r)
        proof = authenticity_proof(o, u, i, r)
    elif kind == 'foreign_continuation_is_unchanged':
        expected = pair(FALSE, var('queue'))
        goal = identity(settlement_type(), settled(FALSE, u, i, r), expected)
        proof = refl(expected)
    elif kind in ('error_closes_owned_without_credit', 'unwind_closes_owned_without_credit'):
        if kind.startswith('error'):
            tail.append(('failure', var('Error')))
            result = error(var('failure'))
        else:
            result = unwind()
        expected = pair(FALSE, app(var('stop'), var('queue')))
        goal = identity(settlement_type(), settled(TRUE, u, i, result), expected)
        proof = refl(expected)
    elif kind == 'interrupted_ok_confirms_transport_without_credit':
        tail.append(('observation', var('Observation')))
        result = ok(var('observation'))
        actual = pair(first(settled(TRUE, TRUE, TRUE, result)),
            FALSE if mutation == 'confirmation-from-credit' else confirmed(result))
        expected = pair(FALSE, TRUE)
        goal = identity(sigma('credit', BOOL, BOOL), actual, expected)
        proof = refl(expected)
    elif kind == 'error_never_confirms_transport':
        tail.append(('failure', var('Error')))
        goal = identity(BOOL, confirmed(error(var('failure'))), FALSE)
        proof = refl(FALSE)
    elif kind == 'repeated_credit_requires_revision_invalidation':
        # Countermodel: reusing the old admission fact after advancing permits
        # a second credit. Move semantics/fresh revisions are bridge premises.
        tail.append(('observation', var('Observation')))
        goal = identity(BOOL, first(settled(TRUE, TRUE, FALSE, ok(var('observation')))), FALSE)
        proof = refl(FALSE)
    elif kind == 'owned_success_advances_exact_continuation':
        tail.append(('observation', var('Observation')))
        expected = pair(TRUE, app(var('advance'), var('queue')))
        goal = identity(settlement_type(), settled(TRUE, TRUE, FALSE, ok(var('observation'))), expected)
        proof = refl(expected)
    else:
        raise ValueError(kind)
    definition = settlement_function(mutation)
    statement = local('settle', definition, bind(pi, tail, goal))
    proof = local('settle', definition, bind(lam, tail, proof))
    aliases = [('Boolean', BOOL), ('Empty', EMPTY), ('Unit', unit()),
        ('Reply', result_type()), ('Settlement', settlement_type())]
    proof, statement = factor(proof, aliases), factor(statement, aliases)
    for index in reversed(range(len(aliases))):
        name, value = aliases[index]
        value = factor(value, aliases[:index])
        proof, statement = local(name, value, proof), local(name, value, statement)
    statement = bind(pi, parameters(), statement)
    proof = bind(lam, parameters(), proof)
    return {'term': lower(proof), 'claimed_type': lower(statement), 'metadata': {
        'theorem': kind, 'scope': 'pure synchronous continuation settlement; lawful full-action and token preflight are explicit premises'}}


def cases():
    kinds = ['credit_requires_action_owner_and_result', 'foreign_continuation_is_unchanged',
        'error_closes_owned_without_credit', 'unwind_closes_owned_without_credit',
        'owned_success_advances_exact_continuation', 'interrupted_ok_confirms_transport_without_credit', 'error_never_confirms_transport']
    result = [(kind, certificate(kind), True) for kind in kinds]
    for mutation in ('ignore-owner', 'ignore-action', 'ignore-interruption', 'credit-error', 'credit-unwind'):
        result.append((mutation, certificate(mutation=mutation), False))
    result.append(('stop-foreign', certificate('foreign_continuation_is_unchanged', 'stop-foreign'), False))
    result.append(('confirmation-from-credit', certificate('interrupted_ok_confirms_transport_without_credit', 'confirmation-from-credit'), False))
    result.append(('repeated-credit-without-revision-change', certificate('repeated_credit_requires_revision_invalidation'), False))
    return result


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    encoded = json.dumps(certificate(), separators=(',', ':')) + '\n'
    if args.output: args.output.write_text(encoded)
    else: print(encoded, end='')
