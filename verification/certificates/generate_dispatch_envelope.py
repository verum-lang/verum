"""Universal pure ownership-envelope laws; Rust affine usage is a separate premise."""
from portable_terms import (
    app, arrow, bind, construct, data, first, identity, induct, lam, lower,
    pair, pi, refl, second, sigma, universe, value_field, var,
)
from generate_dispatch_contract import BOOL, FALSE, TRUE, choose


def envelope(value_type):
    return sigma('receipt', var('Receipt'), sigma('value', value_type, BOOL))


def pack(receipt, value, admissible): return pair(receipt, pair(value, admissible))
def admission(value): return second(second(value))
def payload(value): return first(second(value))
def option(value_type): return data('Maybe', [[], [value_field(value_type)]])
def none(value_type): return construct(option(value_type), 0)
def result(ok_type, error_type): return data('Result', [[value_field(ok_type)], [value_field(error_type)]])


def mapped(source, mutation=None):
    return pack(var('otherReceipt') if mutation == 'receipt-swap' else first(source),
        app(var('transform'), payload(source)),
        TRUE if mutation == 'map-reenable' else admission(source))


def assessed(source, mutation=None):
    return choose(TRUE if mutation == 'assess-after-failure' else admission(source),
        option(var('Evidence')), none(var('Evidence')),
        app(var('evaluate'), var('otherValue') if mutation == 'observation-swap' else payload(source)))


def interpreted(receipt, admissible, answer, mutation=None):
    target = result(envelope(var('U')), envelope(var('Error')))
    return induct(answer, lam('interpret_result', result(var('U'), var('Error')), target),
        lam('interpreted_value', var('U'), construct(target, 0, pack(receipt, var('interpreted_value'), admissible))),
        lam('interpret_error', var('Error'), construct(target, 1, pack(receipt, var('interpret_error'),
            TRUE if mutation == 'try-map-reenable' else FALSE))))


def inspect_result(answer, inspect, target):
    return induct(answer, lam('inspect_result', result(envelope(var('U')), envelope(var('Error'))), target),
        lam('ok_envelope', envelope(var('U')), inspect(var('ok_envelope'))),
        lam('error_envelope', envelope(var('Error')), inspect(var('error_envelope'))))


def certificate(kind, mutation=None):
    params = [(name, universe()) for name in ('Receipt', 'T', 'U', 'Error', 'Evidence')]
    params += [('receipt', var('Receipt')), ('otherReceipt', var('Receipt')),
        ('value', var('T')), ('otherValue', var('U')), ('admissible', BOOL),
        ('transform', arrow(var('T'), var('U'))),
        ('evaluate', arrow(var('U'), option(var('Evidence')))),
        ('answer', result(var('U'), var('Error')))]
    source = pack(var('receipt'), var('value'), var('admissible'))
    if kind == 'map_preserves_receipt':
        target = identity(var('Receipt'), first(mapped(source, mutation)), var('receipt'))
        proof = refl(var('receipt'))
    elif kind == 'map_preserves_admission':
        target = identity(BOOL, admission(mapped(source, mutation)), var('admissible'))
        proof = refl(var('admissible'))
    elif kind == 'failed_map_has_no_assessment':
        failed = pack(var('receipt'), var('value'), FALSE)
        actual = assessed(mapped(failed, mutation), mutation)
        target = identity(option(var('Evidence')), actual, none(var('Evidence')))
        proof = refl(none(var('Evidence')))
    elif kind == 'assessment_uses_owned_value':
        source = pack(var('receipt'), var('value'), TRUE)
        actual = assessed(mapped(source), mutation)
        expected = app(var('evaluate'), app(var('transform'), var('value')))
        target = identity(option(var('Evidence')), actual, expected)
        proof = refl(expected)
    elif kind in ('failed_interpretation_remains_inadmissible', 'interpretation_preserves_receipt'):
        if kind.startswith('failed'):
            flag, inspect, carrier, expected = FALSE, admission, BOOL, FALSE
        else:
            flag, inspect, carrier, expected = var('admissible'), first, var('Receipt'), var('receipt')
        def goal(answer):
            changed = interpreted(var('receipt'), flag, answer, mutation)
            return identity(carrier, inspect_result(changed, inspect, carrier), expected)
        target = goal(var('answer'))
        proof = induct(var('answer'), lam('answer_case', result(var('U'), var('Error')), goal(var('answer_case'))),
            lam('ok_value', var('U'), refl(expected)), lam('error_value', var('Error'), refl(expected)))
    else:
        raise ValueError(kind)
    return {'term': lower(bind(lam, params, proof)), 'claimed_type': lower(bind(pi, params, target)),
        'metadata': {'theorem': kind, 'scope': 'pure Dispatch map/try_map/assessment; arbitrary types and interpreters; no affine or Rust refinement claim'}}


def cases():
    kinds = ['map_preserves_receipt', 'map_preserves_admission', 'failed_map_has_no_assessment',
        'assessment_uses_owned_value', 'failed_interpretation_remains_inadmissible', 'interpretation_preserves_receipt']
    cases = [(kind, certificate(kind), True) for kind in kinds]
    for kind, mutation in [('map_preserves_receipt', 'receipt-swap'),
        ('failed_map_has_no_assessment', 'map-reenable'),
        ('failed_map_has_no_assessment', 'assess-after-failure'),
        ('assessment_uses_owned_value', 'observation-swap'),
        ('failed_interpretation_remains_inadmissible', 'try-map-reenable')]:
        cases.append((mutation, certificate(kind, mutation), False))
    return cases
