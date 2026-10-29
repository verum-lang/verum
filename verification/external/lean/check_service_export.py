#!/usr/bin/env python3
"""Replay actual retained service operations, ownership projections and results."""
import json
from pathlib import Path
import acquired_export_codec
from acquired_export_codec import LEGACY, wide, lineage
from observer_export_replay import (array, boolean, fields, natural, require,
    sequence, module_closure, main as replay_main)

MODELS = module_closure(['GuardedDrainTrace'])
SCOPE = ('Finite native service operations and exported state continuity, work, phase progress, '
         'publication receipts, installed definition identities and outer results. '
         'Snapshots omit VM payloads, allocator state and the rest of the queue; this is not '
         'whole-state Rust refinement, a callback termination proof or a statistical guarantee.')
PHASES = {'Validate': 2, 'Ready': 1, 'Receiver': 0, 'Installed': 0}
OWNERS = ('attempt_owner', 'definition_owner', 'publication_definition_owner',
          'signature_admission_definition_owner')
PUBLICATION = ('publication_definition_owner', 'signature_admission_definition_owner',
               'signature_version', 'awaiting_signatures')
RETAINED = ('attempt_owner', 'definition_owner', 'source', 'look', 'reserved_look',
            'next_look', 'queue')


def encoded(value):
    # Injective canonical JSON bytes retain all exported fields in continuity
    # comparisons; they are data, never Lean source or inferred native state.
    raw = json.dumps(value, sort_keys=True, separators=(',', ':'), ensure_ascii=True).encode()
    return '(' + array(str(b) for b in raw) + ' : List Nat)'


def snapshot(value):
    fields(value, 'stage remaining physical source identity_scope attempt_owner definition_owner '
           'look reserved_look next_look signature_version awaiting_signatures '
           'publication_definition_owner install_refused installed_floors '
           'installed_floor_definitions signature_admission_definition_owner queue')
    require(type(value['stage']) is str and value['stage'] in PHASES, 'Unsupported retained service phase')
    require(value['queue'] in ('Worker', 'Replacement'), 'Unknown service queue')
    require(value['identity_scope'] == 'opaque process-local Arc addresses; not semantic fingerprints',
            'Unsupported identity scope')
    for key in OWNERS + ('look', 'reserved_look', 'next_look'):
        if value[key] is not None: wide(value[key])
    require(value['attempt_owner'] is not None, 'Missing retained owner')
    require(value['definition_owner'] is not None, 'Missing retained definition')
    wide(value['next_look'])
    for key in ('remaining', 'awaiting_signatures', 'installed_floors'):
        natural(value[key])
    if value['signature_version'] is not None: natural(value['signature_version'])
    boolean(value['install_refused'])
    fields(value['physical'], 'lineage epoch ordinal')
    lineage(value['physical']['lineage']); wide(value['physical']['epoch'])
    natural(value['physical']['ordinal'])
    if value['source'] is not None: LEGACY.source(value['source'])
    indices = set()
    for floor in sequence(value['installed_floor_definitions']):
        fields(floor, 'index definition_owner'); natural(floor['index']); wide(floor['definition_owner'])
        require(floor['index'] < value['installed_floors'] and floor['index'] not in indices,
                'Invalid installed floor index')
        indices.add(floor['index'])


def rank(state):
    # Embed the administrative phase order in the generic finite-rank checker.
    # These fixed coordinates are not observations of physical input progress.
    return f'⟨⟨0,by decide⟩,⟨0,by decide⟩,⟨{PHASES[state["stage"]]},by decide⟩⟩'


def emit(data):
    fields(data, 'schema scope events')
    require(data['schema'] == 'synarc-native-service-trace-v2', 'Unknown service schema')
    require(type(data['scope']) is str, 'Missing scope')
    lines = ['import VerumExternalReplay.GuardedDrainTrace', 'open GuardedDrainTrace',
             'set_option warningAsError true', 'set_option maxRecDepth 65536',
             'namespace FrozenObserverExport']
    names = []; steps_count = 0

    def claim(proposition):
        name = 'c' + str(len(names)); names.append(name)
        lines.append(f'theorem {name} : {proposition} := by\n  decide_cbv')

    def equal(a, b): claim(f'{encoded(a)} = {encoded(b)}')
    def same(a, b, keys): equal({k: a[k] for k in keys}, {k: b[k] for k in keys})

    def invariants(before, after):
        snapshot(before); snapshot(after)
        claim(f'({after["remaining"]} : Nat) ≤ {before["remaining"]}')
        same(before, after, RETAINED)
        same(before['physical'], after['physical'], ('lineage', 'ordinal'))
        equal(before['look'], before['reserved_look'])

    events = sequence(data['events']); require(events, 'Missing service calls')
    previous = None
    for event in events:
        fields(event, 'before after result steps')
        before, after = event['before'], event['after']; invariants(before, after)
        require(type(event['result']) is str, 'Invalid outer result')
        if previous is not None:
            same(previous, before, tuple(k for k in before if k != 'remaining'))
        previous = after
        control = [f'.enter {before["remaining"]}']; cursor = before
        callbacks = sequence(event['steps']); status = '.pending'; outer = 'Pending'
        for step in callbacks:
            fields(step, 'operation before after result'); steps_count += 1
            b, a, result = step['before'], step['after'], step['result']
            equal(cursor, b); invariants(b, a); cursor = a
            require(type(result) is dict and 'kind' in result, 'Missing operation result')
            require(step['operation'] in ('publication_gate', 'advance_observer_phase'),
                    'Unknown service operation')
            published = False
            if result['kind'] == 'Err':
                fields(result, 'kind reason'); require(type(result['reason']) is str, 'Invalid error')
                status = f'(.refused {encoded(result["reason"])})'
                outer = f'Err({result["reason"]})'
                if step['operation'] == 'publication_gate':
                    same(b, a, tuple(k for k in b if k != 'remaining'))
            elif step['operation'] == 'publication_gate':
                fields(result, 'kind definition_owner outcome')
                require(result['kind'] == 'Ok', 'Unknown publication result'); wide(result['definition_owner'])
                outcome = result['outcome']; fields(outcome, 'kind version slot')
                require(outcome['kind'] == 'RetainedAwaitingSource', 'Unsupported publication outcome')
                natural(outcome['version']); natural(outcome['slot'])
                equal(b['stage'], 'Ready'); equal(a['stage'], 'Ready')
                equal(b['publication_definition_owner'], None)
                for key in ('definition_owner', 'publication_definition_owner', 'signature_admission_definition_owner'):
                    equal(a[key], result['definition_owner'])
                equal(a['signature_version'], outcome['version'])
                equal(outcome['slot'], b['awaiting_signatures'])
                claim(f'({a["awaiting_signatures"]} : Nat) = {b["awaiting_signatures"]} + 1')
                claim(f'({outcome["version"]} : Nat) = {b["signature_version"] or 0} + 1')
                claim(f'({a["remaining"]} : Nat) < {b["remaining"]}')
                status = '.pending'; outer = 'Pending'
            else:
                require(result['kind'] == 'Ok' and 'value' in result, 'Unknown phase result')
                value = result['value']; fields(result, 'kind value reason' if value == 'Retire' else 'kind value')
                require(value in ('Advanced', 'Blocked', 'Published', 'Retire'), 'Unknown phase step')
                same(b, a, PUBLICATION)
                if value == 'Advanced':
                    allowed = [['Validate', 'Ready'], ['Ready', 'Receiver']]
                    claim(f'{encoded([b["stage"], a["stage"]])} ∈ {array(encoded(edge) for edge in allowed)}')
                    status = '.pending'; outer = 'Pending'
                elif value == 'Blocked':
                    status = '.yielded'; outer = 'Pending'
                    if b['stage'] == 'Ready' and b['source'] is not None and b['source']['owner'] != b['physical']['lineage']:
                        equal(a['install_refused'], True)
                elif value == 'Published':
                    status = '.ready'; outer = 'Published'; published = True
                    equal(b['stage'], 'Receiver'); equal(a['stage'], 'Installed')
                    equal(a['install_refused'], False)
                    require(a['definition_owner'] is not None and a['source'] is not None,
                            'Missing installed definition/source')
                    for key in ('publication_definition_owner', 'signature_admission_definition_owner'):
                        equal(a[key], a['definition_owner'])
                    equal(a['installed_floor_definitions'], b['installed_floor_definitions'] +
                          [dict(index=b['installed_floors'], definition_owner=a['definition_owner'])])
                    claim(f'({a["installed_floors"]} : Nat) = {b["installed_floors"]} + 1')
                    equal(a['source']['owner'], a['physical']['lineage'])
                    claim(f'({a["physical"]["epoch"]} : Nat) = {b["physical"]["epoch"]} + 1')
                else:
                    require(type(result['reason']) is str, 'Invalid retirement reason')
                    status = f'(.refused {encoded(result["reason"])})'; outer = f'Retire({result["reason"]})'
            if not published:
                same(b, a, ('installed_floors', 'installed_floor_definitions'))
                equal(b['physical']['epoch'], a['physical']['epoch'])
            control.append(f'(.callback {b["remaining"]} {a["remaining"]} {rank(b)} {rank(a)} {status})')
        equal(cursor, after); equal(event['result'], outer)
        if status == '.yielded': control.append(f'.yielded {after["remaining"]}')
        elif status == '.pending': control.append(f'.exhausted {after["remaining"]}')
        claim(f'valid (E := List Nat) {array(control)} = true')
    lines += ['#print axioms ' + name for name in names] + ['end FrozenObserverExport']
    return '\n\n'.join(lines) + '\n', dict(services=len(events), steps=steps_count, claims=len(names))


if __name__ == '__main__':
    replay_main(emit=emit, modules=MODELS,
        producers=[Path(__file__).resolve(), Path(acquired_export_codec.__file__).resolve()],
        scope=SCOPE, description=__doc__)
