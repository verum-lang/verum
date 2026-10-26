"""Compositional exact one-step transports; no acquisition or implementation claim."""
from portable_terms import (
    app, arrow, bind, congruence, construct, data, identity, induct, lam, lower,
    pi, refl, transitive, universe, value_field, var,
)


def option(carrier): return data('Option', [[], [value_field(carrier)]])
def none(carrier): return construct(option(carrier), 0)
def some(carrier, value): return construct(option(carrier), 1, value)
def option_map(source, target, function, value):
    return induct(value, lam('option_subject', option(source), option(target)), none(target),
        lam('option_value', source, some(target, app(function, var('option_value')))))


def composed():
    return lam('composition_input', var('A'), app(var('j'), app(var('i'), var('composition_input'))))


def map_i(value): return option_map(var('A'), var('B'), var('i'), value)
def map_j(value): return option_map(var('B'), var('C'), var('j'), value)
def map_ji(value): return option_map(var('A'), var('C'), composed(), value)
def fusion_goal(value): return identity(option(var('C')), map_j(map_i(value)), map_ji(value))
def fusion(value, mutation=None):
    branch = some(var('C'), app(var('j'), app(var('i'), var('fusion_value'))))
    if mutation == 'omit-second-embedding': branch = some(var('B'), app(var('i'), var('fusion_value')))
    return induct(value, lam('fusion_subject', option(var('A')), fusion_goal(var('fusion_subject'))),
        refl(none(var('C'))), lam('fusion_value', var('A'), refl(branch)))


FLAG = data('Definedness', [[], []])
def flag(defined): return construct(FLAG, int(defined))
def definedness(carrier, value):
    return induct(value, lam('defined_subject', option(carrier), FLAG), flag(False),
        lam('defined_value', carrier, flag(True)))
def defined_map_goal(value):
    return identity(FLAG, definedness(var('B'), map_i(value)), definedness(var('A'), value))
def defined_map(value, mutation=None):
    return induct(value, lam('status_subject', option(var('A')), defined_map_goal(var('status_subject'))),
        refl(flag(mutation == 'assume-every-result-defined')),
        lam('status_value', var('A'), refl(flag(True))))


def embedding_parameters():
    return [('A', universe()), ('B', universe()), ('C', universe()),
        ('i', arrow(var('A'), var('B'))), ('j', arrow(var('B'), var('C')))]
def exact(source, target, embedding, before, after):
    return pi('state', source, identity(option(target), app(after, app(embedding, var('state'))),
        option_map(source, target, embedding, app(before, var('state')))))
def injective(source, target, function):
    return bind(pi, [('left', source), ('right', source)],
        arrow(identity(target, app(function, var('left')), app(function, var('right'))),
            identity(source, var('left'), var('right'))))


def certificate(kind, mutation=None):
    a, b, c, i, j = (var(name) for name in ('A', 'B', 'C', 'i', 'j'))
    params = embedding_parameters()
    if kind == 'option_map_composition':
        params += [('value', option(a))]
        target, proof = fusion_goal(var('value')), fusion(var('value'), mutation)
    elif kind == 'injective_embeddings_compose':
        params += [('injectiveI', injective(a, b, i)), ('injectiveJ', injective(b, c, j)),
            ('left', a), ('right', a), ('equal', identity(c,
                app(j, app(i, var('left'))), app(j, app(i, var('right')))))]
        recovered = app(var('injectiveJ'), app(i, var('left')), app(i, var('right')), var('equal'))
        if mutation == 'ignore-second-injectivity': recovered = var('equal')
        left, right = var('left'), var('right')
        if mutation == 'reverse-recovery': left, right = right, left
        proof = app(var('injectiveI'), left, right, recovered)
        target = identity(a, var('left'), var('right'))
    else:
        if kind == 'definedness_reflected':
            params = [('A', universe()), ('B', universe()), ('i', arrow(a, b))]
        params += [('f', arrow(a, option(a))), ('g', arrow(b, option(b))),
            ('exactI', exact(a, b, i, var('f'), var('g')))]
        if kind == 'exact_transports_compose':
            params += [('h', arrow(c, option(c))), ('exactJ', exact(b, c, j, var('g'), var('h')))]
        params += [('x', a)]
        fx, gix = app(var('f'), var('x')), app(var('g'), app(i, var('x')))
        if kind == 'definedness_reflected':
            left, middle, right = definedness(b, gix), definedness(b, map_i(fx)), definedness(a, fx)
            step = congruence(option(b), FLAG, lam('status_input', option(b), definedness(b, var('status_input'))),
                gix, map_i(fx), app(var('exactI'), var('x')))
            proof = transitive(FLAG, left, middle, right, step, defined_map(fx, mutation))
            if mutation == 'ignore-definedness-transport': proof = refl(right)
            target = identity(FLAG, left, right)
        elif kind == 'exact_transports_compose':
            start = app(var('h'), app(j, app(i, var('x'))))
            first = app(var('exactJ'), app(i, var('x')))
            second = congruence(option(b), option(c), lam('transport_input', option(b), map_j(var('transport_input'))),
                gix, map_i(fx), app(var('exactI'), var('x')))
            if mutation == 'omit-first-transport': second = refl(map_j(map_i(fx)))
            if mutation == 'omit-second-transport': first = refl(start)
            proof = transitive(option(c), start, map_j(map_i(fx)), map_ji(fx),
                transitive(option(c), start, map_j(gix), map_j(map_i(fx)), first, second), fusion(fx))
            target = identity(option(c), start, map_ji(fx))
        else:
            raise ValueError(kind)
    return {'term': lower(bind(lam, params, proof)), 'claimed_type': lower(bind(pi, params, target)),
        'metadata': {'theorem': kind, 'scope': 'supplied exact one-step Option contracts; no grammar, learner, Rust or empirical-generalization claim'}}


def cases():
    controls = {
        'option_map_composition': ['omit-second-embedding'],
        'exact_transports_compose': ['omit-first-transport', 'omit-second-transport'],
        'definedness_reflected': ['assume-every-result-defined', 'ignore-definedness-transport'],
        'injective_embeddings_compose': ['ignore-second-injectivity', 'reverse-recovery'],
    }
    result = []
    for name, mutations in controls.items():
        positive = certificate(name)
        result.append((name, positive, True))
        for mutation in mutations:
            altered = certificate(name, mutation)
            assert altered['claimed_type'] == positive['claimed_type']
            result.append((mutation, altered, False))
    return result
