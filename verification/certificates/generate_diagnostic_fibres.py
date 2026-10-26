"""General information-loss obstruction for total diagnostic decoders."""
from portable_terms import (
    app, arrow, bind, congruence, data, identity, lam, lower, pi, refl,
    symm, transitive, universe, var,
)


def certificate(kind, mutation=None):
    history, diagnostic, answer = var('History'), var('Diagnostic'), var('Answer')
    projection, query, decoder = var('projection'), var('query'), var('decoder')
    a, b = var('a'), var('b')
    da, db = app(projection, a), app(projection, b)
    qa, qb = app(query, a), app(query, b)
    fa, fb = app(decoder, da), app(decoder, db)
    empty = data('Empty', [])
    params = [('History', universe()), ('Diagnostic', universe()), ('Answer', universe()),
        ('projection', arrow(history, diagnostic)), ('query', arrow(history, answer)),
        ('a', history), ('b', history), ('collision', identity(diagnostic, da, db))]
    equality = identity(answer, qa, qb)
    if kind != 'factorization_preserves_fibres':
        params.append(('distinct', arrow(equality, empty)))
    assumptions = [('decoder', arrow(diagnostic, answer))]
    if kind == 'no_global_factorization':
        assumptions.append(('correct', pi('h', history,
            identity(answer, app(decoder, app(projection, var('h'))), app(query, var('h'))))))
        left, right = app(var('correct'), a), app(var('correct'), b)
    else:
        assumptions += [('correctA', identity(answer, fa, qa)), ('correctB', identity(answer, fb, qb))]
        left, right = var('correctA'), var('correctB')
    first = symm(answer, fa, qa, left)
    middle = congruence(diagnostic, answer, decoder, da, db, var('collision'))
    if mutation == 'missing-collision': middle = refl(fa)
    if mutation == 'wrong-answer-endpoint': right = left
    if mutation == 'unreversed-first-agreement': first = left
    proof = transitive(answer, qa, fb, qb,
        transitive(answer, qa, fa, fb, first, middle), right)
    if mutation == 'reflexivity-without-factorization': proof = refl(qa)
    if kind == 'factorization_preserves_fibres':
        target = bind(pi, assumptions, equality)
        proof = bind(lam, assumptions, proof)
    else:
        if mutation == 'reverse-inequality-input': proof = symm(answer, qa, qb, proof)
        if mutation == 'diagnostics-as-answers': proof = var('collision')
        target = bind(pi, assumptions, empty)
        proof = bind(lam, assumptions, app(var('distinct'), proof))
    return {'term': lower(bind(lam, params, proof)), 'claimed_type': lower(bind(pi, params, target)),
        'metadata': {'theorem': kind,
            'scope': 'arbitrary histories, projections and queries; a collision with unequal answers forbids a total exact decoder; no Rust correspondence'}}


def cases():
    names = ['factorization_preserves_fibres', 'no_decoder_on_collision_pair', 'no_global_factorization']
    result = [(name, certificate(name), True) for name in names]
    for name, mutations in [
        (names[0], ['missing-collision', 'wrong-answer-endpoint', 'unreversed-first-agreement',
            'reflexivity-without-factorization']),
        (names[1], ['reverse-inequality-input', 'diagnostics-as-answers']),
    ]:
        for mutation in mutations:
            altered = certificate(name, mutation)
            assert altered['claimed_type'] == certificate(name)['claimed_type']
            result.append((mutation, altered, False))
    return result
