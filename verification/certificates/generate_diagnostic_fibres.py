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


def obstruction_cases():
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


def reachable_certificate(kind, mutation=None):
    """Constructive unique initialization and exact images of compatible families."""
    from portable_terms import first, second, pair, sigma
    hty, dty, aty, bty = map(var, ('History', 'Diagnostic', 'Answer', 'Refined'))
    reach, projection, query, fine, forget = map(var, ('reachable', 'projection', 'query', 'fine', 'forget'))
    d, a, b = map(var, ('d', 'a', 'b'))
    params = [('History', universe()), ('Diagnostic', universe()), ('Answer', universe()),
        ('reachable', arrow(hty, universe())), ('projection', arrow(hty, dty)), ('query', arrow(hty, aty))]

    def fibre(observed):
        return sigma('history', hty, sigma('reachable_evidence', app(reach, var('history')),
            identity(dty, app(projection, var('history')), observed)))

    def compatible(carrier, state, observed, value):
        return sigma('history', hty, sigma('reachable_evidence', app(reach, var('history')),
            sigma('observation_evidence', identity(dty, app(projection, var('history')), observed),
                identity(carrier, app(state, var('history')), value))))

    def witness(h, r, collision, equal):
        return pair(h, pair(r, pair(collision, equal)))

    def unique(observed):
        return sigma('answer', aty,
            sigma('inhabited', compatible(aty, query, observed, var('answer')),
                bind(pi, [('other', aty), ('candidate', compatible(aty, query, observed, var('other')))],
                    identity(aty, var('answer'), var('other')))))

    constant = bind(pi, [('a', hty), ('b', hty), ('ra', app(reach, a)), ('rb', app(reach, b)),
        ('collision', identity(dty, app(projection, a), app(projection, b)))],
        identity(aty, app(query, a), app(query, b)))
    initializer = bind(pi, [('d', dty), ('occupied', fibre(d))], unique(d))

    if kind == 'reachable_fibre_constancy_initializes_uniquely':
        params += [('constant', constant), ('d', dty), ('occupied', fibre(d))]
        supplied = var('occupied')
        h, r, hd = first(supplied), first(second(supplied)), second(second(supplied))
        qa = app(query, h)
        candidate = var('candidate')
        k, kr = first(candidate), first(second(candidate))
        kd, ka = first(second(second(candidate))), second(second(second(candidate)))
        collision = transitive(dty, app(projection, h), d, app(projection, k), hd,
            symm(dty, app(projection, k), d, kd))
        agreement = app(var('constant'), h, k, r, kr, collision)
        if mutation == 'ignore-fibre-constancy': agreement = refl(qa)
        eq = transitive(aty, qa, app(query, k), var('other'), agreement, ka)
        only = bind(lam, [('other', aty), ('candidate', compatible(aty, query, d, var('other')))], eq)
        proof = pair(qa, pair(witness(h, r, hd, refl(qa)), only))
        target = unique(d)
    elif kind == 'unique_initialization_requires_reachable_fibre_constancy':
        params += [('initialize', initializer), ('a', hty), ('b', hty),
            ('ra', app(reach, a)), ('rb', app(reach, b)),
            ('collision', identity(dty, app(projection, a), app(projection, b)))]
        qa, qb = app(query, a), app(query, b)
        occupied = pair(a, pair(var('ra'), refl(app(projection, a))))
        initialized = app(var('initialize'), app(projection, a), occupied)
        answer, only = first(initialized), second(second(initialized))
        left = app(only, qa, witness(a, var('ra'), refl(app(projection, a)), refl(qa)))
        collision = symm(dty, app(projection, a), app(projection, b), var('collision'))
        if mutation == 'ignore-observation-collision': collision = refl(app(projection, b))
        right = app(only, qb, witness(b, var('rb'), collision, refl(qb)))
        proof = transitive(aty, qa, answer, qb, symm(aty, answer, qa, left), right)
        target = identity(aty, qa, qb)
    else:
        params += [('Refined', universe()), ('fine', arrow(hty, bty)), ('forget', arrow(bty, aty)),
            ('commutes', pi('h', hty, identity(aty, app(forget, app(fine, var('h'))), app(query, var('h'))))),
            ('d', dty)]
        if kind == 'compatible_family_projection':
            params += [('b', bty), ('candidate', compatible(bty, fine, d, b))]
            candidate = var('candidate')
            h, r = first(candidate), first(second(candidate))
            hd, hb = first(second(second(candidate))), second(second(second(candidate)))
            start, middle, end = app(query, h), app(forget, app(fine, h)), app(forget, b)
            first_path = symm(aty, middle, start, app(var('commutes'), h))
            second_path = congruence(bty, aty, forget, app(fine, h), b, hb)
            if mutation == 'discard-state-commutation': first_path = refl(start)
            proof = witness(h, r, hd, transitive(aty, start, middle, end, first_path, second_path))
            target = compatible(aty, query, d, end)
        elif kind == 'compatible_family_lift_keeps_history':
            params += [('a', aty), ('candidate', compatible(aty, query, d, a))]
            candidate = var('candidate')
            h, r = first(candidate), first(second(candidate))
            hd, ha = first(second(second(candidate))), second(second(second(candidate)))
            b = app(fine, h)
            path = transitive(aty, app(forget, b), app(query, h), a, app(var('commutes'), h), ha)
            if mutation == 'invent-refined-state-equality': path = refl(a)
            proof = pair(b, pair(witness(h, r, hd, refl(b)), path))
            target = sigma('refined_state', bty,
                sigma('compatible', compatible(bty, fine, d, var('refined_state')),
                    identity(aty, app(forget, var('refined_state')), a)))
        else:
            raise ValueError(kind)
    return {'term': lower(bind(lam, params, proof)), 'claimed_type': lower(bind(pi, params, target)),
        'metadata': {'theorem': kind, 'scope': 'reachable histories and exact compatible families; no default, representative choice or implementation correspondence'}}


def cases():
    result = obstruction_cases()
    for kind, mutation in [
        ('reachable_fibre_constancy_initializes_uniquely', 'ignore-fibre-constancy'),
        ('unique_initialization_requires_reachable_fibre_constancy', 'ignore-observation-collision'),
        ('compatible_family_projection', 'discard-state-commutation'),
        ('compatible_family_lift_keeps_history', 'invent-refined-state-equality'),
    ]:
        result.append((kind, reachable_certificate(kind), True))
        result.append((mutation, reachable_certificate(kind, mutation), False))
    return result
