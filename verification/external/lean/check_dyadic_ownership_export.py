#!/usr/bin/env python3
"""Check exported dyadic ownership projections, not hidden VM or authority state."""
import json
import re
from pathlib import Path
from observer_export_replay import (array, boolean, fields, natural, require, sequence,
    unique_object, main as replay_main)

MODELS = ['RectangularNames', 'AdditiveTraversal']
SCOPE = ('Finite exported dyadic naming and issued-owner conservation. V1 covers initializer '
         'equivalence before promotion, projected pause and Unknown transfer. V2 additionally '
         'checks retained source/search/look identities, initializer promotion, typed stops, '
         'capacity changes and denied retries. Candidate grammar/provenance authority, look '
         'allocation/payment and private invocation receipts remain unexported; no full VM, '
         'publication authority or cost refinement.')


def load_export(path):
    # This schema repeats complete before/after rosters. Bound it separately
    # from the shared compact-export format; never change that format's limit.
    require(path.stat().st_size <= 64 * 1024 * 1024, 'Dyadic export exceeds 64 MiB')
    def invalid(value):
        raise ValueError('Non-JSON constant: ' + value)
    return json.loads(path.read_text(), object_pairs_hook=unique_object, parse_constant=invalid)


def emit(data):
    fields(data, 'schema identity_policy scope semantics support_sizes events')
    require(data['schema'] in ('synarc-dyadic-cumulative-ownership-v1',
                              'synarc-dyadic-cumulative-ownership-v2'), 'Unknown dyadic ownership schema')
    rich = data['schema'] == 'synarc-dyadic-cumulative-ownership-v2'
    for key in ('identity_policy', 'scope'):
        require(type(data[key]) is str, 'Expected scope text')
    require(data['semantics'] == {'Identity': 'arity1 returns sole argument',
        'Zero': 'arity0 returns0', 'otherwise': 'undefined'}, 'Unknown primitive interpretation')
    sizes = sequence(data['support_sizes'])
    require(len(sizes) == 3, 'Expected three support sizes')
    for size in sizes: natural(size)
    total = sizes[0] * sizes[1] * sizes[2]
    require(total <= 16384, 'Finite carrier exceeds replay bound')
    rows = sequence(data['events'])
    require(0 < len(rows) <= 16384, 'Invalid event count')
    lines = ['import VerumExternalReplay.RectangularNames\n'
             'import VerumExternalReplay.AdditiveTraversal\nopen RectangularNames\n'
             'set_option warningAsError true\nnamespace FrozenObserverExport']
    claims, terms, checked = {}, {}, set()

    def claim(expression, proof='decide_cbv'):
        if expression not in claims:
            name = 'c' + str(len(claims)); claims[expression] = name
            lines.append(f'theorem {name} : {expression} := by {proof}')

    def term(expression, type_):
        key = expression, type_
        if key not in terms:
            name = 'd' + str(len(terms)); terms[key] = name
            lines.append(f'def {name} : {type_} := {expression}')
        return terms[key]

    def wide(value):
        require(type(value) is str and re.fullmatch(r'0|[1-9][0-9]*', value) is not None
                and len(value) <= 128, 'Invalid canonical natural')
        return value

    def eq(a, b, type_='Nat'):
        claim(f'({a} : {type_}) = {b}')

    def reason(r):
        require(type(r) is dict and type(r.get('refusal')) is dict, 'Expected refusal')
        typed = rich and r['refusal'].get('kind') == 'Materialization'
        fields(r, 'can_resume refusal work_shortage' + (' initial' if typed else ''))
        boolean(r['can_resume']); boolean(r['work_shortage'])
        f = r['refusal']; require(type(f) is dict, 'Expected refusal')
        require(f.get('kind') in ('Materialization', 'Work', 'Capacity'), 'Unknown refusal kind')
        fields(f, 'kind' if f['kind'] == 'Capacity' else 'kind detail')
        if 'detail' in f: require(type(f['detail']) is str, 'Expected opaque diagnostic')
        if rich:
            resumable, shortage = True, f['kind'] != 'Capacity'
            if typed:
                initial = r['initial']; require(type(initial) is dict, 'Missing typed initial stop')
                kind = initial.get('kind')
                require(kind in ('Suspended', 'Work'), 'Unsupported initial stop')
                fields(initial, 'kind phase dispatches reason meter_error' if kind == 'Suspended'
                       else 'kind phase execution_present')
                require(initial['phase'] == 'Update', 'Unsupported initial phase')
                if kind == 'Suspended':
                    natural(initial['dispatches']); boolean(initial['meter_error'])
                    require(initial['reason'] in ('Work', 'OriginalDispatch'), 'Unsupported suspension')
                    resumable = shortage = initial['reason'] == 'Work'
                    eq(boolean(initial['meter_error']), boolean(shortage), 'Bool')
                else:
                    boolean(initial['execution_present'])
                    resumable = not initial['execution_present']
            eq(boolean(r['can_resume']), boolean(resumable), 'Bool')
            eq(boolean(r['work_shortage']), boolean(shortage), 'Bool')
        # Debug details are retained literally, never parsed as VM semantics.
        return json.dumps(json.dumps(r, sort_keys=True), ensure_ascii=False)

    shape = '⟨' + ', '.join(map(str, sizes)) + '⟩'
    candidate_fields = 'original_name coordinates look family_denominator'
    materializer_fields = candidate_fields + (' programs_ready encoded_words initializer_owner '
        'invocation_retained evaluated_retained provenance_name poisoned')
    if rich:
        materializer_fields += ' source_owner paid_look'
        candidate_fields += ' initializer_owner paid_look'

    def look(p):
        fields(p, 'authority index denominator')
        natural(p['authority']); wide(p['index']); wide(p['denominator'])
        claim(f'(0 : Nat) < {p["denominator"]}')

    def same_look(a, b):
        for k in ('index', 'denominator'): eq(wide(a[k]), wide(b[k]))
        eq(natural(a['authority']), natural(b['authority']))

    def owner(o, materializer=False):
        fields(o, materializer_fields if materializer else candidate_fields)
        name = wide(o['original_name']); wide(o['look']); denominator = wide(o['family_denominator'])
        xyz = sequence(o['coordinates']); require(len(xyz) == 3, 'Expected three coordinates')
        for v in xyz: natural(v)
        triple = '⟨' + ', '.join(map(str, xyz)) + '⟩'
        claim(f'Valid {shape} {triple}')
        eq(f'rank {shape} {triple}', name)
        eq(denominator, str(total))
        if rich:
            look(o['paid_look']); eq(wide(o['look']), wide(o['paid_look']['index']))
            if materializer: natural(o['source_owner'])
            else: natural(o['initializer_owner'])
        if materializer:
            flags = sequence(o['programs_ready']); words = sequence(o['encoded_words'])
            require(len(flags) == len(words) == 3, 'Expected three program projections')
            for flag in flags: boolean(flag)
            for word in words:
                if word is not None:
                    for bit in sequence(word): natural(bit, 1)
            if o['initializer_owner'] is not None: natural(o['initializer_owner'])
            for k in ('invocation_retained', 'evaluated_retained', 'poisoned'): boolean(o[k])
            if o['invocation_retained'] or o['evaluated_retained']:
                require(o['initializer_owner'] is not None, 'Retained invocation lacks initializer identity')
            eq(boolean(o['poisoned']), 'false', 'Bool')
            if o['provenance_name'] is not None: eq(wide(o['provenance_name']), name)
        return name

    def snapshot(s):
        fields(s, 'enumeration materializing materialized unknown members' +
               (' owner source_owner paid_look capacity materializer_stop' if rich else ''))
        encoded = json.dumps(s, sort_keys=True, separators=(',', ':'))
        if encoded in checked: return
        checked.add(encoded)
        e = s['enumeration']; fields(e, 'service_order issued_names pending_original_name complete')
        require(e['service_order'] == 'finite-structural-dyadic-service-v1', 'Unknown service order')
        issued = wide(e['issued_names']); boolean(e['complete'])
        claim(f'({issued} : Nat) ≤ {total}')
        names = []
        if s['materializing'] is not None: names.append(owner(s['materializing'], True))
        if s['materialized'] is not None: names.append(owner(s['materialized']))
        claim(f'({int(s["materializing"] is not None) + int(s["materialized"] is not None)} : Nat) ≤ 1')
        for u in sequence(s['unknown']):
            fields(u, 'owner reason'); names.append(owner(u['owner'], True)); reason(u['reason'])
        for c in sequence(s['members']): names.append(owner(c))
        owned = term(array(names), 'List Nat')
        eq(f'{owned}.length', issued)
        claim(f'{owned}.Nodup')
        if e['pending_original_name'] is not None:
            pending = wide(e['pending_original_name'])
            claim(f'({pending} : Nat) < {total}')
            claim(f'({pending} : Nat) ∉ {owned}')
        if e['complete']:
            eq(issued, str(total))
            require(e['pending_original_name'] is None, 'Completed enumeration retains pending name')
        if rich:
            natural(s['owner']); natural(s['source_owner']); look(s['paid_look'])
            cap = s['capacity']; fields(cap, 'address_cells members rows')
            for v in cap.values(): natural(v)
            claim(f'({issued} : Nat) ≤ {cap["members"]}')
            for o in projected_owners(s):
                same_look(s['paid_look'], o['paid_look'])
                if 'source_owner' in o: eq(natural(s['source_owner']), natural(o['source_owner']))
            if s['materializer_stop'] is not None:
                require(s['materializing'] is not None, 'Stop lacks retained materializer')
                reason(s['materializer_stop'])

    def candidate_equal(a, b):
        for k in ('original_name', 'look', 'family_denominator'): eq(wide(a[k]), wide(b[k]))
        eq(array(map(str, a['coordinates'])), array(map(str, b['coordinates'])), 'List Nat')
        if rich:
            same_look(a['paid_look'], b['paid_look'])
            if a['initializer_owner'] is not None and b['initializer_owner'] is not None:
                eq(natural(a['initializer_owner']), natural(b['initializer_owner']))
            if 'source_owner' in a and 'source_owner' in b:
                eq(natural(a['source_owner']), natural(b['source_owner']))

    def projected_owners(s):
        return ([s[k] for k in ('materializing', 'materialized') if s[k] is not None]
                + s['members'] + [u['owner'] for u in s['unknown']])

    horizon = sum(max(0, size - 1).bit_length() for size in sizes)
    order = term('AdditiveTraversal.through ' + ' '.join(
        f'(AdditiveTraversal.boundedOrdinals {s})' for s in sizes) + f' {horizon}',
        'List (Nat × Nat × Nat)')
    previous, segments, issued_coordinates, issued_owners, owner_rosters = None, 0, [], {}, {}
    initializers = {}
    for row in rows:
        operation = row.get('operation')
        require(operation in (('advance', 'extend_capacity', 'retry_materialization') if rich else
                              ('advance', 'extend_capacity')), 'Unknown operation')
        fields(row, {'advance':'case operation before after grant spent remaining dispatch_limit returned',
            'extend_capacity':'case operation before after members rows',
            'retry_materialization':'case operation before after slot accepted'}[operation])
        require(type(row['case']) is str and row['case'], 'Missing case name')
        before, after = row['before'], row['after']; snapshot(before); snapshot(after)
        if previous is None or (row['case'] != previous['case'] and before != previous['after']):
            # The old schema has no instance IDs. New fixture segments must be
            # visibly empty; this is not a proof that source authority is equal.
            require(before['enumeration']['issued_names'] == '0' and not before['members']
                    and not before['unknown'] and before['materializing'] is None
                    and before['materialized'] is None, 'Unrecorded state replacement')
            segments += 1
            issued_coordinates = []
            issued_owners = {}
            initializers = {}
            if rich:
                require(before['owner'] not in owner_rosters, 'Reused search owner at new segment')
                owner_rosters[before['owner']] = before
        else:
            require(before == previous['after'], 'Discontinuous snapshot chain')
        if operation == 'extend_capacity':
            natural(row['members']); natural(row['rows'])
            if rich:
                require({k:v for k,v in before.items() if k != 'capacity'} ==
                        {k:v for k,v in after.items() if k != 'capacity'}, 'Capacity extension changed owned projection')
                eq(natural(after['capacity']['members']),
                   f'max {natural(before["capacity"]["members"])} {natural(row["members"])}')
                eq(natural(after['capacity']['rows']),
                   f'max {natural(before["capacity"]["rows"])} {natural(row["rows"])}')
                eq(natural(after['capacity']['address_cells']), natural(before['capacity']['address_cells']))
            else: require(before == after, 'Capacity extension changed owned projection')
        elif operation == 'retry_materialization':
            slot = int(natural(row['slot'])); boolean(row['accepted'])
            require(before == after, 'Denied retry changed retained state')
            busy = any(before[k] is not None for k in ('materializing', 'materialized', 'materializer_stop'))
            if not busy and slot < len(before['unknown']):
                eq(boolean(before['unknown'][slot]['reason']['can_resume']), 'false', 'Bool')
            eq(boolean(row['accepted']), 'false', 'Bool')
        else:
            grant, spent, remaining = (int(natural(row[k], 1)) for k in ('grant', 'spent', 'remaining'))
            eq(str(spent + remaining), str(grant))
            if row['dispatch_limit'] is not None: natural(row['dispatch_limit'])
            returned = row['returned']; require(type(returned) is dict, 'Expected return')
            kind = returned.get('kind'); require(kind in ('Advanced', 'Stopped'), 'Unsupported progress projection')
            fields(returned, 'kind index reason' if kind == 'Stopped' else 'kind')
            if kind == 'Stopped':
                reason(returned['reason'])
                if rich and returned['reason']['refusal']['kind'] == 'Materialization':
                    if returned['reason']['work_shortage']:
                        require(after['materializer_stop'] is None, 'Work shortage became parked stop')
                    else:
                        require(after['materializer_stop'] is not None, 'Lost active materialization stop')
                        eq(reason(after['materializer_stop']), reason(returned['reason']), 'String')
                active = after['materializing'] or after['materialized']
                expected = 'none' if active is None else 'some ' + wide(active['original_name'])
                actual = 'none' if returned['index'] is None else 'some ' + wide(returned['index'])
                eq(actual, expected, 'Option Nat')
                if returned['index'] is not None:
                    index = wide(returned['index'])
                    retained = projected_owners(after)
                    names = array(wide(o['original_name']) for o in retained)
                    claim(f'({index} : Nat) ∈ ({names} : List Nat)')
            if grant == 0: require(before == after, 'Zero-work changed owned projection')
            require(before['materialized'] is None or after['materializing'] is None,
                    'Materialized candidate rolled back to materializer')
            if rich and before['materializer_stop'] is not None:
                retained = (after['materializing'] == before['materializing'] and
                            after['materializer_stop'] == before['materializer_stop'])
                transferred = (after['materializing'] is None and after['materializer_stop'] is None and
                    len(after['unknown']) == len(before['unknown']) + 1 and
                    after['unknown'][-1] == {'owner': before['materializing'],
                                             'reason': before['materializer_stop']})
                require(retained or transferred, 'Parked owner/stop pair disappeared')
            issued0 = wide(before['enumeration']['issued_names']); issued1 = wide(after['enumeration']['issued_names'])
            claim(f'({issued0} : Nat) ≤ {issued1}')
            claim(f'({issued1} : Nat) ≤ {issued0} + {spent}')
            pending0 = before['enumeration']['pending_original_name']
            pending1 = after['enumeration']['pending_original_name']
            if pending0 is not None and pending1 is not None: eq(wide(pending0), wide(pending1))
            if int(issued1) > int(issued0):
                require(after['materializing'] is not None, 'Issued name lacks materializer')
                require(pending0 is not None, 'Issued name lacks pending selection')
                eq(wide(pending0), wide(after['materializing']['original_name']))
                issued_coordinates.append(after['materializing']['coordinates'])
                selected = after['materializing']
                require(selected['original_name'] not in issued_owners, 'Reissued original name')
                issued_owners[selected['original_name']] = selected
                coordinates = array('(' + ', '.join(map(str, xyz)) + ')' for xyz in issued_coordinates)
                eq(f'{order}.take {issued1}', coordinates, 'List (Nat × Nat × Nat)')
            for k in ('members', 'unknown'):
                require(after[k][:len(before[k])] == before[k], 'Lost or changed retained ' + k)
                claim(f'({len(after[k])} : Nat) ≤ {len(before[k])} + 1')
            a, b = before['materializing'], after['materializing']
            if a is not None and b is not None:
                candidate_equal(a, b)
                if a['initializer_owner'] is not None:
                    require(b['initializer_owner'] is not None, 'Lost initializer owner')
                    eq(str(a['initializer_owner']), str(b['initializer_owner']))
                for x, y in zip(a['encoded_words'], b['encoded_words']):
                    if x is not None: require(x == y, 'Changed retained syntax')
                if a['invocation_retained']: eq(boolean(b['invocation_retained']), 'true', 'Bool')
            if after['materialized'] is not None and before['materialized'] is None:
                require(a is not None, 'Candidate appeared without materializer')
                eq(boolean(a['evaluated_retained']), 'true', 'Bool')
                candidate_equal(a, after['materialized'])
            if after['materialized'] is not None and before['materialized'] is not None:
                candidate_equal(before['materialized'], after['materialized'])
            if len(after['members']) > len(before['members']):
                require(before['materialized'] is not None, 'Member appeared without candidate')
                candidate_equal(before['materialized'], after['members'][-1])
            if len(after['unknown']) > len(before['unknown']):
                require(a is not None and previous is not None, 'Unknown appeared without materializer')
                require(a == after['unknown'][-1]['owner'], 'Unknown lost retained work')
                if rich:
                    require(before['materializer_stop'] is not None, 'Unknown lacks retained stop')
                    stop_reason = before['materializer_stop']
                else:
                    stop = previous.get('returned', {})
                    require(stop.get('kind') == 'Stopped', 'Unknown lacks preceding refusal')
                    stop_reason = stop['reason']
                eq(reason(stop_reason), reason(after['unknown'][-1]['reason']), 'String')
        if rich:
            for key in ('owner', 'source_owner'): eq(natural(before[key]), natural(after[key]))
            same_look(before['paid_look'], after['paid_look'])
            if operation != 'extend_capacity':
                for k in ('address_cells', 'members', 'rows'):
                    eq(natural(before['capacity'][k]), natural(after['capacity'][k]))
        owners = projected_owners(after)
        eq(array(sorted((o['original_name'] for o in owners), key=int)),
           array(sorted(issued_owners, key=int)), 'List Nat')
        for o in owners:
            require(o['original_name'] in issued_owners, 'Unissued retained owner')
            candidate_equal(issued_owners[o['original_name']], o)
            if 'initializer_owner' in o:
                name, identity = o['original_name'], o['initializer_owner']
                if name in initializers:
                    require(identity is not None, 'Erased learned initializer identity')
                    eq(natural(identity), natural(initializers[name]))
                elif identity is not None: initializers[name] = identity
        previous = row
    lines.append('end FrozenObserverExport')
    return '\n\n'.join(lines) + '\n', dict(events=len(rows), segments=segments,
        snapshots=len(checked), claims=len(claims))


if __name__ == '__main__':
    replay_main(emit=emit, modules=MODELS, producers=[Path(__file__).resolve()], scope=SCOPE,
                description=__doc__, export_loader=load_export)
