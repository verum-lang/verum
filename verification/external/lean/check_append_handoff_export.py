#!/usr/bin/env python3
"""Check frozen native-v4/v5 append/handoff observations with the Lean kernel."""
import json
import re
from pathlib import Path
from observer_export_replay import (
    boolean, fields, module_closure, natural, require, sequence, main as replay_main,
)

MODELS = module_closure(['AppendHandoff', 'WakeHandoff'])
SCOPE = ('Finite native-v4/v5 observations: committed index before handoff, retained pending '
         'identity, ordinary scheduling cursor preservation, zero-work/capacity snapshot '
         'nonmutation and emitted owner/output correspondence with the observed indexer. '
         'V4 readiness is a reported flag; its raw wake cursor is absent. V5 additionally '
         'derives readiness, checks every paid wake update and complete ordered visit pass, '
         'and binds the pending owner to indexing and emission. No proof of '
         'hidden Rust state, expression semantics, memory safety, fairness, '
         'universal Rust refinement, or a replacement producer.')
STATE_FIELDS = ('active_constructors active_tuples append_ready catalogue_entries cells complete '
                'constructor_cursor constructors indexed_entries indexer next_ordinary_lane '
                'pending_cells pending_emission pending_index phase plan posting_entries postings seed')


def encoded(value, depth=0):
    """Injective, bounded canonical JSON; all snapshot fields remain represented."""
    require(depth <= 20, 'Snapshot nesting exceeds bound')
    if value is None or type(value) is bool:
        return
    if type(value) is int:
        natural(value)
    elif type(value) is str:
        require(value.isascii() and len(value) <= 256 and not any(ord(c) < 32 for c in value),
                'Unsupported snapshot string')
    elif type(value) is list:
        require(len(value) <= 8192, 'Snapshot array exceeds bound')
        for item in value: encoded(item, depth + 1)
    elif type(value) is dict:
        require(len(value) <= 64, 'Snapshot object exceeds bound')
        for key, item in value.items():
            encoded(key, depth + 1)
            encoded(item, depth + 1)
    else:
        raise ValueError('Non-JSON snapshot value')


def literal(value):
    encoded(value)
    return json.dumps(json.dumps(value, sort_keys=True, separators=(',', ':')))


def snapshot(value):
    encoded(value)
    data = json.dumps(value, sort_keys=True, separators=(',', ':'))
    chunks = [json.dumps(data[i:i + 32]) for i in range(0, len(data), 32)] or ['""']
    def balanced(start, end):
        if end - start == 1: return f'(Snapshot.leaf {chunks[start]})'
        middle = (start + end) // 2
        return f'(Snapshot.branch {balanced(start, middle)} {balanced(middle, end)})'
    return balanced(0, len(chunks))


def optional(value):
    return 'none' if value is None else f'(some {natural(value)})'


def owner(value):
    require(type(value) is str and re.fullmatch(r'[1-9a-f][0-9a-f]{0,15}', value),
            'Expected a nonzero canonical captured owner identity')
    return json.dumps(value)


def wake_claims(stream, number, claim, definitions, child_owners):
    """V5 projection of the actual commit_index/wake/handoff branches."""
    states = stream['states']
    names = {}
    def constructor(state_id, slot):
        key = state_id, slot
        if key not in names:
            c = states[state_id]['constructors'][slot]
            fields(c, 'alternative anchor axes carry newest observed phase scan slot tuple watermark')
            name = f'c_{number}_{state_id}_{slot}'
            values = [json.dumps(c['phase']), natural(c['scan']), optional(c['newest']),
                      natural(c['watermark']), natural(c['observed']),
                      snapshot({k:v for k,v in c.items() if k not in
                               ('phase', 'scan', 'newest', 'watermark', 'observed')})]
            definitions.append(f'def {name} : WakeHandoff.Constructor AppendHandoff.Snapshot := ⟨{", ".join(values)}⟩')
            names[key] = name
        return names[key]
    def frame(state, excluded):
        return snapshot({k:v for k,v in state.items() if k not in excluded})
    def retained_constructors(state):
        # AxisCensus.posting is a lookup into the mutable posting bank, not a
        # retained constructor field. Verify that lookup separately below.
        return [{**c, 'axes': [{k:v for k,v in axis.items() if k != 'posting'}
                              for axis in c['axes']]} for c in state['constructors']]
    def pair(value):
        require(type(value) is list and len(value) == 2, 'Expected posting pair')
        return f'({natural(value[0])}, {natural(value[1])})'
    posting_names = {}
    for state in states:
        pending = state['pending_index']
        cursor, count = natural(state['wake_cursor']), len(sequence(state['constructors']))
        claim(f'{boolean(state["append_ready"])} = WakeHandoff.ready {optional(pending)} '
              f'{natural(state["indexed_entries"])} {natural(state["catalogue_entries"])} {cursor} {count}')
        postings = sequence(state['postings'])
        key = literal(postings)
        if key not in posting_names:
            groups = []
            for group in postings:
                require(type(group) is list and len(group) == 2, 'Expected posting bank')
                groups.append(f'({natural(group[0])}, [{", ".join(pair(p) for p in sequence(group[1]))}])')
            posting_names[key] = f'posting_{number}_{len(posting_names)}'
            definitions.append(f'def {posting_names[key]} : List (Nat × List (Nat × Nat)) := [{", ".join(groups)}]')
        bank = posting_names[key]
        for c in state['constructors']:
            for axis in sequence(c['axes']):
                fields(axis, 'domain_index domain consumed before after coordinate posting')
                group = f'({bank}[{natural(axis["domain_index"])}]?)'
                claim(f'{group}.map Prod.fst = some {natural(axis["domain"])}')
                actual = 'none' if axis['posting'] is None else f'some {pair(axis["posting"])}'
                claim(f'{group}.bind (fun g => g.2[{natural(axis["coordinate"])}]?) = {actual}')
        if pending is None:
            require(state['pending_owner'] is None, 'Owner without pending append')
        else:
            arrival = natural(pending)
            require(arrival in child_owners, 'Pending owner lacks observed indexing identity')
            claim(f'{owner(state["pending_owner"])} = {child_owners[arrival]}')
            claim(f'({cursor} : Nat) ≤ {count}')
    for child in child_owners.values():
        claim(f'{child} ≠ {owner(stream["language_owner"])}')
    visits, commits, wakes, handoffs = {}, 0, 0, 0
    for event in stream['events']:
        if event['result'] in ('work_refusal', 'capacity_refusal'):
            continue  # Full snapshot equality is already checked, including v5 fields.
        a, b = event['before'], event['after']
        before, after = states[a], states[b]
        pending = before['pending_index']
        if pending is None:
            continue
        count = len(before['constructors'])
        claim(f'({len(after["constructors"])} : Nat) = {count}')
        if before['indexed_entries'] < before['catalogue_entries']:
            committed = after['indexed_entries'] > before['indexed_entries']
            if committed:
                claim(f'({natural(after["indexed_entries"])} : Nat) = {natural(before["indexed_entries"])} + 1')
                claim(f'({natural(after["wake_cursor"])} : Nat) = 0')
                visits[pending] = []
                commits += 1
            else:
                claim(f'({natural(after["wake_cursor"])} : Nat) = {natural(before["wake_cursor"])}')
            claim(f'{snapshot(retained_constructors(before))} = {snapshot(retained_constructors(after))}')
        elif before['wake_cursor'] < count:
            cursor = before['wake_cursor']
            require(pending in visits, 'Wake lacks observed commit/reset')
            visits[pending].append(cursor)
            claim(f'{json.dumps(event["result"])} = "advanced"')
            claim(f'({natural(after["wake_cursor"])} : Nat) = {natural(cursor)} + 1')
            indexed = natural(before['indexed_entries'])
            for slot in range(count):
                c, d = constructor(a, slot), constructor(b, slot)
                claim(f'{d} = ' + (f'WakeHandoff.wake {indexed} {c}' if slot == cursor else c))
            c = constructor(a, cursor)
            claim(f'({natural(after["active_constructors"])} : Nat) = {natural(before["active_constructors"])} + '
                  f'(if WakeHandoff.needsWake {indexed} {c} then 1 else 0)')
            excluded = ('wake_cursor', 'constructors', 'active_constructors', 'append_ready', 'phase')
            claim(f'{frame(before, excluded)} = {frame(after, excluded)}')
            wakes += 1
        else:
            require(pending in visits, 'Handoff lacks observed commit/reset')
            claim(f'WakeHandoff.visits {count} 0 {json.dumps(visits[pending])} {count}')
            claim(f'({natural(before["wake_cursor"])} : Nat) = {count}')
            excluded = ('pending_index', 'pending_owner', 'pending_emission', 'append_ready', 'phase')
            claim(f'{frame(before, excluded)} = {frame(after, excluded)}')
            handoffs += 1
    return dict(commits=commits, wake_visits=wakes, completed_wake_passes=handoffs)


def emit(data):
    fields(data, 'schema scope states streams')
    require(data['schema'] in ('synarc-causal-constructor-native-v4', 'synarc-causal-constructor-native-v5'),
            'Unsupported constructor schema')
    v5 = data['schema'].endswith('-v5')
    require(type(data['scope']) is str and type(data['states']) is str, 'Missing export scope')
    streams = sequence(data['streams'])
    require(0 < len(streams) <= 16, 'Invalid stream count')
    definitions = ['import VerumExternalReplay.AppendHandoff', 'import VerumExternalReplay.WakeHandoff', 'open AppendHandoff',
                   'set_option warningAsError true', 'namespace FrozenObserverExport']
    propositions, unique = [], set()
    event_count = publications = transactions = refusals = 0
    language_owners = []
    wake_counts = dict(commits=0, wake_visits=0, completed_wake_passes=0)

    def claim(proposition):
        if proposition not in unique:
            unique.add(proposition)
            propositions.append(f'({proposition})')

    for number, stream in enumerate(streams):
        fields(stream, 'argument_domains emitted events foreign_owner_control language_owner '
                       'max_steps owner rows states')
        encoded({k:v for k,v in stream.items() if k not in ('states', 'events')})
        language_owners.append(owner(stream['language_owner']))
        claim(f'({natural(stream["owner"])} : Nat) = {number}')
        snapshots, events, emitted = map(sequence, (stream['states'], stream['events'], stream['emitted']))
        require(0 < len(snapshots) <= 8192 and 0 < len(events) <= 8192, 'Trace exceeds bounds')
        state_names, raw_names, indexed_owners, child_owners = [], [], {}, {}
        for index, state in enumerate(snapshots):
            fields(state, STATE_FIELDS + (' wake_cursor pending_owner' if v5 else ''))
            encoded(state)
            s, raw = f's_{number}_{index}', f'raw_{number}_{index}'
            state_names.append(s); raw_names.append(raw)
            values = [natural(state['catalogue_entries']), natural(state['indexed_entries']),
                      optional(state['pending_index']), boolean(state['pending_emission']),
                      boolean(state['append_ready']), natural(state['next_ordinary_lane']),
                      natural(state['constructor_cursor'])]
            definitions += [f'def {s} : State := ⟨{", ".join(values)}⟩',
                            f'def {raw} : Snapshot := {snapshot(state)}']
            claim(f'valid {s}')
            indexer = state['indexer']
            if indexer is not None:
                fields(indexer, 'arrival case child_owner domain domain_index language_owner mask outputs phase')
                arrival = natural(indexer['arrival'])
                child_identity = owner(indexer['child_owner'])
                claim(f'({arrival} : Nat) = {s}.indexed')
                claim(f'{s}.pending = some {arrival}')
                claim(f'{owner(indexer["language_owner"])} = {language_owners[-1]}')
                identity = literal([indexer['child_owner'], indexer['outputs']])
                if arrival in indexed_owners: claim(f'{identity} = {indexed_owners[arrival]}')
                else:
                    indexed_owners[arrival] = identity
                    child_owners[arrival] = child_identity

        claim(f'([{", ".join(child_owners.values())}] : List String).Nodup')

        previous = 0
        used_publications = []
        for event in events:
            result = event.get('result')
            require(result in ('advanced', 'expression', 'complete', 'work_refusal', 'capacity_refusal'),
                    'Unknown observation result')
            refused = result in ('work_refusal', 'capacity_refusal')
            fields(event, 'before after capacity charged grant result' +
                   ('' if refused else ' publication resumed'))
            before_id = int(natural(event['before'], len(snapshots) - 1))
            after_id = int(natural(event['after'], len(snapshots) - 1))
            before, after = snapshots[before_id], snapshots[after_id]
            a, b = state_names[before_id], state_names[after_id]
            claim(f'({before_id} : Nat) = {previous}')
            previous = after_id
            grant, charged = natural(event['grant']), natural(event['charged'])
            natural(event['capacity'])
            claim(f'({charged} : Nat) ≤ {grant}')
            if refused:
                refusals += 1
                expected = 0 if result == 'work_refusal' else 1
                claim(f'({grant} : Nat) = {expected} ∧ ({charged} : Nat) = {expected}')
                claim(f'{raw_names[before_id]} = {raw_names[after_id]}')
            else:
                boolean(event['resumed'])
                claim(f'({charged} : Nat) = 1')
            if before['pending_index'] is not None:
                transactions += 1
                claim(f'preservesTransaction {a} {b}')
            if after['catalogue_entries'] > before['catalogue_entries']:
                claim(f'{a}.pending = none ∧ {b}.catalogue = {a}.catalogue + 1 ∧ '
                      f'{b}.indexed = {a}.indexed ∧ {b}.pending = some {a}.catalogue')
            if not refused and before['pending_index'] is not None and after['pending_index'] is None:
                output = optional(before['pending_index']) if result == 'expression' else 'none'
                claim(f'handoff {a} = some ({b}, {output})')
            if result == 'expression':
                publication = int(natural(event['publication'], len(emitted) - 1))
                used_publications.append(publication)
                value = emitted[publication]
                fields(value, 'arrival body dependencies outputs owner size')
                arrival = natural(value['arrival'])
                owner(value['owner'])
                claim(f'handoff {a} = some ({b}, some {arrival})')
                require(arrival in indexed_owners, 'Handoff lacks an observed catalogue owner')
                claim(f'{literal([value["owner"], value["outputs"]])} = {indexed_owners[arrival]}')
                publications += 1
            elif not refused:
                require(event['publication'] is None, 'Non-emission references an expression')
            event_count += 1
        claim(f'({json.dumps(used_publications)} : List Nat) = {json.dumps(list(range(len(emitted))))}')
        claim(f'{boolean(snapshots[previous]["complete"])} = true')
        claim(f'{state_names[previous]}.pending = none')
        claim(f'{json.dumps(events[-1]["result"])} = "complete"')
        if v5:
            for key, value in wake_claims(stream, number, claim, definitions, child_owners).items():
                wake_counts[key] += value

    claim(f'([{", ".join(language_owners)}] : List String).Nodup')

    # Group only to bound Lean invocation overhead; every proposition is kept.
    groups = [propositions[i:i + 16] for i in range(0, len(propositions), 16)]
    def proof(count):
        # Definitional equality is kernel evidence. Avoid materializing a
        # byte-by-byte decision proof when two exact snapshots are identical.
        leaf = '(by first | rfl | decide)'
        return leaf if count == 1 else f'And.intro {leaf} ({proof(count - 1)})'
    claims = [f'theorem observation_{i} : {" ∧ ".join(group)} := {proof(len(group))}'
              for i, group in enumerate(groups)]
    return '\n\n'.join(definitions + claims + ['end FrozenObserverExport']) + '\n', {
        'schema':data['schema'], 'streams':len(streams), 'events':event_count, 'claims':len(groups),
        'propositions':len(propositions), 'publications':publications,
        'transaction_events':transactions, 'refusals':refusals,
        **(wake_counts if v5 else {}),
    }


if __name__ == '__main__':
    replay_main(emit=emit, modules=MODELS, producers=[Path(__file__).resolve()],
                scope=SCOPE, description=__doc__)
