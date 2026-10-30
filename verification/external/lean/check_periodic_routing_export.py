"""Check the observed Automatic routing/lease projection of native executions.

This is finite correspondence evidence, not a theorem about hidden execution,
incumbent retention, allocation, or the complete engine. Source/binary provenance
and an independent native replay are required separately.
"""
from fractions import Fraction


def require(condition, message):
    if not condition:
        raise ValueError(message)


def natural(value):
    require(type(value) is int or (type(value) is str and value.isascii()
            and value.isdecimal() and str(int(value)) == value), "noncanonical natural")
    result = int(value)
    require(result >= 0, "negative natural")
    return result


def check_incumbent(data):
    """Check only exported periodic boundaries, not the private VM transition.

    The native court separately asserts actual acquisition, source/floor ownership
    and absence of legacy acquisition. Those fields are not exported here and
    cannot be inferred from a cut's current value or a successful test name.
    """
    require(data['schema'] == 'synarc.native-routing-incumbent.v1', 'unsupported schema')
    initial, *boundaries = data['events']
    require(initial['kind'] == 'AcquiredIncumbent' and boundaries, 'missing acquired execution')
    window = natural(initial['window'])
    require(window > 0, 'zero physical window')
    require(natural(initial['candidate']) > 0 and natural(initial['look']) > 0,
            'missing acquisition identity')
    numerator, denominator = map(natural, initial['mass'])
    require(0 < numerator <= denominator, 'invalid acquired mass')
    ordinal, sitting = natural(initial['ordinal']), natural(initial['sitting'])
    cut = initial['cut']
    refusal = boundaries[0]['first_refused']
    require(natural(refusal['ordinal']) == ordinal, 'refusal moved beyond acquisition')
    probation = None
    for event in boundaries:
        require(event['kind'] == 'PhysicalPeriodicBoundary', 'foreign transition')
        ordinal += window
        sitting += 1
        require(natural(event['ordinal']) == ordinal and natural(event['sitting']) == sitting,
                'missing or duplicate physical boundary')
        require(event['halted'] is True and event['first_refused'] == refusal,
                'refusal changed or resumed')
        require(natural(event['fresh']) == 0 and natural(event['captured_receipts']) == 0,
                'boundary phase or acquisition lease changed')
        require(natural(event['requests']) == natural(initial['ordinal']), 'acquisition resumed')
        current = event['cut']
        require(all(current[key] == cut[key] for key in ('definition', 'owner')), 'cut owner replaced')
        require(natural(current['state']) == natural(cut['state']) + 1, 'missing native cut commit')
        remaining = natural(event['probation_left'])
        require(probation is None or remaining == max(0, probation - 1), 'probation lifecycle')
        require(bool(event['execution_window']) and bool(event['chronicle']), 'missing execution record')
        cut, probation = current, remaining
    return {'periodic_boundaries': len(boundaries), 'cut_commits': len(boundaries)}


def ledger(state):
    value = state['ledger']
    receipts = value['settled_receipts']
    require(value['poisoned'] is False, "poisoned ledger")
    require(natural(value['reserved']) == 0, "unsettled reservation")
    require(natural(value['requests']) == natural(value['settled_requests']) == len(receipts),
            "missing or extra lease receipt")
    spent = quoted = 0
    for index, receipt in enumerate(receipts, 1):
        require(receipt['lane'] == 'Automatic', "foreign acquisition lane")
        require(natural(receipt['request']) == index, "reordered lease")
        quote, allowed, work, returned, transferred = (
            natural(receipt[key]) for key in ('quote', 'allowed', 'spent', 'returned', 'transferred'))
        require(allowed <= quote and work + returned + transferred == allowed, "lease conservation")
        spent += work
        quoted += quote
    require(natural(value['spent']) == spent and natural(value['quoted']) == quoted, "ledger totals")
    require(natural(state['automatic_spent']) == spent, "Automatic work differs from actual leases")
    available, entitled = (Fraction(*(natural(n) for n in value[key]))
                           for key in ('available', 'entitled'))
    require(available + spent == entitled, "entitlement conservation")
    return receipts


def check(data):
    require(data['schema'] == 'synarc.native-periodic-routing.v1', "unsupported schema")
    require(bool(data['events']), "empty execution")
    previous = None
    physical = 0
    services = 0
    for event in data['events']:
        before, after = event['before'], event['after']
        require(previous is None or before == previous, "disconnected execution")
        for state in (before, after):
            require(state['exclusive'] is True, "nonexclusive owner")
            require(state['legacy_owner'] is False and state['legacy_pending'] is False,
                    "legacy acquisition owner")
            require(all(natural(state[key]) == 0 for key in ('floors', 'climbs', 'demolitions')),
                    "incumbent execution outside this projection")
        old, new = before['agenda'], after['agenda']
        require(old['source_owner'] == new['source_owner'], "replaced source owner")
        require(old['current']['identity'] == new['current']['identity'], "replaced pending owner")
        prior, receipts = ledger(before), ledger(after)
        require(receipts[:len(prior)] == prior, "rewritten settled lease")
        operation = event['operation']
        if operation == 'test_retention_limit':
            expected = dict(before, agenda=dict(old, capacity=natural(event['new'])))
            require(natural(event['old']) == old['capacity'] and after == expected,
                    "retention injection changed execution state")
        elif operation == 'advance_program_acquisition':
            require(event['published'] is False and not old['halted'] and not new['halted'],
                    "service outside paused projection")
            require(len(receipts) == len(prior) + 1, "service lease count")
            require(all(before[key] == after[key] for key in ('fresh', 'sittings', 'witnessed')),
                    "service invented physical action")
            require(new['current']['materializer'] is not None, "missing actual materializer")
            services += 1
        elif operation == 'step_forced':
            require(natural(event['physical_calls_before']) == physical, "physical journal gap")
            physical += 1
            require(natural(event['physical_calls_after']) == physical, "missing or duplicate action")
            require(natural(after['witnessed']) == natural(before['witnessed']) + 1, "missing observation")
            window = natural(before['window'])
            require(window > 0 and after['window'] == window, "invalid retention period")
            fresh = natural(before['fresh']) + 1
            require(natural(after['fresh']) == fresh % window and
                    natural(after['sittings']) == natural(before['sittings']) + fresh // window,
                    "periodic lifecycle mismatch")
            require(type(old['halted']) is bool and type(new['halted']) is bool, "invalid refusal flag")
            if new['halted']:
                require(new['first_unretained'] is not None and len(receipts) == len(prior),
                        "refusal acquired another lease")
                require(new['current'] == old['current'], "refusal replaced pending evidence")
            else:
                require(not old['halted'] and len(receipts) == len(prior) + 1,
                        "resumed refusal or periodic second lease")
            if old['halted']:
                require(new == old, "sticky refusal changed ownership or evidence")
            materializer = old['current']['materializer']
            if materializer is not None:
                for key in ('materializer', 'profile', 'snapshot', 'reserved_look', 'declared'):
                    require(new['current'][key] == old['current'][key], "paused materializer advanced without work")
        else:
            raise ValueError('unsupported operation')
        previous = after
    return {'physical_actions': physical, 'explicit_services': services,
            'periodic_windows': previous['sittings'], 'settled_leases': len(ledger(previous))}
