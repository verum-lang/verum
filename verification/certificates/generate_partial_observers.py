"""Native finite observer folds and exact partial-domain contracts.

One ordered history is replayed from one initial state. All definitions and
premises are checked terms. No learner, runtime adapter, ownership provenance,
or resource-exhaustion interpretation is assumed.
"""
from portable_terms import (
    app, arrow, bind, congruence, cons, construct, data, first, identity, induct,
    lam, list_map, list_type, local, lower, nil, pair, pi, refl, second, sigma,
    symm, transitive, universe, var,
)
from generate_exact_transports import option, none, some, option_map, FLAG, flag, definedness

E, Z, e, es, z, step, ih, current = map(var, ('E', 'Z', 'e', 'es', 'z', 'step', 'ih', 'current'))
F, W, state_map, event_map = map(var, ('F', 'W', 'state_map', 'event_map'))
source_step, target_step = map(var, ('source_step', 'target_step'))
A, B, x, y = map(var, ('A', 'B', 'x', 'y'))

def optional_bind(a, b, value, function):
    return induct(value, lam('bound_option', option(a), option(b)), none(b),
        lam('bound_value', a, app(function, var('bound_value'))))


def fold(event, state, update, events, initial):
    return app(var('fold'), event, state, update, events, initial)


def online(event, state, update, events, value):
    return app(var('online'), event, state, update, events, value)


parameters = [('E', universe()), ('Z', universe()), ('step', arrow(Z, arrow(E, option(Z))))]
fold_type = arrow(Z, option(Z))
fold_definition = bind(lam, parameters + [('es', list_type(E))],
    induct(es, lam('events', list_type(E), fold_type), lam('z', Z, some(Z, z)),
        bind(lam, [('e', E), ('es', list_type(E)), ('ih', fold_type), ('z', Z)],
            optional_bind(Z, Z, app(step, z, e), ih))))
online_type = arrow(option(Z), option(Z))
online_definition = bind(lam, parameters + [('es', list_type(E))],
    induct(es, lam('events', list_type(E), online_type), lam('current', option(Z), current),
        bind(lam, [('e', E), ('es', list_type(E)), ('ih', online_type), ('current', option(Z))],
            app(ih, optional_bind(Z, Z, current, lam('z', Z, app(step, z, e)))))))
definitions = [('fold', fold_definition), ('online', online_definition)]
contracts = []


def names(term):
    if not isinstance(term, (tuple, list)):
        return set()
    if term and term[0] == 'Var':
        return {term[1]}
    return set().union(*(names(child) for child in term))


def close(term, library=None):
    used = names(term)
    for name, value in reversed(definitions if library is None else library):
        if name in used:
            term = local(name, value, term)
            used.update(names(value))
    return term


def certificate(term, claim, library=None):
    return {'term': lower(close(term, library)), 'claimed_type': lower(close(claim, library)), 'metadata': {}}


def add(name, term, claim):
    definitions.append((name, term))
    contracts.append((name, certificate(var(name), claim), True))


none_goal = lambda events: identity(option(Z), online(E, Z, step, events, none(Z)), none(Z))
none_proof = bind(lam, parameters + [('es', list_type(E))],
    induct(es, lam('events', list_type(E), none_goal(var('events'))), refl(none(Z)),
        bind(lam, [('e', E), ('es', list_type(E)), ('ih', none_goal(es))], ih)))
add('online_absence_is_stable', none_proof, bind(pi, parameters + [('es', list_type(E))], none_goal(es)))

online_goal = lambda events, initial: identity(option(Z),
    online(E, Z, step, events, some(Z, initial)), fold(E, Z, step, events, initial))
online_motive = lambda events: pi('z', Z, online_goal(events, z))
head_outcome_goal = lambda outcome: identity(option(Z), online(E, Z, step, es, outcome),
    optional_bind(Z, Z, outcome, lam('next_state', Z, fold(E, Z, step, es, var('next_state')))))
online_case = induct(app(step, z, e), lam('outcome', option(Z), head_outcome_goal(var('outcome'))),
    app(var('online_absence_is_stable'), E, Z, step, es), lam('next_state', Z, app(ih, var('next_state'))))
online_proof = bind(lam, parameters + [('es', list_type(E))],
    induct(es, lam('events', list_type(E), online_motive(var('events'))), lam('z', Z, refl(some(Z, z))),
        bind(lam, [('e', E), ('es', list_type(E)), ('ih', online_motive(es)), ('z', Z)], online_case)))
add('replay_equals_online_fold', online_proof, bind(pi, parameters + [('es', list_type(E))], online_motive(es)))

transport_params = [('E', universe()), ('F', universe()), ('Z', universe()), ('W', universe()),
    ('state_map', arrow(Z, W)), ('event_map', arrow(E, F)),
    ('source_step', arrow(Z, arrow(E, option(Z)))), ('target_step', arrow(W, arrow(F, option(W))))]
update_target = lambda initial, event: app(target_step, app(state_map, initial), app(event_map, event))
update_source = lambda initial, event: app(source_step, initial, event)
mapped = lambda outcome: option_map(Z, W, state_map, outcome)
commutation = bind(pi, [('z', Z), ('e', E)], identity(option(W), update_target(z, e), mapped(update_source(z, e))))
transport_params += [('commutes', commutation)]
events_map = lambda events: list_map(E, F, event_map, events)
source_fold = lambda events, initial: fold(E, Z, source_step, events, initial)
target_fold = lambda events, initial: fold(F, W, target_step, events_map(events), initial)
transport_goal = lambda events, initial: identity(option(W), target_fold(events, app(state_map, initial)), mapped(source_fold(events, initial)))
transport_motive = lambda events: pi('z', Z, transport_goal(events, z))
bind_target = lambda outcome: optional_bind(W, W, outcome, lam('target_state', W, target_fold(es, var('target_state'))))
bind_source = lambda outcome: optional_bind(Z, Z, outcome, lam('source_state', Z, source_fold(es, var('source_state'))))
first_path = congruence(option(W), option(W), lam('target_outcome', option(W), bind_target(var('target_outcome'))),
    update_target(z, e), mapped(update_source(z, e)), app(var('commutes'), z, e))
second_goal = lambda outcome: identity(option(W), bind_target(mapped(outcome)), mapped(bind_source(outcome)))
second_path = induct(update_source(z, e), lam('source_outcome', option(Z), second_goal(var('source_outcome'))),
    refl(none(W)), lam('next_state', Z, app(ih, var('next_state'))))
transport_case = transitive(option(W), bind_target(update_target(z, e)), bind_target(mapped(update_source(z, e))),
    mapped(bind_source(update_source(z, e))), first_path, second_path)
transport_proof = bind(lam, transport_params + [('es', list_type(E))],
    induct(es, lam('events', list_type(E), transport_motive(var('events'))),
        lam('z', Z, refl(some(W, app(state_map, z)))),
        bind(lam, [('e', E), ('es', list_type(E)), ('ih', transport_motive(es)), ('z', Z)], transport_case)))
transport_claim = bind(pi, transport_params + [('es', list_type(E))], transport_motive(es))
add('finite_observer_state_transport', transport_proof, transport_claim)

# Products share the event word. Each partial component is required.
product = lambda a, b: sigma('product_left', a, b)
zip_option = lambda a, b, left, right: optional_bind(a, product(a, b), left,
    lam('zip_left', a, option_map(b, product(a, b), lam('zip_right', b, pair(var('zip_left'), var('zip_right'))), right)))
zip_goal = lambda left: identity(option(product(A, B)), zip_option(A, B, left, none(B)), none(product(A, B)))
zip_none_proof = bind(lam, [('A', universe()), ('B', universe()), ('left', option(A))],
    induct(var('left'), lam('left', option(A), zip_goal(var('left'))), refl(none(product(A, B))),
        lam('x', A, refl(none(product(A, B))))))
add('product_requires_right_component', zip_none_proof,
    bind(pi, [('A', universe()), ('B', universe()), ('left', option(A))], zip_goal(var('left'))))

left_step, right_step = var('left_step'), var('right_step')
product_params = [('E', universe()), ('A', universe()), ('B', universe()),
    ('left_step', arrow(A, arrow(E, option(A)))), ('right_step', arrow(B, arrow(E, option(B))))]
product_step = bind(lam, [('state', product(A, B)), ('event', E)],
    zip_option(A, B, app(left_step, first(var('state')), var('event')), app(right_step, second(var('state')), var('event'))))
left_fold = lambda events, initial: fold(E, A, left_step, events, initial)
right_fold = lambda events, initial: fold(E, B, right_step, events, initial)
combined_fold = lambda events, left, right: fold(E, product(A, B), product_step, events, pair(left, right))
product_goal = lambda events, left, right: identity(option(product(A, B)), combined_fold(events, left, right),
    zip_option(A, B, left_fold(events, left), right_fold(events, right)))
product_motive = lambda events: bind(pi, [('x', A), ('y', B)], product_goal(events, x, y))
continue_left = lambda outcome: optional_bind(A, A, outcome, lam('next_left', A, left_fold(es, var('next_left'))))
continue_right = lambda outcome: optional_bind(B, B, outcome, lam('next_right', B, right_fold(es, var('next_right'))))
continue_product = lambda left, right: optional_bind(product(A, B), product(A, B), zip_option(A, B, left, right),
    lam('next_pair', product(A, B), fold(E, product(A, B), product_step, es, var('next_pair'))))
case_goal = lambda left, right: identity(option(product(A, B)), continue_product(left, right),
    zip_option(A, B, continue_left(left), continue_right(right)))
right_case = induct(app(right_step, y, e), lam('right_outcome', option(B), case_goal(some(A, var('next_left')), var('right_outcome'))),
    # The left tail may also be partial; its unknown result cannot hide a failed right component.
    symm(option(product(A, B)), zip_option(A, B, left_fold(es, var('next_left')), none(B)), none(product(A, B)),
        app(var('product_requires_right_component'), A, B, left_fold(es, var('next_left')))),
    lam('next_right', B, app(ih, var('next_left'), var('next_right'))))
product_case = induct(app(left_step, x, e), lam('left_outcome', option(A), case_goal(var('left_outcome'), app(right_step, y, e))),
    refl(none(product(A, B))), lam('next_left', A, right_case))
product_proof = bind(lam, product_params + [('es', list_type(E))],
    induct(es, lam('events', list_type(E), product_motive(var('events'))),
        bind(lam, [('x', A), ('y', B)], refl(some(product(A, B), pair(x, y)))),
        bind(lam, [('e', E), ('es', list_type(E)), ('ih', product_motive(es)), ('x', A), ('y', B)], product_case)))
add('observer_product_fold', product_proof, bind(pi, product_params + [('es', list_type(E))], product_motive(es)))

and_flag = lambda left, right: induct(left, lam('left_flag', FLAG, FLAG), flag(False), right)
left, right = var('left'), var('right')
domain_goal = lambda a, b: identity(FLAG, definedness(product(A, B), zip_option(A, B, a, b)),
    and_flag(definedness(A, a), definedness(B, b)))
domain_proof = bind(lam, [('A', universe()), ('B', universe()), ('left', option(A)), ('right', option(B))],
    induct(left, lam('left_option', option(A), domain_goal(var('left_option'), right)), refl(flag(False)),
        lam('left_value', A, induct(right, lam('right_option', option(B), domain_goal(some(A, var('left_value')), var('right_option'))),
            refl(flag(False)), lam('right_value', B, refl(flag(True)))))))
add('observer_product_domain_intersection', domain_proof,
    bind(pi, [('A', universe()), ('B', universe()), ('left', option(A)), ('right', option(B))], domain_goal(left, right)))


# Reading is a separate partial operation; update success cannot make it total.
R, Q, source_read, target_read, result_map = map(var, ('R', 'Q', 'source_read', 'target_read', 'result_map'))
observation_params = transport_params + [('R', universe()), ('Q', universe()),
    ('result_map', arrow(R, Q)), ('source_read', arrow(Z, option(R))), ('target_read', arrow(W, option(Q))),
    ('read_commutes', pi('z', Z, identity(option(Q), app(target_read, app(state_map, z)),
        option_map(R, Q, result_map, app(source_read, z)))))]
source_observation = lambda events, initial: optional_bind(Z, R, source_fold(events, initial), source_read)
target_observation = lambda events, initial: optional_bind(W, Q, target_fold(events, initial), target_read)
map_result = lambda value: option_map(R, Q, result_map, value)
observation_goal = lambda events, initial: identity(option(Q), target_observation(events, app(state_map, initial)),
    map_result(source_observation(events, initial)))
transport_actual = [var(name) for name, _ in transport_params]
folded_source = source_fold(es, z)
read_intermediate = optional_bind(W, Q, mapped(folded_source), target_read)
read_first_path = congruence(option(W), option(Q), lam('state_outcome', option(W), optional_bind(W, Q, var('state_outcome'), target_read)),
    target_fold(es, app(state_map, z)), mapped(folded_source), app(var('finite_observer_state_transport'), *transport_actual, es, z))
read_case_goal = lambda value: identity(option(Q), optional_bind(W, Q, mapped(value), target_read),
    map_result(optional_bind(Z, R, value, source_read)))
read_second_path = induct(folded_source, lam('state_outcome', option(Z), read_case_goal(var('state_outcome'))),
    refl(none(Q)), lam('reached_state', Z, app(var('read_commutes'), var('reached_state'))))
observation_proof = bind(lam, observation_params + [('es', list_type(E)), ('z', Z)],
    transitive(option(Q), target_observation(es, app(state_map, z)), read_intermediate,
        map_result(source_observation(es, z)), read_first_path, read_second_path))
add('finite_observer_read_transport', observation_proof,
    bind(pi, observation_params + [('es', list_type(E)), ('z', Z)], observation_goal(es, z)))

initial, target_initial = var('initial'), var('target_initial')
initialized_params = observation_params + [('initial', Z), ('target_initial', W),
    ('initial_commutes', identity(W, target_initial, app(state_map, initial))), ('es', list_type(E))]
initialized_goal = identity(option(Q), target_observation(es, target_initial), map_result(source_observation(es, initial)))
initial_path = congruence(W, option(Q), lam('initial_state', W, target_observation(es, var('initial_state'))),
    target_initial, app(state_map, initial), var('initial_commutes'))
initialized_path = transitive(option(Q), target_observation(es, target_initial), target_observation(es, app(state_map, initial)),
    map_result(source_observation(es, initial)), initial_path,
    app(var('finite_observer_read_transport'), *[var(name) for name, _ in observation_params], es, initial))
add('initial_step_read_diagrams_preserve_observations', bind(lam, initialized_params, initialized_path),
    bind(pi, initialized_params, initialized_goal))

observed_source = source_observation(es, initial)
observed_target = target_observation(es, target_initial)
status_goal = identity(FLAG, definedness(Q, observed_target), definedness(R, observed_source))
status_first = congruence(option(Q), FLAG, lam('result_outcome', option(Q), definedness(Q, var('result_outcome'))),
    observed_target, map_result(observed_source),
    app(var('initial_step_read_diagrams_preserve_observations'), *[var(name) for name, _ in initialized_params]))
status_second_goal = lambda value: identity(FLAG, definedness(Q, map_result(value)), definedness(R, value))
status_second = induct(observed_source, lam('result_outcome', option(R), status_second_goal(var('result_outcome'))),
    refl(flag(False)), lam('result_value', R, refl(flag(True))))
status_proof = bind(lam, initialized_params, transitive(FLAG, definedness(Q, observed_target),
    definedness(Q, map_result(observed_source)), definedness(R, observed_source), status_first, status_second))
add('observer_read_definedness_preserved_and_reflected', status_proof, bind(pi, initialized_params, status_goal))

left_read, right_read = var('left_read'), var('right_read')
product_read_params = product_params + [('R', universe()), ('Q', universe()),
    ('left_read', arrow(A, option(R))), ('right_read', arrow(B, option(Q))),
    ('es', list_type(E)), ('x', A), ('y', B)]
product_read = lam('product_state', product(A, B), zip_option(R, Q,
    app(left_read, first(var('product_state'))), app(right_read, second(var('product_state')))))
left_observed = optional_bind(A, R, left_fold(es, x), left_read)
right_observed = optional_bind(B, Q, right_fold(es, y), right_read)
product_observed = optional_bind(product(A, B), product(R, Q), combined_fold(es, x, y), product_read)
product_read_mid = optional_bind(product(A, B), product(R, Q), zip_option(A, B, left_fold(es, x), right_fold(es, y)), product_read)
product_read_end = zip_option(R, Q, left_observed, right_observed)
product_read_first = congruence(option(product(A, B)), option(product(R, Q)),
    lam('product_outcome', option(product(A, B)), optional_bind(product(A, B), product(R, Q), var('product_outcome'), product_read)),
    combined_fold(es, x, y), zip_option(A, B, left_fold(es, x), right_fold(es, y)),
    app(var('observer_product_fold'), *[var(name) for name, _ in product_params], es, x, y))
product_read_case_goal = lambda a, b: identity(option(product(R, Q)),
    optional_bind(product(A, B), product(R, Q), zip_option(A, B, a, b), product_read),
    zip_option(R, Q, optional_bind(A, R, a, left_read), optional_bind(B, Q, b, right_read)))
right_read_case = induct(right_fold(es, y), lam('right_outcome', option(B), product_read_case_goal(some(A, var('left_state')), var('right_outcome'))),
    symm(option(product(R, Q)), zip_option(R, Q, app(left_read, var('left_state')), none(Q)), none(product(R, Q)),
        app(var('product_requires_right_component'), R, Q, app(left_read, var('left_state')))),
    lam('right_state', B, refl(zip_option(R, Q, app(left_read, var('left_state')), app(right_read, var('right_state'))))))
product_read_second = induct(left_fold(es, x), lam('left_outcome', option(A), product_read_case_goal(var('left_outcome'), right_fold(es, y))),
    refl(none(product(R, Q))), lam('left_state', A, right_read_case))
add('observer_product_read', bind(lam, product_read_params,
    transitive(option(product(R, Q)), product_observed, product_read_mid, product_read_end, product_read_first, product_read_second)),
    bind(pi, product_read_params, identity(option(product(R, Q)), product_observed, product_read_end)))


# Reuse the existing exact-transport map-fusion proof to compose observer steps.
from generate_exact_transports import embedding_parameters, fusion

definitions.append(('observer_map_fusion', bind(lam,
    embedding_parameters() + [('value', option(var('A')))], fusion(var('value')))))
ThirdEvent, ThirdState, state_map2, event_map2, third_step = map(var,
    ('ThirdEvent', 'ThirdState', 'state_map2', 'event_map2', 'third_step'))
composite_state = lam('composed_state', Z, app(state_map2, app(state_map, var('composed_state'))))
composite_event = lam('composed_event', E, app(event_map2, app(event_map, var('composed_event'))))
second_commutation = bind(pi, [('middle_state', W), ('middle_event', F)],
    identity(option(ThirdState), app(third_step, app(state_map2, var('middle_state')), app(event_map2, var('middle_event'))),
        option_map(W, ThirdState, state_map2, app(target_step, var('middle_state'), var('middle_event')))))
composition_params = transport_params + [('ThirdEvent', universe()), ('ThirdState', universe()),
    ('state_map2', arrow(W, ThirdState)), ('event_map2', arrow(F, ThirdEvent)),
    ('third_step', arrow(ThirdState, arrow(ThirdEvent, option(ThirdState)))), ('second_commutes', second_commutation)]
map_second = lambda outcome: option_map(W, ThirdState, state_map2, outcome)
composed_start = app(third_step, app(composite_state, z), app(composite_event, e))
composed_middle = map_second(update_target(z, e))
composed_middle2 = map_second(mapped(update_source(z, e)))
composed_end = option_map(Z, ThirdState, composite_state, update_source(z, e))
composed_second_path = congruence(option(W), option(ThirdState), lam('middle_outcome', option(W), map_second(var('middle_outcome'))),
    update_target(z, e), mapped(update_source(z, e)), app(var('commutes'), z, e))
composed_last_path = app(var('observer_map_fusion'), Z, W, ThirdState, state_map, state_map2, update_source(z, e))
composition_goal = identity(option(ThirdState),
    fold(ThirdEvent, ThirdState, third_step, list_map(E, ThirdEvent, composite_event, es), app(composite_state, z)),
    option_map(Z, ThirdState, composite_state, source_fold(es, z)))


def composed_observer_proof(omit_second=False):
    first = refl(composed_start) if omit_second else app(var('second_commutes'), app(state_map, z), app(event_map, e))
    diagram = bind(lam, [('z', Z), ('e', E)], transitive(option(ThirdState),
        composed_start, composed_middle, composed_end, first,
        transitive(option(ThirdState), composed_middle, composed_middle2, composed_end, composed_second_path, composed_last_path)))
    return bind(lam, composition_params + [('es', list_type(E)), ('z', Z)],
        app(var('finite_observer_state_transport'), E, ThirdEvent, Z, ThirdState,
            composite_state, composite_event, source_step, third_step, diagram, es, z))


composition_claim = bind(pi, composition_params + [('es', list_type(E)), ('z', Z)], composition_goal)
add('finite_observer_transports_compose', composed_observer_proof(), composition_claim)


def typed_observer_certificate(definedness_only=False):
    """Discharge all diagrams for arbitrary typed event and read-query rows."""
    import generate_typed_programs as programs
    state_sort, result_sort = map(var, ('state_sort', 'result_sort'))
    event_context, query_context = map(var, ('event_context', 'query_context'))
    update, read, initial, events, query = map(var,
        ('observer_update', 'observer_read', 'observer_initial', 'observer_events', 'observer_query'))
    source_value = lambda sort: app(programs.V, sort)
    target_value = lambda sort: app(programs.W, app(programs.rho, sort))
    embed = lambda sort: app(programs.jval, sort)
    source_row = lambda context: programs.HL(programs.S, programs.V, context)
    target_row = lambda context: programs.HL(programs.T, programs.W, programs.MAP(context))
    source_context = programs.C(programs.S, state_sort, event_context)
    read_context = programs.C(programs.S, state_sort, query_context)
    state, event = var('observer_state'), var('observer_event')
    source_update = bind(lam, [('observer_state', source_value(state_sort)), ('observer_event', source_row(event_context))],
        programs.RUN(source_context, state_sort, update, pair(state, event)))
    target_update = bind(lam, [('observer_state', target_value(state_sort)), ('observer_event', target_row(event_context))],
        programs.TRUN(programs.MAP(source_context), app(programs.rho, state_sort),
            programs.TR(source_context, state_sort, update), pair(state, event)))
    source_read = lam('observer_state', source_value(state_sort), programs.RUN(read_context, result_sort, read, pair(state, query)))
    target_read = lam('observer_state', target_value(state_sort),
        programs.TRUN(programs.MAP(read_context), app(programs.rho, result_sort),
            programs.TR(read_context, result_sort, read), pair(state, programs.MAP_ENV(query_context, query))))
    exact_update = bind(lam, [('observer_state', source_value(state_sort)), ('observer_event', source_row(event_context))],
        app(var('runTranslate'), *programs.translation_sem_actual, source_context, state_sort, update, pair(state, event)))
    exact_read = lam('observer_state', source_value(state_sort),
        app(var('runTranslate'), *programs.translation_sem_actual, read_context, result_sort, read, pair(state, query)))
    event_embed = lam('observer_event', source_row(event_context), programs.MAP_ENV(event_context, event))
    params = programs.translation_sem_params + [('state_sort', programs.S), ('result_sort', programs.S),
        ('event_context', programs.L(programs.S)), ('query_context', programs.L(programs.S)),
        ('observer_update', programs.PR(programs.S, programs.P, programs.Lit, source_context, state_sort)),
        ('observer_read', programs.PR(programs.S, programs.P, programs.Lit, read_context, result_sort)),
        ('observer_initial', source_value(state_sort)), ('observer_events', list_type(source_row(event_context))),
        ('observer_query', source_row(query_context))]
    mapped_initial = app(embed(state_sort), initial)
    proof_name = 'observer_read_definedness_preserved_and_reflected' if definedness_only else 'initial_step_read_diagrams_preserve_observations'
    proof = app(var(proof_name), source_row(event_context), target_row(event_context),
        source_value(state_sort), target_value(state_sort), embed(state_sort), event_embed, source_update, target_update,
        exact_update, source_value(result_sort), target_value(result_sort), embed(result_sort), source_read, target_read, exact_read,
        initial, mapped_initial, refl(mapped_initial), events)
    source_observed = optional_bind(source_value(state_sort), source_value(result_sort),
        fold(source_row(event_context), source_value(state_sort), source_update, events, initial), source_read)
    target_observed = optional_bind(target_value(state_sort), target_value(result_sort),
        fold(target_row(event_context), target_value(state_sort), target_update,
            list_map(source_row(event_context), target_row(event_context), event_embed, events), mapped_initial), target_read)
    claim = identity(option(target_value(result_sort)), target_observed,
        option_map(source_value(result_sort), target_value(result_sort), embed(result_sort), source_observed))
    if definedness_only:
        claim = identity(FLAG, definedness(target_value(result_sort), target_observed),
            definedness(source_value(result_sort), source_observed))
    library = programs.definitions + definitions
    return certificate(bind(lam, params, proof), bind(pi, params, claim), library)


def cases():
    result = list(contracts)
    result.append(('discard-second-observer-transport', certificate(composed_observer_proof(True), composition_claim), False))
    result.append(('typed_program_observer_trace_preservation', typed_observer_certificate(), True))
    result.append(('typed_program_observer_definedness_reflection', typed_observer_certificate(True), True))
    # Remove the step diagram while retaining the same general claimed theorem.
    altered_case = transitive(option(W), bind_target(update_target(z, e)), bind_target(mapped(update_source(z, e))),
        mapped(bind_source(update_source(z, e))), refl(bind_target(update_target(z, e))), second_path)
    altered = bind(lam, transport_params + [('es', list_type(E))],
        induct(es, lam('events', list_type(E), transport_motive(var('events'))), lam('z', Z, refl(some(W, app(state_map, z)))),
            bind(lam, [('e', E), ('es', list_type(E)), ('ih', transport_motive(es)), ('z', Z)], altered_case)))
    result.append(('discard-update-commutation', certificate(altered, transport_claim), False))
    omitted_initial = transitive(option(Q), target_observation(es, target_initial),
        target_observation(es, app(state_map, initial)), map_result(source_observation(es, initial)),
        refl(target_observation(es, target_initial)),
        app(var('finite_observer_read_transport'), *[var(name) for name, _ in observation_params], es, initial))
    result.append(('discard-initial-state-commutation', certificate(bind(lam, initialized_params, omitted_initial),
        bind(pi, initialized_params, initialized_goal)), False))
    omitted_read = induct(folded_source, lam('state_outcome', option(Z), read_case_goal(var('state_outcome'))),
        refl(none(Q)), lam('reached_state', Z, refl(app(target_read, app(state_map, var('reached_state'))))))
    read_changed = bind(lam, observation_params + [('es', list_type(E)), ('z', Z)],
        transitive(option(Q), target_observation(es, app(state_map, z)), read_intermediate,
            map_result(source_observation(es, z)), read_first_path, omitted_read))
    result.append(('discard-read-commutation', certificate(read_changed,
        bind(pi, observation_params + [('es', list_type(E)), ('z', Z)], observation_goal(es, z))), False))
    bit = data('Bit', [[], []])
    zero = construct(bit, 0)
    blocked = zip_option(bit, bit, some(bit, zero), none(bit))
    result.append(('pending-product-cannot-emit-old-output', certificate(refl(blocked),
        identity(option(product(bit, bit)), blocked, some(product(bit, bit), pair(zero, zero)))), False))
    one = construct(bit, 1)
    last_event = bind(lam, [('z', bit), ('e', bit)], some(bit, e))
    ordered = fold(bit, bit, last_event, cons(bit, zero, cons(bit, one, nil(bit))), zero)
    reversed_events = fold(bit, bit, last_event, cons(bit, one, cons(bit, zero, nil(bit))), zero)
    result.append(('ordered-events-keep-last-input', certificate(refl(ordered), identity(option(bit), ordered, some(bit, one))), True))
    result.append(('reordered-events-are-not-the-same-history', certificate(refl(ordered), identity(option(bit), ordered, reversed_events)), False))
    unchanged = fold(bit, bit, last_event, nil(bit), one)
    result.append(('empty-history-does-not-reset-initial-state', certificate(refl(unchanged), identity(option(bit), unchanged, some(bit, zero))), False))
    return result
