#!/usr/bin/env python3
"""Replay frozen native chunk-forest operations with exact retained identities."""
import copy
import json
from pathlib import Path
from observer_export_replay import (array, boolean, fields, natural, require, sequence,
    module_closure, main as replay_main)

MODELS = module_closure(['PersistentEvents'])
SCOPE = ('Finite exported append, cached lengths, exact node/input identity, indexing, '
         'work refusal, retained owner/slot counts and capacity lower bounds. Origins are abstract bytes; physical '
         'Cut ownership, allocator failure, panic semantics, balance bounds and whole '
         'Automatic engine refinement are not certified.')


def emit(data):
    fields(data, 'schema index_bits scope identity histories nodes calls retained')
    require(data['schema'] == 'synarc.native-prefix-history.v1', 'Unknown history schema')
    require(type(data['index_bits']) is int and data['index_bits'] in (32, 64), 'Invalid index width')
    maximum = 2**data['index_bits'] - 1
    for key in ('scope', 'identity'):
        require(type(data[key]) is str, 'Expected scope text')
    nodes, histories, rows = (sequence(data[k]) for k in ('nodes', 'histories', 'calls'))
    require(len(nodes) <= 4096 and len(histories) <= 4096 and 0 < len(rows) <= 16384,
            'History export exceeds finite replay bounds')
    lines = ['import VerumExternalReplay.PersistentEvents\nopen PersistentEvents\n'
             'set_option warningAsError true\nnamespace FrozenObserverExport']
    claims, event_ids, inputs, computed, visiting = {}, {}, {}, set(), set()

    def claim(expression, proof='decide_cbv'):
        if expression not in claims:
            name = 'c' + str(len(claims)); claims[expression] = name
            lines.append(f'theorem {name} : {expression} := by {proof}')

    def equal(actual, expected):
        # JSON is only a closed structural equality carrier. It is not parsed
        # by Lean as semantics; the forest laws below use typed model terms.
        quote = lambda v: json.dumps(json.dumps(v, sort_keys=True, separators=(',', ':')))
        claim(f'({quote(actual)} : String) = {quote(expected)}')

    def index(value, values):
        natural(value)
        require(value < len(values), 'Unknown retained identity')
        return value

    def event(e):
        require(type(e) is dict, 'Expected event')
        kind = e.get('kind')
        require(kind in ('Gap', 'Observed', 'Completed'), 'Unknown event kind')
        fields(e, 'kind' if kind == 'Gap' else 'kind input_owner interface values outcome')
        if kind != 'Gap':
            natural(e['input_owner']); natural(e['interface'], 255)
            for value in sequence(e['values']): natural(value, 255)
            if kind == 'Completed':
                for value in sequence(e['outcome']): natural(value, 255)
            else: require(e['outcome'] is None, 'Observed event has an outcome')
            payload = [e['interface'], e['values']]
            if e['input_owner'] in inputs: equal(payload, inputs[e['input_owner']])
            else: inputs[e['input_owner']] = payload
        key = json.dumps(e, sort_keys=True)
        if key not in event_ids: event_ids[key] = len(event_ids)
        return str(event_ids[key])

    def events(es):
        require(len(sequence(es)) <= 4096, 'Event sequence exceeds replay bound')
        return array([event(e) for e in es])

    def tree(i):
        index(i, nodes)
        if i in computed: return 'n' + str(i)
        require(i not in visiting and len(visiting) < 128, 'Cyclic or excessive history depth')
        visiting.add(i)
        n = fields(nodes[i], 'id event_len rank body')
        natural(n['id']); equal(n['id'], i)
        natural(n['event_len'], maximum); natural(n['rank'], data['index_bits'] - 1)
        body = n['body']; require(type(body) is dict, 'Expected node body')
        if body.get('kind') == 'Leaf':
            fields(body, 'kind events'); term = f'.leaf {events(body["events"])}'
            equal(n['rank'], 0); equal(n['event_len'], len(body['events']))
        else:
            fields(body, 'kind left right'); require(body['kind'] == 'Branch', 'Unknown node kind')
            left, right = tree(body['left']), tree(body['right'])
            equal(nodes[body['left']]['rank'], nodes[body['right']]['rank'])
            equal(n['rank'], nodes[body['left']]['rank'] + 1)
            term = f'.branch {n["event_len"]} {left} {right}'
        name = 'n' + str(i)
        lines.append(f'def {name} : Tree Nat := {term}')
        claim(name + '.Valid')
        visiting.remove(i); computed.add(i)
        return name

    def roots(rs):
        return array([tree(i) for i in sequence(rs)])

    def history(h):
        fields(h, 'id origin end chunks roots events')
        index(h['id'], histories); natural(h['origin'], 255)
        natural(h['end'], maximum); natural(h['chunks'], maximum)
        equal(h, histories[h['id']])
        return 'h' + str(h['id'])

    for i in range(len(nodes)): tree(i)
    for i, h in enumerate(histories):
        name = history(h); equal(h['id'], i)
        rs, es = roots(h['roots']), events(h['events'])
        lines.append(f'def {name} : Snapshot Nat := ⟨{rs},{h["end"]}⟩')
        claim(name + '.Valid'); claim(f'flatten {rs} = ({es} : List Nat)')
        equal(h['chunks'], sum(2**nodes[j]['rank'] for j in h['roots']))
        ranks = [nodes[j]['rank'] for j in h['roots']]
        equal(ranks, sorted(set(ranks), reverse=True))
        for j in range(len(h['events']) + 1):
            claim(f'{name}.get {j} = ({es} : List Nat)[{j}]?')

    def state(s):
        fields(s, 'origin previous copied_roots roots carry suffix_len shape complete poisoned stopped')
        if s['origin'] is not None: natural(s['origin'], 255)
        for k in ('previous', 'complete'):
            if s[k] is not None: index(s[k], histories)
        natural(s['copied_roots']); roots(s['roots'])
        if s['carry'] is not None: tree(s['carry'])
        if s['suffix_len'] is not None: natural(s['suffix_len'], maximum)
        if s['shape'] is not None:
            require(len(sequence(s['shape'])) == 2, 'Invalid shape')
            for x in s['shape']: natural(x, maximum)
        boolean(s['poisoned'])
        require(s['stopped'] in (None, 'ForeignOrigin', 'Overflow', 'Poisoned'), 'Unknown sticky refusal')
        return s

    active, published, allocated, completed = {}, set(), set(), set()
    for row in rows:
        require(type(row) is dict and type(row.get('case')) is str, 'Missing case')
        case, op = row['case'], row.get('operation')
        if op == 'begin_append':
            fields(row, 'case operation origin previous suffix after')
            require(case not in active, 'Repeated append case')
            natural(row['origin'], 255); events(row['suffix'])
            old = row['previous']
            if old is not None:
                history(old); require(old['id'] in published, 'Unpublished predecessor')
            expected = dict(origin=row['origin'], previous=None if old is None else old['id'],
                copied_roots=0, roots=[], carry=None, suffix_len=len(row['suffix']), shape=None,
                complete=None, poisoned=False, stopped=None)
            equal(state(row['after']), expected)
            active[case] = dict(last=row['after'], old=old, suffix=row['suffix'], finished=False)
        elif op == 'advance':
            fields(row, 'case operation capacity grant spent remaining before returned after')
            require(case in active and not active[case]['finished'], 'Advance without live append')
            current = active[case]; before, after = state(row['before']), state(row['after'])
            equal(before, current['last']); expected = copy.deepcopy(before)
            for k in ('capacity', 'grant', 'spent', 'remaining'): natural(row[k], maximum)
            result, reason, spent = 'Pending', None, 0
            old = current['old']; old_roots = [] if old is None else old['roots']
            if before['poisoned']: reason = 'Poisoned'
            elif before['stopped'] is not None: reason = before['stopped']
            elif before['complete'] is not None: result = 'Complete'
            elif row['grant'] == 0: reason = 'Work'
            else:
                spent = 1
                if before['shape'] is None:
                    require(before['suffix_len'] is not None, 'Missing pending suffix')
                    shape = [0, 0] if old is None else [old['end'], old['chunks']]
                    shape = [shape[0] + before['suffix_len'], shape[1] + int(before['suffix_len'] > 0)]
                    if old is not None and old['origin'] != before['origin']: reason = 'ForeignOrigin'
                    elif max(shape) > maximum: reason = 'Overflow'
                    elif shape[0] > row['capacity']: reason = 'Capacity'
                    else: expected['shape'] = shape
                elif before['shape'][0] > row['capacity']: reason = 'Capacity'
                elif before['copied_roots'] < len(old_roots):
                    expected['roots'].append(old_roots[before['copied_roots']])
                    expected['copied_roots'] += 1
                elif before['suffix_len'] is not None:
                    expected['suffix_len'] = None
                    if before['suffix_len'] > 0:
                        nid = index(after['carry'], nodes)
                        require(nid not in allocated, 'Reused leaf owner')
                        allocated.add(nid); expected['carry'] = nid
                        equal(nodes[nid]['body'], dict(kind='Leaf', events=current['suffix']))
                elif before['carry'] is not None:
                    right = before['carry']; left = before['roots'][-1] if before['roots'] else None
                    if left is not None and nodes[left]['rank'] == nodes[right]['rank']:
                        nid = index(after['carry'], nodes)
                        require(nid not in allocated, 'Reused branch owner')
                        allocated.add(nid); expected['carry'] = nid; expected['roots'].pop()
                        equal(nodes[nid]['body'], dict(kind='Branch', left=left, right=right))
                        claim(f'n{nid}.events = n{left}.events ++ n{right}.events')
                    else:
                        expected['roots'].append(right); expected['carry'] = None
                else:
                    nid = index(after['complete'], histories)
                    require(nid not in completed, 'Reused snapshot owner')
                    completed.add(nid)
                    h = histories[nid]; expected['complete'] = nid
                    expected['origin'] = None; expected['roots'] = []; result = 'Complete'
                    equal(h, dict(id=nid, origin=before['origin'], end=before['shape'][0],
                        chunks=before['shape'][1], roots=before['roots'],
                        events=([] if old is None else old['events']) + current['suffix']))
            if reason in ('ForeignOrigin', 'Overflow', 'Poisoned') and spent:
                expected['stopped'] = reason
            equal(row['returned'], dict(kind='Refused', reason=reason) if reason else dict(kind=result))
            equal(row['spent'], spent); equal(row['remaining'], row['grant'] - spent)
            equal(after, expected); current['last'] = after
        elif op == 'finish':
            fields(row, 'case operation returned')
            require(case in active and not active[case]['finished'], 'Finish without live append')
            current = active[case]; require(current['last']['complete'] is not None, 'Premature finish')
            history(row['returned']); equal(row['returned']['id'], current['last']['complete'])
            published.add(row['returned']['id']); current['finished'] = True
        elif op == 'iter':
            fields(row, 'case operation history items reverse end_get_is_none')
            index(row['history'], histories); require(row['history'] in published, 'Unpublished iteration')
            es = histories[row['history']]['events']; events(row['items']); events(row['reverse'])
            boolean(row['end_get_is_none']); equal(row['items'], es); equal(row['reverse'], es[::-1])
            equal(row['end_get_is_none'], True)
        elif op == 'shape':
            fields(row, 'case operation old_len old_chunks suffix_len returned scope')
            require(type(row['scope']) is str, 'Missing shape scope')
            for k in ('old_len', 'old_chunks', 'suffix_len'): natural(row[k], maximum)
            shape = [row['old_len'] + row['suffix_len'], row['old_chunks'] + int(row['suffix_len'] > 0)]
            equal(row['returned'], None if max(shape) > maximum else shape)
        else: raise ValueError('Unknown history operation')
    require(published == set(range(len(histories))), 'Missing snapshot publication')
    require(allocated == set(range(len(nodes))), 'Unobserved node allocation')
    require(all(s['finished'] or s['last']['stopped'] is not None for s in active.values()),
            'Truncated append lifecycle')
    retained = fields(data['retained'], 'tree_nodes chunks event_slots event_capacity snapshot_roots snapshot_root_capacity')
    for value in retained.values(): natural(value)
    reachable = set()
    def retain(i):
        if i in reachable: return
        reachable.add(i); b = nodes[i]['body']
        if b['kind'] == 'Branch': retain(b['left']); retain(b['right'])
    for h in histories:
        for i in h['roots']: retain(i)
    leaves = [nodes[i] for i in reachable if nodes[i]['body']['kind'] == 'Leaf']
    equal(retained['tree_nodes'], len(reachable)); equal(retained['chunks'], len(leaves))
    equal(retained['event_slots'], sum(len(n['body']['events']) for n in leaves))
    equal(retained['snapshot_roots'], sum(len(h['roots']) for h in histories))
    claim(f'({retained["event_slots"]} : Nat) ≤ {retained["event_capacity"]}')
    claim(f'({retained["snapshot_roots"]} : Nat) ≤ {retained["snapshot_root_capacity"]}')
    lines.append('end FrozenObserverExport')
    return '\n\n'.join(lines) + '\n', dict(calls=len(rows), snapshots=len(histories),
        nodes=len(nodes), claims=len(claims))


if __name__ == '__main__':
    replay_main(emit=emit, modules=MODELS, producers=[Path(__file__).resolve()], scope=SCOPE,
                description=__doc__)
