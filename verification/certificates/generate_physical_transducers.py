"""Universal schedule erasure, preserving exact ordered physical payloads.

Untrusted producer. Service is a stutter in the defined model, not an assumed
property of runtime scheduling. Physical payloads can include source/cut IDs.
No finite capacity, allocation, statistical or native refinement claim.
"""
from portable_terms import (RECURSIVE, app, arrow, bind, congruence, construct,
    data, identity, induct, lam, local, lower, pi, refl, symm, transitive,
    universe, value_field, var)

X, G, S, observe, state, e, events, ih = map(var,
    ('X', 'G', 'S', 'observe', 'state', 'e', 'events', 'ih'))
LX, LG, LS = ({'Var': name} for name in ('u', 'v', 'w'))
LE = {'Max': [LX, LG]}
types = [('X', universe(LX)), ('G', universe(LG)), ('S', universe(LS))]
params = types + [('observe', arrow(S, arrow(X, S)))]
event_type = data('Event', [[value_field(X)], [value_field(G)]], level=LE)
observation = lambda x: construct(event_type, 0, x)
service = lambda grant: construct(event_type, 1, grant)
list_of = lambda a, level: data('List', [[], [value_field(a), RECURSIVE]], level=level)
word, physical_word = list_of(event_type, LE), list_of(X, LX)
empty = lambda declaration: construct(declaration, 0)
cons = lambda declaration, x, xs: construct(declaration, 1, x, xs)
definitions, contracts = [], []


def close(term):
    for name, definition in reversed(definitions): term = local(name, definition, term)
    return term


def certificate(term, claim):
    return dict(term=lower(close(term)), claimed_type=lower(close(claim)), metadata={})


def add(name, bindings, proof, claim):
    definitions.append((name, bind(lam, bindings, proof)))
    contracts.append((name, certificate(var(name), bind(pi, bindings, claim)), True))


step = lambda initial, event: app(var('step'), X, G, S, observe, initial, event)
definitions.append(('step', bind(lam, params + [('state', S), ('e', event_type)],
    induct(e, lam('event', event_type, S),
        lam('x', X, app(observe, state, var('x'))), lam('grant', G, state)))))
run = lambda items, initial: app(var('run'), X, G, S, observe, items, initial)
definitions.append(('run', bind(lam, params + [('events', word)],
    induct(events, lam('word', word, arrow(S, S)), lam('state', S, state),
        bind(lam, [('e', event_type), ('events', word), ('ih', arrow(S, S)), ('state', S)],
             app(ih, step(state, e)))))))
physical = lambda items: app(var('physical'), X, G, items)
definitions.append(('physical', bind(lam, types[:2] + [('events', word)],
    induct(events, lam('word', word, physical_word), empty(physical_word),
        bind(lam, [('e', event_type), ('events', word), ('ih', physical_word)],
            induct(e, lam('event', event_type, physical_word),
                lam('x', X, cons(physical_word, var('x'), ih)), lam('grant', G, ih)))))))
fold = lambda items, initial: app(var('fold'), X, S, observe, items, initial)
definitions.append(('fold', bind(lam, [types[0], types[2], params[-1], ('events', physical_word)],
    induct(events, lam('word', physical_word, arrow(S, S)), lam('state', S, state),
        bind(lam, [('x', X), ('events', physical_word), ('ih', arrow(S, S)), ('state', S)],
             app(ih, app(observe, state, var('x'))))))))
add('service_stutters', params + [('state', S), ('grant', G)], refl(state),
    identity(S, step(state, service(var('grant'))), state))
add('observation_updates', params + [('state', S), ('x', X)], refl(app(observe, state, var('x'))),
    identity(S, step(state, observation(var('x'))), app(observe, state, var('x'))))

erasure = lambda items, initial: identity(S, run(items, initial), fold(physical(items), initial))
motive = lambda items: pi('state', S, erasure(items, state))
case_goal = lambda event: erasure(cons(word, event, events), state)
case = induct(e, lam('event', event_type, case_goal(var('event'))),
    lam('x', X, app(ih, app(observe, state, var('x')))), lam('grant', G, app(ih, state)))
proof = app(induct(events, lam('word', word, motive(var('word'))), lam('state', S, refl(state)),
    bind(lam, [('e', event_type), ('events', word), ('ih', motive(events)), ('state', S)], case)), state)
add('service_erasure', params + [('events', word), ('state', S)], proof, erasure(events, state))
law = lambda items: app(var('service_erasure'), X, G, S, observe, items, state)
a, b = var('a'), var('b')
middle = congruence(physical_word, S, lam('xs', physical_word, fold(var('xs'), state)),
    physical(a), physical(b), var('same_history'))
proof = transitive(S, run(a, state), fold(physical(a), state), run(b, state), law(a),
    transitive(S, fold(physical(a), state), fold(physical(b), state), run(b, state), middle,
        symm(S, run(b, state), fold(physical(b), state), law(b))))
add('same_physical_history_same_state', params + [('a', word), ('b', word), ('state', S),
    ('same_history', identity(physical_word, physical(a), physical(b)))], proof,
    identity(S, run(a, state), run(b, state)))


def cases():
    result = list(contracts)
    # False universal claims fail at proof checking, not at schema admission.
    result.append(('service_changes_state', certificate(
        bind(lam, params + [('state', S), ('other', S), ('grant', G)], refl(state)),
        bind(pi, params + [('state', S), ('other', S), ('grant', G)],
            identity(S, step(state, service(var('grant'))), var('other')))), False))
    result.append(('physical_observation_erased', certificate(
        bind(lam, params + [('state', S), ('x', X)], refl(state)),
        bind(pi, params + [('state', S), ('x', X)],
            identity(S, step(state, observation(var('x'))), state))), False))
    result.append(('physical_order_ignored', certificate(
        bind(lam, params + [('state', S), ('x', X), ('y', X)],
            refl(app(observe, app(observe, state, var('x')), var('y')))),
        bind(pi, params + [('state', S), ('x', X), ('y', X)], identity(S,
            app(observe, app(observe, state, var('x')), var('y')),
            app(observe, app(observe, state, var('y')), var('x'))))), False))
    return result
