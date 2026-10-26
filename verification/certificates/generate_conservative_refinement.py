#!/usr/bin/env python3
"""Construct a closed finite-execution preservation certificate.

This is an untrusted proof producer. The emitted term and statement must both
be checked by the native kernels and the independent Lean replay. No metadata,
finite execution sample, or theorem name authorizes a typing rule.
"""
import argparse
import json
from pathlib import Path


def var(name): return ('Var', name)
def universe(): return ('Universe',)
def pi(name, domain, body): return ('Pi', name, domain, body)
def lam(name, domain, body): return ('Lam', name, domain, body)
def local(name, value, body): return ('Let', name, value, body)
def app(function, *arguments):
    for argument in arguments: function = ('App', function, argument)
    return function

def bind(kind, parameters, body):
    for name, domain in reversed(parameters): body = kind(name, domain, body)
    return body

def identity(carrier, left, right): return ('Id', carrier, left, right)
def refl(value): return ('Refl', value)
def j(motive, base, path): return ('J', motive, base, path)
def pair(left, right): return ('Pair', left, right)
def first(value): return ('Fst', value)
def second(value): return ('Snd', value)
def sigma(name, domain, body): return ('Sigma', name, domain, body)
def data(name, constructors): return ('Data', name, constructors)
def value_field(term): return ('Value', term)
RECURSIVE = ('Recursive',)
def construct(declaration, constructor, *arguments): return ('Construct', declaration, constructor, arguments)
def induct(scrutinee, motive, *cases): return ('Induct', scrutinee, motive, cases)
def list_type(element): return data('List', [[], [value_field(element), RECURSIVE]])
def nil(element): return construct(list_type(element), 0)
def cons(element, head, tail): return construct(list_type(element), 1, head, tail)
def singleton(element, item): return cons(element, item, nil(element))
def outcome(state):
    return data('Outcome', [[value_field(state)], [value_field(state), value_field(var('Terminal'))], [value_field(var('Reason'))]])
def next_state(state, value): return construct(outcome(state), 0, value)
def stop(state, value, terminal): return construct(outcome(state), 1, value, terminal)
def refuse(state, reason): return construct(outcome(state), 2, reason)
def execution(state): return sigma('_states', list_type(state), outcome(state))
def unit(): return data('Unit', [[]])
def tt(): return construct(unit(), 0)


def arrow(domain, codomain): return pi('_', domain, codomain)
def step_type(state): return arrow(state, arrow(var('Action'), outcome(state)))
def prepend(state, head, tail): return pair(cons(state, head, first(tail)), second(tail))


def symm(carrier, left, right, path):
    return j(lam('sym_endpoint', carrier, identity(carrier, var('sym_endpoint'), left)), refl(left), path)


def congruence(source, target, function, left, right, path):
    return j(lam('cong_endpoint', source, identity(target, app(function, left), app(function, var('cong_endpoint')))), refl(app(function, left)), path)


def transitive(carrier, left, middle, right, first_path, second_path):
    return j(lam('trans_endpoint', carrier, identity(carrier, left, var('trans_endpoint'))), first_path, second_path)


def list_map(source, target, function, items):
    return induct(items, lam('map_subject', list_type(source), list_type(target)), nil(target),
        bind(lam, [('map_head', source), ('map_tail', list_type(source)), ('map_ih', list_type(target))],
             cons(target, app(function, var('map_head')), var('map_ih'))))


def outcome_map(project, result):
    r, s = var('R'), var('S')
    return induct(result, lam('outcome_subject', outcome(r), outcome(s)),
        lam('out_next', r, next_state(s, app(project, var('out_next')))),
        bind(lam, [('out_stop', r), ('terminal', var('Terminal'))], stop(s, app(project, var('out_stop')), var('terminal'))),
        lam('out_refuse', var('Reason'), refuse(s, var('out_refuse'))))


def execution_map(result):
    return pair(app(var('mapStates'), first(result)), app(var('mapOutcome'), second(result)))


def run_from_outcome(state, head, tail_run, result):
    return induct(result, lam('run_outcome', outcome(state), execution(state)),
        lam('successor', state, prepend(state, head, app(tail_run, var('successor')))),
        bind(lam, [('terminal_state', state), ('terminal_payload', var('Terminal'))],
            pair(cons(state, head, singleton(state, var('terminal_state'))), stop(state, var('terminal_state'), var('terminal_payload')))),
        lam('refusal_payload', var('Reason'), pair(singleton(state, head), refuse(state, var('refusal_payload')))))


def run_function(state, step):
    action = var('Action')
    return lam('program', list_type(action), induct(var('program'),
        lam('run_program', list_type(action), arrow(state, execution(state))),
        lam('initial', state, pair(singleton(state, var('initial')), next_state(state, var('initial')))),
        bind(lam, [('action', action), ('tail', list_type(action)), ('tailRun', arrow(state, execution(state))), ('initial', state)],
            run_from_outcome(state, var('initial'), var('tailRun'), app(step, var('initial'), var('action'))))))


def remaining(tail, result):
    r = var('R')
    return induct(result, lam('remaining_outcome', outcome(r), universe()),
        lam('remaining_successor', r, app(var('admissible'), tail, var('remaining_successor'))),
        bind(lam, [('remaining_state', r), ('remaining_terminal', var('Terminal'))], unit()),
        lam('remaining_reason', var('Reason'), unit()))


def admissible_function():
    r, action = var('R'), var('Action')
    remainder = induct(app(var('liftedStep'), var('initial'), var('action')),
        lam('allowed_outcome', outcome(r), universe()),
        lam('allowed_successor', r, app(var('tailAllowed'), var('allowed_successor'))),
        bind(lam, [('allowed_state', r), ('allowed_terminal', var('Terminal'))], unit()),
        lam('allowed_reason', var('Reason'), unit()))
    return lam('program', list_type(action), induct(var('program'),
        lam('allowed_program', list_type(action), arrow(r, universe())),
        lam('initial', r, unit()),
        bind(lam, [('action', action), ('tail', list_type(action)), ('tailAllowed', arrow(r, universe())), ('initial', r)],
            sigma('head_admissible', app(var('allowed'), var('initial'), var('action')), remainder))))


def trace_goal(program, initial):
    return identity(execution(var('S')), app(var('runS'), program, app(var('project'), initial)),
                    app(var('mapExecution'), app(var('runR'), program, initial)))


def trace_statement(program):
    return pi('initial', var('R'), pi('evidence', app(var('admissible'), program, var('initial')), trace_goal(program, var('initial'))))


def trace_proof():
    s, r, action = var('S'), var('R'), var('Action')
    program = cons(action, var('action'), var('tail'))
    initial, tail = var('initial'), var('tail')
    projected = app(var('project'), initial)
    left_step = app(var('stepS'), projected, var('action'))
    right_step = app(var('liftedStep'), initial, var('action'))
    tail_s = app(var('runS'), tail)
    tail_r = app(var('runR'), tail)
    projected_outcome = app(var('mapOutcome'), var('matched'))
    left_result = run_from_outcome(s, projected, tail_s, left_step)
    right_result = app(var('mapExecution'), run_from_outcome(r, initial, tail_r, var('matched')))
    matched_goal = identity(execution(s), left_result, right_result)
    matched_motive = lam('matched', outcome(r),
        pi('head_commutes', identity(outcome(s), left_step, projected_outcome),
           pi('tail_evidence', remaining(tail, var('matched')), matched_goal)))

    def outcome_case(fields, matched, base_proof):
        mapped = app(var('mapOutcome'), matched)
        rhs = app(var('mapExecution'), run_from_outcome(r, initial, tail_r, matched))
        motive = lam('transported_outcome', outcome(s), identity(execution(s),
            run_from_outcome(s, projected, tail_s, var('transported_outcome')), rhs))
        transported = j(motive, base_proof, symm(outcome(s), left_step, mapped, var('head_commutes')))
        return bind(lam, fields + [
            ('head_commutes', identity(outcome(s), left_step, mapped)),
            ('tail_evidence', remaining(tail, matched)),
        ], transported)

    successor = var('successor')
    tail_left = app(var('runS'), tail, app(var('project'), successor))
    tail_right = app(var('mapExecution'), app(var('runR'), tail, successor))
    next_base = congruence(execution(s), execution(s),
        lam('prepend_tail', execution(s), prepend(s, projected, var('prepend_tail'))),
        tail_left, tail_right, app(var('traceIH'), successor, var('tail_evidence')))
    next_case = outcome_case([('successor', r)], next_state(r, successor), next_base)
    stop_value = stop(r, var('final_state'), var('terminal_value'))
    stop_base = refl(run_from_outcome(s, projected, tail_s, app(var('mapOutcome'), stop_value)))
    stop_case = outcome_case([('final_state', r), ('terminal_value', var('Terminal'))], stop_value, stop_base)
    refuse_value = refuse(r, var('reason_value'))
    refuse_base = refl(run_from_outcome(s, projected, tail_s, app(var('mapOutcome'), refuse_value)))
    refuse_case = outcome_case([('reason_value', var('Reason'))], refuse_value, refuse_base)
    step_proof = app(induct(right_step, matched_motive, next_case, stop_case, refuse_case),
        app(var('commutes'), initial, var('action'), first(var('evidence'))), second(var('evidence')))
    nil_proof = bind(lam, [('initial', r), ('evidence', app(var('admissible'), nil(action), var('initial')))],
        refl(app(var('runS'), nil(action), projected)))
    cons_proof = bind(lam, [('action', action), ('tail', list_type(action)), ('traceIH', trace_statement(tail)),
                          ('initial', r), ('evidence', app(var('admissible'), program, initial))], step_proof)
    return lam('program', list_type(action), induct(var('program'), lam('induction_program', list_type(action), trace_statement(var('induction_program'))), nil_proof, cons_proof))


def observed(state, observer, result):
    return list_map(state, var('Observation'), observer, first(result))


def observations_statement(program):
    initial = var('initial')
    left = app(var('observeStatesS'), first(app(var('runS'), program, app(var('project'), initial))))
    right = app(var('observeStatesR'), first(app(var('runR'), program, initial)))
    return pi('initial', var('R'), pi('evidence', app(var('admissible'), program, initial), identity(list_type(var('Observation')), left, right)))


def observation_map_law():
    r, obs = var('R'), var('Observation')
    def left(items): return app(var('observeStatesS'), app(var('mapStates'), items))
    def right(items): return app(var('observeStatesR'), items)
    head, tail = var('observed_head'), var('observed_tail')
    lhs_head = app(var('observeS'), app(var('project'), head))
    rhs_head = app(var('observeR'), head)
    lhs_tail, rhs_tail = left(tail), right(tail)
    lhs = cons(obs, lhs_head, lhs_tail)
    middle = cons(obs, rhs_head, lhs_tail)
    rhs = cons(obs, rhs_head, rhs_tail)
    head_path = congruence(obs, list_type(obs), lam('head_value', obs, cons(obs, var('head_value'), lhs_tail)),
        lhs_head, rhs_head, app(var('observationsCommute'), head))
    tail_path = congruence(list_type(obs), list_type(obs), lam('tail_value', list_type(obs), cons(obs, rhs_head, var('tail_value'))),
        lhs_tail, rhs_tail, var('observationsIH'))
    cons_proof = bind(lam, [('observed_head', r), ('observed_tail', list_type(r)),
        ('observationsIH', identity(list_type(obs), lhs_tail, rhs_tail))],
        transitive(list_type(obs), lhs, middle, rhs, head_path, tail_path))
    return lam('observed_states', list_type(r), induct(var('observed_states'),
        lam('states_subject', list_type(r), identity(list_type(obs), left(var('states_subject')), right(var('states_subject')))),
        refl(nil(obs)), cons_proof))


def observations_proof():
    s, obs, program, initial = var('S'), var('Observation'), var('program'), var('initial')
    source_execution = app(var('runS'), program, app(var('project'), initial))
    refined_execution = app(var('runR'), program, initial)
    mapped_execution = app(var('mapExecution'), refined_execution)
    left = app(var('observeStatesS'), first(source_execution))
    middle = app(var('observeStatesS'), app(var('mapStates'), first(refined_execution)))
    right = app(var('observeStatesR'), first(refined_execution))
    trace_path = congruence(execution(s), list_type(obs),
        lam('execution_subject', execution(s), app(var('observeStatesS'), first(var('execution_subject')))),
        source_execution, mapped_execution, app(var('traceProof'), program, initial, var('evidence')))
    map_path = app(var('observationMapLaw'), first(refined_execution))
    return bind(lam, [('program', list_type(var('Action'))), ('initial', var('R')),
        ('evidence', app(var('admissible'), program, initial))],
        transitive(list_type(obs), left, middle, right, trace_path, map_path))


def lower(term, context=()):
    kind = term[0]
    if kind == 'Var':
        if term[1] not in context: raise ValueError(f'unbound generator variable: {term[1]}')
        return {'Var': context.index(term[1])}
    if kind == 'Universe': return {'Universe': {'Concrete': 0}}
    if kind == 'Let':
        return {'Let': {'value': lower(term[2], context), 'body': lower(term[3], (term[1],) + context)}}
    if kind in ('Pi', 'Lam', 'Sigma'):
        _, name, domain, body = term
        return {kind: [lower(domain, context), lower(body, (name,) + context)]}
    if kind in ('App', 'Pair'): return {kind: [lower(term[1], context), lower(term[2], context)]}
    if kind in ('Refl', 'Fst', 'Snd'): return {kind: lower(term[1], context)}
    if kind == 'Id': return {'Id': dict(zip(('ty', 'lhs', 'rhs'), (lower(x, context) for x in term[1:])))}
    if kind == 'J': return {'J': dict(zip(('motive', 'base', 'scrutinee'), (lower(x, context) for x in term[1:])))}
    if kind == 'Data':
        return {'Data': {'name': term[1], 'level': {'Concrete': 0}, 'constructors': [
            [('Recursive' if field[0] == 'Recursive' else {'Value': lower(field[1], context)}) for field in fields]
            for fields in term[2]]}}
    if kind == 'Construct': return {'Construct': {'data': lower(term[1], context), 'constructor': term[2], 'args': [lower(x, context) for x in term[3]]}}
    if kind == 'Induct': return {'Induct': {'scrutinee': lower(term[1], context), 'motive': lower(term[2], context), 'cases': [lower(x, context) for x in term[3]]}}
    raise ValueError(kind)


def mutate_proof(proof, mutation):
    """Alter proof-bearing code only; keep the independently generated goal."""
    count = 0
    def rewrite(node, target, replacement):
        nonlocal count
        if node == target:
            count += 1
            return replacement
        if isinstance(node, tuple): return tuple(rewrite(x, target, replacement) for x in node)
        if isinstance(node, list): return [rewrite(x, target, replacement) for x in node]
        return node
    def definition(node, name, target, replacement):
        if isinstance(node, tuple) and node and node[0] == 'Let' and node[1] == name:
            return ('Let', node[1], rewrite(node[2], target, replacement), node[3])
        if isinstance(node, tuple): return tuple(definition(x, name, target, replacement) for x in node)
        if isinstance(node, list): return [definition(x, name, target, replacement) for x in node]
        return node
    if mutation == 'continue-state':
        proof = rewrite(proof, app(var('traceIH'), var('successor'), var('tail_evidence')),
                        app(var('traceIH'), var('initial'), var('tail_evidence')))
    elif mutation == 'stop-outcome':
        proof = definition(proof, 'runR', stop(var('R'), var('terminal_state'), var('terminal_payload')),
                           next_state(var('R'), var('terminal_state')))
    elif mutation == 'refusal-outcome':
        proof = definition(proof, 'runR', refuse(var('R'), var('refusal_payload')), next_state(var('R'), var('initial')))
    elif mutation == 'tail-admissibility':
        proof = definition(proof, 'admissible', app(var('tailAllowed'), var('allowed_successor')), unit())
    elif mutation == 'observation-map':
        proof = rewrite(proof, app(var('observationsCommute'), var('observed_head')),
                        refl(app(var('observeS'), app(var('project'), var('observed_head')))))
    else:
        raise ValueError('unknown mutation: '+mutation)
    if count != 1: raise ValueError(f'{mutation}: expected exactly one causal change, got {count}')
    return proof


def certificate(observations=False, mutation=None):
    s, r, a, b = (var(x) for x in ('S', 'R', 'Action', 'RefinedAction'))
    parameters = [(name, universe()) for name in ('S', 'R', 'Action', 'RefinedAction', 'Terminal', 'Reason')]
    parameters += [('project', arrow(r, s)), ('liftAction', arrow(r, arrow(a, b))),
        ('stepS', step_type(s)), ('stepR', arrow(r, arrow(b, outcome(r)))), ('allowed', arrow(r, arrow(a, universe())))]
    if observations:
        parameters += [('Observation', universe()), ('observeS', arrow(s, var('Observation'))),
            ('observeR', arrow(r, var('Observation'))),
            ('observationsCommute', pi('observed_r', r, identity(var('Observation'),
                app(var('observeS'), app(var('project'), var('observed_r'))), app(var('observeR'), var('observed_r')))))]
    definitions = [
        ('liftedStep', step_type(r), bind(lam, [('r', r), ('a', a)], app(var('stepR'), var('r'), app(var('liftAction'), var('r'), var('a'))))),
        ('mapOutcome', arrow(outcome(r), outcome(s)), lam('result', outcome(r), outcome_map(var('project'), var('result')))),
        ('mapStates', arrow(list_type(r), list_type(s)), lam('states', list_type(r), list_map(r, s, var('project'), var('states')))),
        ('mapExecution', arrow(execution(r), execution(s)), lam('result', execution(r), execution_map(var('result')))),
        ('runS', arrow(list_type(a), arrow(s, execution(s))), run_function(s, var('stepS'))),
        ('runR', arrow(list_type(a), arrow(r, execution(r))), run_function(r, var('liftedStep'))),
        ('admissible', arrow(list_type(a), arrow(r, universe())), admissible_function()),
    ]
    commutation = bind(pi, [('r', r), ('a', a), ('admissible_step', app(var('allowed'), var('r'), var('a')))],
        identity(outcome(s), app(var('stepS'), app(var('project'), var('r')), var('a')),
                 app(var('mapOutcome'), app(var('liftedStep'), var('r'), var('a')))))
    if observations:
        definitions += [
            ('observeStatesS', arrow(list_type(s), list_type(var('Observation'))), lam('states_to_observe', list_type(s), list_map(s, var('Observation'), var('observeS'), var('states_to_observe')))),
            ('observeStatesR', arrow(list_type(r), list_type(var('Observation'))), lam('states_to_observe', list_type(r), list_map(r, var('Observation'), var('observeR'), var('states_to_observe')))),
        ]
        statement = pi('commutes', commutation, pi('program', list_type(a), observations_statement(var('program'))))
        proof = lam('commutes', commutation,
            local('traceProof', trace_proof(), local('observationMapLaw', observation_map_law(), observations_proof())))
    else:
        statement = pi('commutes', commutation, pi('program', list_type(a), trace_statement(var('program'))))
        proof = lam('commutes', commutation, trace_proof())
    # Local definitions are transparent checked terms, not assumed function
    # parameters. Share datatype signatures without changing their identity.
    aliases = [(name, value) for name, value in [
        ('ListS', list_type(s)), ('ListR', list_type(r)), ('ListAction', list_type(a)),
        ('OutcomeS', outcome(s)), ('OutcomeR', outcome(r)),
        ('ExecutionS', execution(s)), ('ExecutionR', execution(r)), ('Unit', unit()),
    ]]
    if observations: aliases.append(('ListObservation', list_type(var('Observation'))))
    def factor(value, available):
        for name, expression in available:
            if value == expression: return var(name)
        if isinstance(value, tuple): return tuple(factor(item, available) for item in value)
        if isinstance(value, list): return [factor(item, available) for item in value]
        return value
    for name, ty, body in reversed(definitions):
        proof = local(name, body, proof)
        statement = local(name, body, statement)
    if mutation: proof = mutate_proof(proof, mutation)
    proof, statement = factor(proof, aliases), factor(statement, aliases)
    for index in reversed(range(len(aliases))):
        name, value = aliases[index]
        value = factor(value, aliases[:index])
        proof, statement = local(name, value, proof), local(name, value, statement)
    proof = bind(lam, parameters, proof)
    statement = bind(pi, parameters, statement)
    return {'term': lower(proof), 'claimed_type': lower(statement), 'metadata': {'theorem': 'finite_observations_preserved' if observations else 'finite_execution_preserved', 'scope': 'admissible selected finite programs; exact Continue/Stop/Refuse and visited states'}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('--observations', action='store_true', help='include pointwise observation commutation and prove exact observation traces')
    parser.add_argument('--mutation', choices=['continue-state','stop-outcome','refusal-outcome','tail-admissibility','observation-map'], help='generate a negative control; its original statement remains fixed')
    args = parser.parse_args()
    encoded = json.dumps(certificate(args.observations, args.mutation), separators=(',', ':')) + '\n'
    args.output.write_text(encoded)
    print(f'wrote {len(encoded.encode())} bytes to {args.output}; not yet a verification verdict')

if __name__ == '__main__': main()
