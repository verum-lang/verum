#!/usr/bin/env python3
"""Replay actual body-ready native naming cursors, including interleaved fork appends."""
from pathlib import Path
import check_lifecycle_export as life
from observer_export_replay import (array, boolean, fields, natural, require,
    sequence, main as replay_main)

MODELS = life.MODELS + ['RetainedNames']


def emit(data):
    fields(data, 'schema cases sibling_append capacity_grants')
    require(data['schema'] == 'synarc-retained-names-v1', 'Unknown retained naming schema')
    rows = sequence(data['cases']); require(bool(rows), 'Missing cursor observations')
    lines = ['import VerumExternalReplay.AcquiredObserver\nimport VerumExternalReplay.RetainedNames\n'
             'open AcquiredObserver\nset_option warningAsError true\nnamespace FrozenObserverExport']
    claims, shared, cursors, inputs, owners, states = [], {}, {}, {}, {}, {}
    emitted = set()

    def claim(expression, proof='rfl'):
        if (expression, proof) in emitted: return
        emitted.add((expression, proof))
        name = 'c' + str(len(claims)); claims.append(name)
        lines.append(f'theorem {name} : {expression} := by\n  {proof}')

    def term(expression, type_):
        key = (expression, type_)
        if key not in shared:
            name = 'd' + str(len(shared)); shared[key] = name
            lines.append(f'def {name} : {type_} := {expression}')
        return shared[key]

    def eq(a, b, type_='Nat'):
        claim(f'({a} : {type_}) = {b}')

    def number(a, b): eq(natural(a), natural(b))

    def value(v):
        # This schema carries the current acquired value format, including the
        # explicit optional pose. Absence must not silently select a legacy ABI.
        pending = [v]
        while pending:
            node = pending.pop()
            if type(node) is dict:
                if node.get('kind') == 'Event':
                    require(type(node.get('value')) is dict and 'pose' in node['value'], 'Missing event pose')
                if node.get('kind') == 'Trace':
                    require(all(type(e) is dict and 'pose' in e for e in sequence(node.get('events'))), 'Missing trace pose')
                pending.extend(node.values())
            elif type(node) is list: pending.extend(node)
        return term(life.value(v), 'Value')

    def query(q):
        require(type(q) is dict and 'values' in q, 'Missing query values')
        for v in sequence(q['values']): value(v)
        interface, values = life.input_row(q)
        return term(f'({interface}, {values})', 'Source × List Value')

    def key(s):
        return f'({natural(s["input"])}, {value(s["value"])})'

    def namespace(s):
        life.namespace(s)
        # The runtime reserves eight action bits. Existing entries may exceed
        # a subsequently lowered capacity, but never this representable domain.
        require(len(s['entries']) <= 2**56, 'Unrepresentable native names')
        encoded = array(f'({natural(e["input"])}, {value(e["value"])})' for e in s['entries'])
        return term(encoded, 'List (Nat × Value)')

    def same_namespace(a, b, table=True, capacity=True):
        namespace(a); namespace(b)
        number(a['owner'], b['owner']); eq(life.wide(a['epoch']), life.wide(b['epoch']))
        if capacity: number(a['capacity'], b['capacity'])
        if table: eq(namespace(a), namespace(b), 'List (Nat × Value)')

    def immutable(a, b):
        for field in ('owner', 'input_owner', 'input'): number(a[field], b[field])
        for field in ('definition', 'owner', 'state'): number(a['cut'][field], b['cut'][field])
        eq(value(a['cut']['value']), value(b['cut']['value']), 'Value')
        eq(value(a['value']), value(b['value']), 'Value')
        eq(query(a['query']), query(b['query']), 'Source × List Value')
        same_namespace(a['namespace'], b['namespace'], table=False, capacity=False)

    def progress(a, b):
        immutable(a, b); number(a['scanned'], b['scanned'])
        for field in ('finished', 'poisoned'): eq(boolean(a[field]), boolean(b[field]), 'Bool')

    def snapshot(s):
        fields(s, 'owner input_owner namespace expected_namespace same_namespace query call_input same_input_owner cut input value scanned finished poisoned')
        for field in ('owner', 'input_owner', 'input', 'scanned'): natural(s[field])
        for field in ('finished', 'poisoned'): boolean(s[field])
        eq(boolean(s['same_namespace']), 'true', 'Bool')
        same_namespace(s['namespace'], s['expected_namespace'])
        life.cut(s['cut']); query(s['query']); value(s['value'])
        eq(boolean(s['same_input_owner']), 'true', 'Bool')
        eq(query(s['query']), query(s['call_input']), 'Source × List Value')
        supplied = s['query']['values']
        require(len(supplied) == 1 and supplied[0].get('kind') == 'Query', 'Expected actual acquired query row')
        q = supplied[0]['value']
        expected_interface = {k: q[k] for k in ('epoch', 'raw_space', 'owner')}
        expected_interface['shape'] = 'Query'
        eq(life.source(s['query']['interface']), life.source(expected_interface), 'Source')
        number(s['input'], q['input'])
        eq(life.option(q['action'], lambda a: natural(a, 255)), 'none', 'Option UInt8')
        c = s['cut']; owner = c['owner']; state = c['state']
        if owner in owners: number(owners[owner], c['definition'])
        else: owners[owner] = c['definition']
        if state in states:
            definition, payload = states[state]
            number(definition, c['definition']); eq(payload, value(c['value']), 'Value')
        else: states[state] = (c['definition'], value(c['value']))
        if s['input_owner'] in inputs:
            eq(inputs[s['input_owner']], query(s['query']), 'Source × List Value')
        else: inputs[s['input_owner']] = query(s['query'])
        eq(boolean(s['poisoned']), 'false', 'Bool')

    append = data['sibling_append']
    fields(append, 'before_case before after same_namespace name target_cursor sibling_cursor')
    position = int(natural(append['before_case']))
    require(0 < position < len(rows), 'Append must occur between observed cases')
    events = {position: ('append', append)}
    previous_position = -1
    for grant in sequence(data['capacity_grants']):
        fields(grant, 'before_case before after')
        position = int(natural(grant['before_case']))
        require(previous_position < position < len(rows) and position not in events,
                'Ambiguous or unordered namespace event')
        previous_position = position; events[position] = ('capacity', grant)
    current = None
    for index, row in enumerate(rows):
        if index in events:
            kind, event = events[index]
            require(current is not None, 'Namespace event has no observed predecessor')
            same_namespace(current, event['before'])
            same_namespace(event['before'], event['after'], table=kind == 'capacity', capacity=kind == 'append')
            if kind == 'capacity':
                claim(f'({len(event["after"]["entries"])} : Nat) ≤ {natural(event["after"]["capacity"])}', 'decide')
            else:
                eq(boolean(event['same_namespace']), 'true', 'Bool')
                target, sibling = event['target_cursor'], event['sibling_cursor']
                snapshot(target); snapshot(sibling)
                require(target['owner'] in cursors and sibling['owner'] not in cursors,
                        'Append must retain its target and identify a fresh sibling cursor')
                progress(cursors[target['owner']], target)
                eq(boolean(target['finished']), 'false', 'Bool')
                eq(boolean(sibling['finished']), 'true', 'Bool')
                same_namespace(target['namespace'], event['after'])
                same_namespace(sibling['namespace'], event['after'])
                eq(query(target['query']), query(sibling['query']), 'Source × List Value')
                eq(key(target), key(sibling), 'Nat × Value')
                for field in ('definition', 'state'): number(target['cut'][field], sibling['cut'][field])
                eq(value(target['cut']['value']), value(sibling['cut']['value']), 'Value')
                claim(f'({natural(target["cut"]["owner"])} : Nat) ≠ {natural(sibling["cut"]["owner"])}', 'decide')
                before, after = namespace(event['before']), namespace(event['after'])
                cap = min(event['before']['capacity'], 2**56)
                claim(f'ExactNames.intern {cap} {before} {key(sibling)} = ({after}, some {natural(event["name"])})')
                eq(after, f'{before} ++ [{key(sibling)}]', 'List (Nat × Value)')
                number(event['name'], len(event['before']['entries']))
                number(sibling['scanned'], event['name'])
                cursors[target['owner']] = target; cursors[sibling['owner']] = sibling
            current = event['after']
        fields(row, 'name before grant remaining result after')
        require(type(row['name']) is str and bool(row['name']), 'Missing observation name')
        before, after = row['before'], row['after']; snapshot(before); snapshot(after)
        if current is not None: same_namespace(current, before['namespace'])
        if before['owner'] in cursors: progress(cursors[before['owner']], before)
        else: number(before['scanned'], 0)
        immutable(before, after)
        same_namespace(before['namespace'], after['namespace'], table=False)
        eq(boolean(before['finished']), 'false', 'Bool')
        # This schema observes zero/one-unit grants after native body selection.
        # Larger grants require a separately represented multi-step execution.
        natural(row['grant'], 1); natural(row['remaining'], row['grant'])
        number(row['remaining'], 0)
        paid = row['grant'] - row['remaining'] == 1
        keys, final = namespace(before['namespace']), namespace(after['namespace'])
        cap = min(before['namespace']['capacity'], 2**56)
        scanned = natural(before['scanned']); result = row['result']
        require(type(result) is dict and 'kind' in result, 'Missing naming result')
        if result['kind'] == 'Named':
            fields(result, 'kind name'); natural(result['name'], 2**56 - 1)
            outcome = '.named ' + natural(result['name'])
            number(after['scanned'], before['scanned'])
            claim(f'ExactNames.intern {cap} {keys} {key(before)} = ({final}, some {natural(result["name"])})')
        elif result['kind'] == 'Work':
            fields(result, 'kind'); outcome = '.pending ' + natural(after['scanned'])
            claim(f'RetainedNames.Checked {final} {key(after)} {natural(after["scanned"])}',
                  'unfold RetainedNames.Checked\n  decide')
        else:
            require(result['kind'] == 'Capacity', 'Body-ready naming cannot return an unobserved Pending result')
            fields(result, 'kind epoch'); outcome = '.capacity'
            eq(life.wide(result['epoch']), life.wide(before['namespace']['epoch']))
            number(after['scanned'], before['scanned'])
        eq(boolean(after['finished']), boolean(result['kind'] == 'Named'), 'Bool')
        claim(f'RetainedNames.Checked {keys} {key(before)} {scanned}', 'unfold RetainedNames.Checked\n  decide')
        claim(f'({keys} : List (Nat × Value)).Nodup', 'decide')
        claim(f'RetainedNames.step {cap} {keys} {key(before)} {scanned} {boolean(paid)} = ({final}, {outcome})')
        cursors[after['owner']] = after; current = after['namespace']
    lines += ['#print axioms ' + name for name in claims] + ['end FrozenObserverExport']
    return '\n\n'.join(lines) + '\n', dict(cases=len(rows), claims=len(claims),
        sibling_appends=1, capacity_grants=len(data['capacity_grants']), cursors=len(cursors))


def main():
    replay_main(emit=emit, modules=MODELS, producers=[Path(__file__).resolve(), *life.PRODUCERS],
        description=__doc__, scope='Finite body-ready naming cursors under zero/one-unit grants, '
        'actual retained nominal identities, exact keys, shared table chronology and sibling append. '
        'Not VM evaluation, authenticated pointer provenance, arbitrary meters, allocator failures, '
        'concurrent lock correctness or universal native continuation refinement.')


if __name__ == '__main__': main()
