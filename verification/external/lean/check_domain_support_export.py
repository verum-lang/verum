#!/usr/bin/env python3
"""Check captured native unary support steps with an independent Lean model."""
import copy
import json
import re
from pathlib import Path
from observer_export_replay import (MAX64, array, boolean, fields, natural, sequence,
    require, module_closure, main as replay_main)

MODELS = module_closure(['DomainSupport'])
SCOPE = ('Finite captured unary support visits: exact immutable child/owner/domain bindings, '
         'scan bits, memo entries, lookup, per-child continuation, paid work/capacity refusals, '
         'resumption and local retained-cell deltas. Aggregate other-lane costs are consistency '
         'claims, not reconstructed execution. Foreign-owner controls describe the existing '
         'flatten boundary only. No general Rust refinement, FIFO scheduling, allocator failure, '
         'emitted-expression semantics or native u64 arithmetic theorem is claimed.')


def emit(data):
    fields(data, 'schema scope identity phase_codes streams controls')
    require(data['schema'] == 'synarc-domain-support-native-v1', 'Unsupported support schema')
    require(type(data['scope']) is str and type(data['identity']) is str, 'Missing scope')
    fields(data['phase_codes'], 'Domain Lookup Cases Publish Ready')
    for key, value in zip(('Domain', 'Lookup', 'Cases', 'Publish', 'Ready'), range(5)):
        natural(data['phase_codes'][key], 4)
        require(data['phase_codes'][key] == value, 'Unknown support phase encoding')
    streams, controls = sequence(data['streams']), sequence(data['controls'])
    require(0 < len(streams) <= 16 and len(controls) <= 64, 'Support export exceeds bounds')
    lines = ['import VerumExternalReplay.DomainSupport', 'open DomainSupport',
             'set_option warningAsError true', 'namespace FrozenObserverExport']
    claims, states, owners, publications = 0, {}, {}, {}
    propositions = set()

    def claim(proposition):
        nonlocal claims
        if proposition in propositions: return
        propositions.add(proposition)
        lines.append(f'theorem support_{claims} : {proposition} := by decide')
        claims += 1

    def same(a, b):
        claim(f'{a} = {b}')

    def address(value):
        require(type(value) is str and re.fullmatch('[0-9a-f]{1,16}', value) and int(value, 16) != 0,
                'Invalid captured owner address')
        return json.dumps(value)

    def state(value, requests, case_count):
        fields(value, 'language_owner child_owner arrival alternative anchor phase lookup case domain mask cache outputs')
        language, child = address(value['language_owner']), address(value['child_owner'])
        for name in ('arrival', 'domain'): natural(value[name])
        natural(value['alternative'], len(requests)); natural(value['anchor'], 0)
        phase, lookup = value['phase'], value['lookup']
        natural(phase, 4); natural(lookup, len(sequence(value['cache'])))
        if phase != 1: require(lookup == 0, 'Lookup cursor outside lookup phase')
        natural(value['case'], case_count); natural(value['mask'], (1 << case_count) - 1)
        require(len(sequence(value['outputs'])) == case_count, 'Wrong immutable case count')
        outputs = array(natural(v) for v in value['outputs'])
        cache = []
        for entry in value['cache']:
            require(len(sequence(entry)) == 2, 'Invalid cache entry')
            cache.append(f'({natural(entry[0])}, {natural(entry[1], (1 << case_count) - 1)})')
        require(len(cache) <= len(requests), 'Cache exceeds request domain count')
        # Bounds are explicit: no Lean default lookup may stand for native OOB.
        if value['alternative'] == len(requests):
            require(phase == 0, 'Release outside idle phase')
        elif phase == 2:
            require(value['case'] < case_count, 'Scan cursor outside native outputs')
        key = json.dumps(value, sort_keys=True)
        if key not in states:
            name = f'state_{len(states)}'
            tag = ['.domain', f'.lookup {lookup}', '.scan', '.publish', '.ready'][phase]
            lines.append(f'def {name} : State := ⟨{language}, {child}, {value["arrival"]}, '
                f'{outputs}, {value["alternative"]}, {tag}, {value["case"]}, {value["domain"]}, '
                f'{value["mask"]}, {array(cache)}⟩')
            states[key] = name
            claim(f'DomainSupport.Sound {name}')
        return states[key]

    for index, stream in enumerate(streams):
        fields(stream, 'stream language_owner domain_requests case_count emitted producer_charged_total '
            'support_charged_total other_charged_total producer_charged_refusals support_charged_refusals '
            'cache_entries_at_end pending_cells_at_end events')
        natural(stream['stream']); require(stream['stream'] == index, 'Duplicate/reordered stream identifier')
        owner = stream['language_owner']; address(owner)
        require(owner not in owners, 'Stream ownership aliases another live language')
        owners[owner] = stream
        requests = sequence(stream['domain_requests']); require(0 < len(requests) <= 64, 'Request count exceeds bounds')
        req = array(natural(v) for v in requests)
        count = stream['case_count']; natural(count, 64); require(count > 0, 'No native cases')
        for name in ('emitted', 'producer_charged_total', 'support_charged_total', 'other_charged_total',
                     'producer_charged_refusals', 'support_charged_refusals', 'cache_entries_at_end', 'pending_cells_at_end'):
            natural(stream[name])
        rows = sequence(stream['events']); require(0 < len(rows) <= 256, 'Event count exceeds bounds')
        expected, retired, arrivals = {}, set(), {}
        previous = None
        charges, refusals = [], []

        def observe(snapshot):
            name = state(snapshot, requests, count)
            same(f'{name}.language', address(owner))
            key = snapshot['child_owner']
            require(key not in retired, 'Released child reused without a new lifetime')
            if key not in expected:
                arrival = snapshot['arrival']
                require(arrival not in arrivals, 'Arrival identity reused by another child')
                arrivals[arrival] = key
                initial = copy.deepcopy(snapshot)
                initial.update(alternative=0, anchor=0, phase=0, lookup=0, case=0, domain=0, mask=0, cache=[])
                expected[key] = initial
            same(name, state(expected[key], requests, count))
            return name

        for position, row in enumerate(rows):
            base = 'kind result grant charged capacity cells_before cells_after before after'
            fields(row, base + (' resumed' if row.get('result') == 'advanced' else ''))
            require(row['result'] in ('work_refusal', 'capacity_refusal', 'advanced'), 'Unknown step outcome')
            for key in ('grant', 'charged', 'capacity', 'cells_before', 'cells_after'): natural(row[key])
            require(row['cells_before'] < MAX64, 'Potential native cell-count overflow is outside this export')
            before = row['before']; before_name = observe(before)
            phase, alternative = before['phase'], before['alternative']
            if alternative == len(requests):
                kind = 'release'
                require(row['cells_before'] >= 1 + len(before['cache']), 'Release cell count underflow')
            elif phase == 1:
                entry = before['cache'][before['lookup']] if before['lookup'] < len(before['cache']) else None
                kind = 'miss' if entry is None else ('hit' if entry[0] == before['domain'] else 'probe')
            else: kind = {0:'domain', 2:'scan', 3:'cache_publish', 4:'slab_publish_or_skip'}[phase]
            require(row['kind'] == kind, 'Step label does not describe captured state')
            if previous is not None and previous['after'] is not None:
                same(before_name, state(previous['after'], requests, count))
            if row['result'] != 'work_refusal':
                require(previous is not None and previous['result'] in ('work_refusal', 'capacity_refusal'),
                        'Missing preceding retained attempt')
                same(before_name, state(previous['before'], requests, count))
                same(str(row['cells_before']), str(previous['cells_before']))
            else:
                require(previous is None or previous['result'] == 'advanced', 'Repeated incomplete attempt')
            advanced = row['result'] == 'advanced'
            if advanced:
                same(boolean(row['resumed']), boolean(previous['result'] == 'capacity_refusal'))
            outcome = {'advanced':'.advanced', 'work_refusal':'.workRefusal', 'capacity_refusal':'.capacityRefusal'}[row['result']]
            child = before['child_owner']
            if advanced and kind == 'release':
                next_state = 'none'
                del expected[child]; retired.add(child)
            elif advanced and kind == 'slab_publish_or_skip':
                following = copy.deepcopy(before)
                following.update(alternative=alternative + 1, phase=0, lookup=0, case=0, domain=0, mask=0)
                expected[child] = following
                next_state = f'(some {state(following, requests, count)})'
            else:
                require(row['after'] is not None, 'Live support state disappeared')
                next_state = f'(some {state(row["after"], requests, count)})'
                expected[child] = copy.deepcopy(row['after'])
            claim(f'DomainSupport.step {req} {before_name} {row["cells_before"]} {row["grant"]} '
                  f'{row["capacity"]} = ⟨{outcome}, {row["charged"]}, {row["cells_after"]}, {next_state}⟩')
            if row['after'] is not None: observe(row['after'])
            if advanced and kind == 'cache_publish':
                publications[owner, before['arrival'], before['domain']] = (before['outputs'], before['mask'])
            charges.append(str(row['charged']))
            if row['result'] == 'capacity_refusal': refusals.append(str(row['charged']))
            previous = row
        require(previous['result'] == 'advanced' and previous['after'] is None and not expected,
                'Incomplete memo lifetime')
        same(f'({array(charges)} : List Nat).sum', str(stream['support_charged_total']))
        same(f'({array(refusals)} : List Nat).sum', str(stream['support_charged_refusals']))
        same(str(stream['producer_charged_total']), f'{stream["support_charged_total"]} + {stream["other_charged_total"]}')
        claim(f'{stream["support_charged_refusals"]} ≤ {stream["producer_charged_refusals"]} ∧ '
              f'{stream["producer_charged_refusals"]} ≤ {stream["producer_charged_total"]}')
        same(str(stream['cache_entries_at_end']), '0')
        same(str(stream['pending_cells_at_end']), '0')

    for control in controls:
        if control.get('kind') == 'foreign_owner':
            fields(control, 'kind source_language_owner attempted_language_owner result charged cache_unchanged scope')
            source, target = control['source_language_owner'], control['attempted_language_owner']
            require(source in owners and target in owners, 'Foreign control names an uncaptured language')
            require(control['scope'] == 'existing flatten ownership boundary; cache accepts no external insertion',
                    'Foreign control scope changed')
            claim(f'{address(source)} ≠ {address(target)}')
            same(json.dumps(control['result']), json.dumps('ForeignOwner'))
            same(natural(control['charged']), '0'); same(boolean(control['cache_unchanged']), 'true')
        elif control.get('kind') == 'changed_domain':
            fields(control, 'kind language_owner arrival first_domain first_mask changed_domain changed_mask distinct_native_supports')
            owner = control['language_owner']; require(owner in owners, 'Unknown control owner')
            natural(control['arrival']); natural(control['first_domain']); natural(control['changed_domain'])
            claim(f'{control["first_domain"]} ≠ {control["changed_domain"]}')
            for key in ('first', 'changed'):
                published = publications.get((owner, control['arrival'], control[key + '_domain']))
                require(published is not None, 'Control lacks a captured publication')
                outputs, observed = published
                same(f'DomainSupport.mask {array(natural(v) for v in outputs)} {control[key + "_domain"]}', natural(control[key + '_mask']))
                same(str(observed), str(control[key + '_mask']))
            same(boolean(control['distinct_native_supports']), 'true')
            claim(f'{control["first_mask"]} ≠ {control["changed_mask"]}')
        else: raise ValueError('Unknown support control')
    return '\n\n'.join(lines + ['end FrozenObserverExport']) + '\n', {
        'streams':len(streams), 'events':sum(len(s['events']) for s in streams), 'claims':claims}


if __name__ == '__main__':
    replay_main(emit=emit, modules=MODELS, producers=[Path(__file__).resolve()], scope=SCOPE,
                description=__doc__)
