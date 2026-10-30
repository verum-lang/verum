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
