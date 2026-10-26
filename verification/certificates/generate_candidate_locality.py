"""Quantified witness-preserving locality, not a statistical-confidence theorem."""
from portable_terms import (
    app, arrow, bind, congruence, factor, first, identity, lam, local, lower,
    pair, pi, refl, second, sigma, universe, var,
)


def product(types):
    result = types[-1]
    for ty in reversed(types[:-1]): result = sigma('_component', ty, result)
    return result


def tuple_term(values):
    result = values[-1]
    for value in reversed(values[:-1]): result = pair(value, result)
    return result


def field(value, index, length):
    for _ in range(index): value = second(value)
    return value if index == length - 1 else first(value)


def predicate(): return arrow(var('Situation'), universe())
def inhabited(domain): return sigma('inhabitant', var('Situation'), app(domain, var('inhabitant')))
def subset(domain, target):
    return pi('s', var('Situation'), arrow(app(domain, var('s')), app(target, var('s'))))
def witness(domain, support):
    return sigma('s', var('Situation'), sigma('member', app(domain, var('s')), app(support, var('s'))))


def admitted(domain, prefix):
    return product([var('ValidRung'), inhabited(domain), subset(domain, var(prefix+'Universe')),
        subset(domain, var(prefix+'Count')), witness(domain, var(prefix+'Time')), witness(domain, var(prefix+'Event'))])


def preserved(old, new):
    return bind(pi, [('s', var('Situation')), ('member', app(var('D'), var('s'))),
        ('evidence', app(old, var('s')))], app(new, var('s')))


def map_points(kind, old):
    return bind(lam, [('s', var('Situation')), ('member', app(var('D'), var('s')))],
        app(var('preserve'+kind), var('s'), var('member'), app(old, var('s'), var('member'))))


def map_witness(kind, old):
    point, rest = first(old), second(old)
    return pair(point, pair(first(rest), app(var('preserve'+kind), point, first(rest), second(rest))))


def maintained(mutation=None):
    old = var('admission')
    components = [field(old, 0, 6), field(old, 1, 6),
        map_points('Universe', field(old, 2, 6)), map_points('Count', field(old, 3, 6)),
        map_witness('Time', field(old, 4, 6)), map_witness('Event', field(old, 5, 6))]
    if mutation == 'unpreserved-count': components[3] = field(old, 3, 6)
    if mutation == 'unpreserved-membership': components[2] = field(old, 2, 6)
    if mutation == 'exchanged-axis-witness': components[4] = map_witness('Time', field(old, 5, 6))
    if mutation == 'support-is-not-domain-membership':
        nonempty = field(old, 1, 6)
        components[1] = pair(first(nonempty), app(field(old, 3, 6), first(nonempty), second(nonempty)))
    if mutation == 'count-as-membership': components[2] = components[3]
    return tuple_term(components)


def eligible(s):
    def extent(kind):
        return sigma('extent_point', var('Situation'), product([
            app(var('newUniverse'), var('extent_point')),
            app(var('newCount'), var('extent_point')),
            app(var('new'+kind), var('extent_point'))]))
    return product([var('ValidRung'), app(var('newUniverse'), s), app(var('newCount'), s),
        extent('Time'), extent('Event')])


def eligible_proof():
    admission = var('maintained')
    inclusion, count = field(admission, 2, 6), field(admission, 3, 6)
    def extent(index):
        proof = field(admission, index, 6)
        point, member = first(proof), first(second(proof))
        return pair(point, tuple_term([app(inclusion, point, member), app(count, point, member), second(second(proof))]))
    return bind(lam, [('s', var('Situation')), ('member', app(var('D'), var('s')))], tuple_term([
        field(admission, 0, 6), app(inclusion, var('s'), var('member')),
        app(count, var('s'), var('member')), extent(4), extent(5)]))


def base_parameters():
    return [('Situation', universe()), ('ValidRung', universe())] + [
        (name, predicate()) for name in ('D', 'oldUniverse', 'newUniverse',
            'oldCount', 'newCount', 'oldTime', 'newTime', 'oldEvent', 'newEvent')]


def certificate(kind, mutation=None):
    if kind == 'same_domain_identity_comparison':
        params = [('Situation', universe()), ('D', predicate()), ('Other', predicate()),
            ('Score', universe()), ('identityScore', arrow(predicate(), var('Score'))),
            ('sameDomain', identity(predicate(), var('D'), var('Other')))]
        target = identity(var('Score'), app(var('identityScore'), var('D')), app(var('identityScore'), var('Other')))
        proof = refl(app(var('identityScore'), var('D'))) if mutation else congruence(
            predicate(), var('Score'), var('identityScore'), var('D'), var('Other'), var('sameDomain'))
        return encode(bind(lam, params, proof), bind(pi, params, target), kind)
    if kind == 'selected_eligible_domain_stays_in_full_universe':
        params = [('Situation', universe()), ('ValidRung', universe())] + [
            (name, predicate()) for name in ('newUniverse', 'newCount', 'newTime', 'newEvent', 'Selected')]
        eligible_set = lam('eligible_s', var('Situation'), eligible(var('eligible_s')))
        premises = [('selectedIsEligible', subset(var('Selected'), eligible_set))]
        target = subset(var('Selected'), var('newUniverse'))
        evidence = app(var('selectedIsEligible'), var('s'), var('selected_member'))
        proof = bind(lam, [('s', var('Situation')), ('selected_member', app(var('Selected'), var('s')))],
            field(evidence, 2 if mutation == 'count-is-not-universe-membership' else 1, 5))
        return encode(bind(lam, params + premises, proof), bind(pi, params + premises, target), kind)
    params = base_parameters()
    aliases = [('OldAdmission', admitted(var('D'), 'old')),
        ('NewAdmission', admitted(var('D'), 'new')),
        ('Eligible', lam('eligible_s', var('Situation'), eligible(var('eligible_s'))))]
    premises = [('admission', var('OldAdmission'))] + [
        ('preserve'+kind, preserved(var('old'+kind), var('new'+kind))) for kind in ('Universe', 'Count', 'Time', 'Event')]
    definitions = [('maintained', maintained(mutation))]
    if kind == 'same_domain_remains_admitted':
        target, proof = var('NewAdmission'), var('maintained')
    elif kind == 'an_admitted_domain_still_exists':
        target = sigma('preserved_domain', predicate(), admitted(var('preserved_domain'), 'new'))
        proof = pair(var('D'), var('maintained'))
    elif kind == 'old_domain_belongs_to_extended_eligible':
        target, proof = subset(var('D'), var('Eligible')), eligible_proof()
    else:
        raise ValueError(kind)
    for name, value in reversed(definitions):
        proof = local(name, value, proof)
    proof, target = bind(lam, premises, proof), bind(pi, premises, target)
    proof, target = factor(proof, aliases), factor(target, aliases)
    for index in reversed(range(len(aliases))):
        name, value = aliases[index]
        value = factor(value, aliases[:index])
        proof, target = local(name, value, proof), local(name, value, target)
    return encode(bind(lam, params, proof), bind(pi, params, target), kind)


def encode(proof, target, name):
    return {'term': lower(proof), 'claimed_type': lower(target), 'metadata': {'theorem': name,
        'scope': 'arbitrary predicates and witness-preserving extension at one fixed valid rung; no purchase, confidence or Rust refinement claim'}}


def cases():
    kinds = ['same_domain_remains_admitted', 'an_admitted_domain_still_exists',
        'old_domain_belongs_to_extended_eligible', 'selected_eligible_domain_stays_in_full_universe',
        'same_domain_identity_comparison']
    result = [(kind, certificate(kind), True) for kind in kinds]
    for mutation in ('unpreserved-count', 'unpreserved-membership', 'exchanged-axis-witness',
            'support-is-not-domain-membership', 'count-as-membership'):
        result.append((mutation, certificate('same_domain_remains_admitted', mutation), False))
    result.append(('count-is-not-universe-membership', certificate('selected_eligible_domain_stays_in_full_universe', 'count-is-not-universe-membership'), False))
    result.append(('different-domain-comparison', certificate('same_domain_identity_comparison', 'drop-domain-equality'), False))
    return result
