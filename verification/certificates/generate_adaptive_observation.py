"""Universal reversible lifts and total/Option readout laws.

Untrusted proof producer: native and independent replay check every definition.
Products, existence and iff use constructive Sigma encodings; false is the empty
inductive type. Types have independent symbolic universe levels. No runtime,
computability, statistical-risk, resource-cost or physical-register claim.
"""
from portable_terms import (
    app, arrow, bind, congruence, construct, data, first, fun_ext, identity,
    induct, j, lam, local, lower, pair, pi, refl, second, sigma, symm,
    transitive, tt, unit, universe, value_field, var,
)


def u(name): return universe({'Var': name})


BOOL = data('Bool', [[], []])
EMPTY = data('Empty', [])
false, true = construct(BOOL, 0), construct(BOOL, 1)
product = lambda a, b: sigma('product_component', a, b)
option = lambda a: data('Option', [[], [value_field(a)]], level={'Var': 'x'})
none = lambda a: construct(option(a), 0)
X, I, f, s, t, a, b, c, h = map(var, ('X', 'I', 'f', 's', 't', 'a', 'b', 'c', 'h'))
definitions = []
contracts = []
foundations = {}


def names(term):
    if not isinstance(term, (tuple, list)):
        return set()
    if term and term[0] == 'Var':
        return {term[1]}
    return set().union(*(names(child) for child in term))


def close(term):
    used = names(term)
    for name, definition in reversed(definitions):
        if name in used:
            term = local(name, definition, term)
            used.update(names(definition))
    return term


def certificate(term, claim):
    return {'term': lower(close(term)), 'claimed_type': lower(close(claim)), 'metadata': {}}


def add(name, params, proof, claim, *, extensional=False):
    definitions.append((name, bind(lam, params, proof)))
    contracts.append((name, certificate(var(name), bind(pi, params, claim)), True))
    if extensional:
        foundations[name] = 'function-extensionality'


xor_body = induct(b, lam('xor_left', BOOL, BOOL), c,
    induct(c, lam('xor_right', BOOL, BOOL), true, false))
definitions.append(('xor', bind(lam, [('b', BOOL), ('c', BOOL)], xor_body)))
xor = lambda left, right: app(var('xor'), left, right)
cancel = lambda left, right: app(var('xor_cancel'), left, right)
cancel_goal = lambda left, right: identity(BOOL, xor(xor(left, right), right), left)
cancel_case = lambda left: lam('c', BOOL, induct(c,
    lam('cancel_right', BOOL, cancel_goal(left, var('cancel_right'))), refl(left), refl(left)))
add('xor_cancel', [('b', BOOL), ('c', BOOL)], app(induct(b,
    lam('cancel_left', BOOL, pi('c', BOOL, cancel_goal(var('cancel_left'), c))),
    cancel_case(false), cancel_case(true)), c), cancel_goal(b, c))


state = product(X, BOOL)
lift_params = [('X', u('u')), ('f', arrow(X, BOOL))]
lift = lambda value: app(var('lift'), X, f, value)
definitions.append(('lift', bind(lam, lift_params + [('s', state)],
    pair(first(s), xor(second(s), app(f, first(s)))))))
add('lift_preserves_source', lift_params + [('s', state)], refl(first(s)),
    identity(X, first(lift(s)), first(s)))
add('lift_initial_readout', lift_params + [('a', X)], refl(app(f, a)),
    identity(BOOL, second(lift(pair(a, false))), app(f, a)))
involution = lambda value: app(var('lift_involutive'), X, f, value)
add('lift_involutive', lift_params + [('s', state)],
    congruence(BOOL, state, lam('lift_bit', BOOL, pair(first(s), var('lift_bit'))),
        xor(xor(second(s), app(f, first(s))), app(f, first(s))), second(s),
        cancel(second(s), app(f, first(s)))), identity(state, lift(lift(s)), s))
first_inverse = symm(state, lift(lift(s)), s, involution(s))
lift_congruence = congruence(state, state, app(var('lift'), X, f), lift(s), lift(t), h)
add('lift_injective', lift_params + [('s', state), ('t', state), ('h', identity(state, lift(s), lift(t)))],
    transitive(state, s, lift(lift(s)), t, first_inverse,
        transitive(state, lift(lift(s)), lift(lift(t)), t, lift_congruence, involution(t))),
    identity(state, s, t))


bits = arrow(I, BOOL)
bit_state = product(X, bits)
bits_params = [('X', u('u')), ('I', u('v')), ('f', arrow(X, bits))]
lift_bits = lambda value: app(var('liftBits'), X, I, f, value)
definitions.append(('liftBits', bind(lam, bits_params + [('s', bit_state)], pair(first(s),
    lam('i', I, xor(app(second(s), var('i')), app(f, first(s), var('i'))))))))
twice = second(lift_bits(lift_bits(s)))
pointwise = lam('i', I, cancel(app(second(s), var('i')), app(f, first(s), var('i'))))
add('liftBits_involutive', bits_params + [('s', bit_state)],
    congruence(bits, bit_state, lam('lift_register', bits, pair(first(s), var('lift_register'))),
        twice, second(s), fun_ext(bits, twice, second(s), pointwise)),
    identity(bit_state, lift_bits(lift_bits(s)), s), extensional=True)
initial_readout = second(lift_bits(pair(a, lam('i', I, false))))
add('liftBits_initial_readout', bits_params + [('a', X)],
    fun_ext(bits, initial_readout, app(f, a), lam('i', I, refl(app(f, a, var('i'))))),
    identity(bits, initial_readout, app(f, a)), extensional=True)


H, Z0, Z1, A, L = map(var, ('H', 'Z0', 'Z1', 'A', 'L'))
r0, r1, projection, refines, old, loss = map(var, ('r0', 'r1', 'projection', 'refines', 'old', 'loss'))
observed = app(r0, h)
projected = app(projection, app(r1, h))
left_decision, right_decision = app(old, observed), app(old, projected)
type_params = [('H', u('u')), ('Z0', u('v')), ('Z1', u('w')), ('A', u('x'))]
observation_params = [('r0', arrow(H, Z0)), ('r1', arrow(H, Z1)), ('projection', arrow(Z1, Z0)),
    ('refines', pi('h', H, identity(Z0, observed, projected)))]
decision_proof = lambda carrier: congruence(Z0, carrier, old, observed, projected, app(refines, h))
add('decision_preserved', type_params + observation_params + [('old', arrow(Z0, A)), ('h', H)],
    decision_proof(A), identity(A, left_decision, right_decision))
add('pointwise_loss_preserved', type_params + [('L', u('y'))] + observation_params +
    [('old', arrow(Z0, A)), ('loss', arrow(H, arrow(A, L))), ('h', H)],
    congruence(A, L, app(loss, h), left_decision, right_decision, decision_proof(A)),
    identity(L, app(loss, h, left_decision), app(loss, h, right_decision)))
partial_params = type_params + observation_params + [('old', arrow(Z0, option(A))), ('h', H)]
add('partial_decision_preserved', partial_params, decision_proof(option(A)),
    identity(option(A), left_decision, right_decision))
left_none = identity(option(A), left_decision, none(A))
right_none = identity(option(A), right_decision, none(A))
none_family = lam('undefined_value', option(A), identity(option(A), var('undefined_value'), none(A)))
add('undefined_reflected', partial_params, pair(
    lam('left_none', left_none, j(none_family, var('left_none'), decision_proof(option(A)))),
    lam('right_none', right_none, j(none_family, var('right_none'),
        symm(option(A), left_decision, right_decision, decision_proof(option(A)))))),
    product(arrow(left_none, right_none), arrow(right_none, left_none)))


def no_factor(levels):
    H, Z, Y, r, target, a, b, same, different, factors = map(var,
        ('H', 'Z', 'Y', 'r', 'target', 'a', 'b', 'same', 'different', 'factors'))
    readout = first(factors)
    validity = second(factors)
    ta, tb = app(target, a), app(target, b)
    ra, rb = app(readout, app(r, a)), app(readout, app(r, b))
    factor_type = sigma('readout', arrow(Z, Y), pi('history', H,
        identity(Y, app(var('readout'), app(r, var('history'))), app(target, var('history')))))
    params = [('H', universe(levels[0])), ('Z', universe(levels[1])), ('Y', universe(levels[2])),
        ('r', arrow(H, Z)), ('target', arrow(H, Y)), ('a', H), ('b', H),
        ('same', identity(Z, app(r, a), app(r, b))), ('different', arrow(identity(Y, ta, tb), EMPTY))]
    proof = lam('factors', factor_type, app(different,
        transitive(Y, ta, ra, tb, symm(Y, ra, ta, app(validity, a)),
            transitive(Y, ra, rb, tb, congruence(Z, Y, readout, app(r, a), app(r, b), same), app(validity, b)))))
    return params, proof, arrow(factor_type, EMPTY)


params, proof, claim = no_factor([{'Var': level} for level in ('u', 'v', 'w')])
add('no_factor_of_separated_fiber', params, proof, claim)
zero_params, zero_proof, _ = no_factor([0, 0, 0])
discriminate = lam('distinct_bit', BOOL,
    induct(var('distinct_bit'), lam('bit_type', BOOL, universe()), unit(), EMPTY))
different_bits = lam('false_equals_true', identity(BOOL, false, true),
    j(discriminate, tt(), var('false_equals_true')))
blind_exists = sigma('readout', arrow(unit(), BOOL), pi('b', BOOL,
    identity(BOOL, app(var('readout'), tt()), b)))
add('blind_bit_impossible', [], app(bind(lam, zero_params, zero_proof),
    BOOL, unit(), BOOL, lam('b', BOOL, tt()), lam('b', BOOL, b), false, true, refl(tt()), different_bits),
    arrow(blind_exists, EMPTY))
rich_exists = sigma('readout', arrow(BOOL, BOOL), pi('b', BOOL,
    identity(BOOL, app(var('readout'), b), b)))
add('richer_bit_suffices', [], pair(lam('b', BOOL, b), lam('b', BOOL, refl(b))), rich_exists)


def cases():
    result = list(contracts)
    # Semantic controls matching the frozen source: erased history destroys
    # involution; a constant readout cannot reconstruct an arbitrary bit.
    erased = lam('state', product(BOOL, BOOL), pair(false,
        xor(second(var('state')), first(var('state')))))
    initial = pair(true, false)
    result.append(('false_erased_lift', certificate(refl(initial),
        identity(product(BOOL, BOOL), app(erased, app(erased, initial)), initial)), False))
    result.append(('false_blind_readout', certificate(lam('b', BOOL, refl(false)),
        pi('b', BOOL, identity(BOOL, false, b))), False))
    # Full generic theorems cannot lose their refinement or universal evidence.
    params = type_params + observation_params + [('old', arrow(Z0, A)), ('h', H)]
    result.append(('missing_refinement_evidence', certificate(bind(lam, params, refl(left_decision)),
        bind(pi, params, identity(A, left_decision, right_decision))), False))
    params = bits_params + [('s', bit_state)]
    forged = congruence(bits, bit_state, lam('lift_register', bits, pair(first(s), var('lift_register'))),
        twice, second(s), fun_ext(bits, twice, second(s), lam('i', I, refl(app(second(s), var('i'))))))
    result.append(('missing_pointwise_cancellation', certificate(bind(lam, params, forged),
        bind(pi, params, identity(bit_state, lift_bits(lift_bits(s)), s))), False))
    foundations['missing_pointwise_cancellation'] = 'function-extensionality'
    return result
