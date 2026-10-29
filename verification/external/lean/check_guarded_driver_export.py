#!/usr/bin/env python3
"""Replay every supplied native work/rank/control boundary with the checked guard.

The source and binary must be bound independently. Retained rank projections
do not carry complete VM/receiver state, ownership, or callback CPU termination.
"""
from pathlib import Path
from check_residual_queries import load_jsonl
from observer_export_replay import (array, boolean, fields, natural, require,
    sequence, module_closure, main as replay_main)

SCOPE = ('actual native outer callbacks, retained rank projection and exact work/results; '
         'not complete VM-state refinement or callback CPU termination')
LEVELS = ('Idle', 'Commit', 'Learn', 'Observe', 'Predict', 'Install', 'Intake')
MODULES = module_closure(['GuardedDrainTrace'])


def rank(value):
    fields(value, 'ordinal level phase')
    require(type(value['level']) is str and value['level'] in LEVELS, 'Unknown native lifecycle level')
    return '⟨' + ','.join(f'⟨{n},by decide⟩' for n in
        (natural(value['ordinal']), str(LEVELS.index(value['level'])), natural(value['phase']))) + '⟩'


def status(value):
    require(type(value) is dict, 'Expected callback result')
    if value.get('kind') in ('pending', 'ready'):
        fields(value, 'kind')
        return '.' + value['kind']
    fields(value, 'kind reason resumable')
    require(value['kind'] == 'refused' and type(value['reason']) is str, 'Unknown callback result')
    # The diagnostic and resumability are retained data, not independently
    # justified native error semantics. Never infer a result from Debug text.
    reason = array(str(b) for b in value['reason'].encode())
    return f'(.refused ({reason},{boolean(value["resumable"])}))'


def event(value):
    require(type(value) is dict, 'Expected driver event')
    kind = value.get('kind')
    if kind in ('enter', 'yielded', 'exhausted'):
        fields(value, 'kind remaining')
        return f'(.{kind} {natural(value["remaining"])})'
    fields(value, 'kind before_remaining after_remaining before after result')
    require(kind == 'transition', 'Unknown driver event')
    return (f'(.callback {natural(value["before_remaining"])} {natural(value["after_remaining"])} '
            f'{rank(value["before"])} {rank(value["after"])} {status(value["result"])})')


def emit(data):
    fields(data, 'packets')
    packets = sequence(data['packets']); require(packets, 'Missing native packets')
    lines = ['import VerumExternalReplay.GuardedDrainTrace', 'open GuardedDrainTrace',
             'set_option warningAsError true', 'set_option maxRecDepth 65536',
             'set_option maxHeartbeats 30000000', 'set_option cbv.maxSteps 10000000',
             'namespace FrozenObserverExport']
    names = set(); counts = dict(cases=0, services=0, callbacks=0, yields=0, exhaustions=0, claims=0)
    for ci, packet in enumerate(packets):
        fields(packet, 'case events format scope')
        require(packet['format'] == 'synarc-work-drain-v1' and packet['scope'] == SCOPE,
                'Unsupported driver format or scope')
        require(type(packet['case']) is str and packet['case'] not in names, 'Duplicate/invalid native case')
        names.add(packet['case']); counts['cases'] += 1
        events = sequence(packet['events']); require(events, 'Empty driver case')
        # Split only at the exported service entry. Each previous service must
        # close in Lean; a new entry cannot silently repair a truncated one.
        groups, current = [], []
        for value in events:
            encoded = event(value)
            if value['kind'] == 'enter':
                counts['services'] += 1
                if current: groups.append(current); current = []
            if value['kind'] == 'transition': counts['callbacks'] += 1
            if value['kind'] == 'yielded': counts['yields'] += 1
            if value['kind'] == 'exhausted': counts['exhaustions'] += 1
            current.append(encoded)
        if current: groups.append(current)
        # Bounded groups amortize kernel startup while retaining every event.
        for start in range(0, len(groups), 32):
            name = f'c{ci}_g{start}'
            values = array(array(group) for group in groups[start:start+32])
            lines.append(f'def {name} : List (List (Event (List Nat × Bool))) := {values}')
            lines.append(f'theorem {name}_valid : {name}.all valid = true := by decide_cbv')
            counts['claims'] += 1
    lines.append('end FrozenObserverExport')
    return '\n\n'.join(lines)+'\n', counts


if __name__ == '__main__':
    replay_main(emit=emit, modules=MODULES,
        producers=[Path(__file__).resolve(), Path(__file__).with_name('check_residual_queries.py')],
        scope='Every supplied native grant, callback work/rank projection, guard decision and explicit '
              'yield/exhaustion boundary. Ready/refused outer returns follow the inspected source branch, '
              'not a separately exported outer result. No full receiver/VM state, ownership or callback '
              'CPU termination proof.', description=__doc__, export_loader=load_jsonl)
