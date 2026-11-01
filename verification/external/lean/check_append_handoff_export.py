#!/usr/bin/env python3
"""Check frozen native-v4 append/handoff observations with the Lean kernel."""
import json
import re
from pathlib import Path
from observer_export_replay import (
    boolean, fields, module_closure, natural, require, sequence, main as replay_main,
)

MODELS = module_closure(['AppendHandoff'])
SCOPE = ('Finite native-v4 observations: committed index before handoff, retained pending '
         'identity, ordinary scheduling cursor preservation, zero-work/capacity snapshot '
         'nonmutation and emitted owner/output correspondence with the observed indexer. '
         'Readiness is a reported flag: the raw wake cursor is absent. No proof of complete '
         'wake visitation, hidden Rust state, expression semantics, memory safety, fairness, '
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


def emit(data):
    fields(data, 'schema scope states streams')
    require(data['schema'] == 'synarc-causal-constructor-native-v4', 'Unsupported constructor schema')
    require(type(data['scope']) is str and type(data['states']) is str, 'Missing export scope')
    streams = sequence(data['streams'])
    require(0 < len(streams) <= 16, 'Invalid stream count')
    definitions = ['import VerumExternalReplay.AppendHandoff', 'open AppendHandoff',
                   'set_option warningAsError true', 'namespace FrozenObserverExport']
    propositions, unique = [], set()
    event_count = publications = transactions = refusals = 0
    language_owners = []

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
            fields(state, STATE_FIELDS)
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
        'streams':len(streams), 'events':event_count, 'claims':len(groups),
        'propositions':len(propositions), 'publications':publications,
        'transaction_events':transactions, 'refusals':refusals,
    }


if __name__ == '__main__':
    replay_main(emit=emit, modules=MODELS, producers=[Path(__file__).resolve()],
                scope=SCOPE, description=__doc__)
