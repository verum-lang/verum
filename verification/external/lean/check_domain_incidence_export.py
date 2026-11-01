#!/usr/bin/env python3
"""Replay captured native immutable incidence transitions with the Lean kernel."""
import copy
import json
import re
from pathlib import Path
from observer_export_replay import (array, boolean, fields, natural, sequence,
    require, module_closure, main as replay_main)

MODELS = module_closure(['DomainIncidence'])
SCOPE = ('Finite native v2 unary plan/expansion observations: original position partition, '
         'exact domain keys, support masks, empty-group skipping, owned child continuations, '
         'work/capacity refusals, resumption and local cell deltas. The export does not record '
         'the slab actually appended, nullary or higher-arity constructors, other producer '
         'lanes, allocation failures or scheduling. Their execution is not proved. Foreign '
         'ownership controls concern the existing flatten boundary only. This replay does '
         'not verify a replacement producer or establish universal Rust refinement.')


def emit(data):
    fields(data, 'schema scope identity plan_phases expansion_phases streams controls')
    require(data['schema'] == 'synarc-domain-incidence-native-v2', 'Unsupported incidence schema')
    require(type(data['scope']) is str and type(data['identity']) is str, 'Missing export scope')
    for name, tags in [('plan_phases', ['Position', 'Lookup', 'Publish', 'Done']),
                       ('expansion_phases', ['Cases', 'Positions'])]:
        fields(data[name], ' '.join(tags))
        for index, tag in enumerate(tags):
            natural(data[name][tag], len(tags) - 1)
            require(data[name][tag] == index, 'Unknown phase encoding')
    streams, controls = sequence(data['streams']), sequence(data['controls'])
    require(0 < len(streams) <= 16 and len(controls) <= 64, 'Incidence export exceeds bounds')
    lines = ['import VerumExternalReplay.DomainIncidence', 'open DomainIncidence',
             'set_option warningAsError true', 'namespace FrozenObserverExport']
    claims, states, owners, supports, propositions = 0, {}, {}, {}, set()

    def claim(proposition, proof='by decide'):
        nonlocal claims
        if proposition in propositions: return
        propositions.add(proposition)
        lines.append(f'theorem incidence_{claims} : {proposition} := {proof}')
        claims += 1

    def same(left, right): claim(f'{left} = {right}')

    def address(value):
        require(type(value) is str and re.fullmatch('[0-9a-f]{1,16}', value) and int(value, 16) != 0,
                'Invalid captured owner address')
        return json.dumps(value)

    def state(value, requests, count):
        fields(value, 'plan_phase alternative anchor requested_domain lookup plan_cells groups child')
        phase = value['plan_phase']; natural(phase, 3)
        alternative = value['alternative']; natural(alternative, len(requests))
        natural(value['anchor'], 0)  # v2's domain_requests describes unary alternatives only.
        natural(value['requested_domain']); natural(value['plan_cells'])
        groups = sequence(value['groups']); require(len(groups) <= len(requests), 'Too many groups')
        natural(value['lookup'], len(groups))
        if alternative == len(requests):
            require(phase in (0, 3), 'Plan cursor outside native alternatives')
        else: require(phase != 3, 'Unfinished plan marked done')
        encoded = []
        for g in groups:
            require(len(sequence(g)) == 2, 'Invalid domain group')
            natural(g[0]); positions = sequence(g[1])
            require(0 < len(positions) <= len(requests), 'Invalid incidence count')
            pairs = []
            for pair in positions:
                require(len(sequence(pair)) == 2, 'Invalid original position')
                pairs.append(f'({natural(pair[0], len(requests)-1)}, {natural(pair[1], 0)})')
            encoded.append(f'⟨{g[0]}, {array(pairs)}⟩')
        child = value['child']; child_text = 'none'
        if child is not None:
            fields(child, 'language_owner child_owner arrival domain_index domain case mask position phase outputs')
            require(phase == 3, 'Expansion visible before immutable plan completion')
            language, identity = address(child['language_owner']), address(child['child_owner'])
            natural(child['arrival']); natural(child['domain_index'], len(groups))
            natural(child['phase'], 1); natural(child['case'], count)
            natural(child['mask'], (1 << count) - 1)
            outputs = sequence(child['outputs']); require(len(outputs) == count, 'Wrong immutable case count')
            outputs_text = array(natural(v) for v in outputs)
            if child['domain'] is not None: natural(child['domain'])
            if child['domain_index'] < len(groups):
                group = groups[child['domain_index']]
                if child['phase'] == 0:
                    require(child['case'] < count, 'Case cursor outside native outputs')
                    natural(child['position'], 0)
                else: natural(child['position'], len(group[1]) - 1)
            else:
                natural(child['position'], 0)
                require(child['phase'] == 0, 'Release outside case phase')
            tag = ['.cases', '.positions'][child['phase']]
            child_text = f'(some ⟨{language}, {identity}, {child["arrival"]}, {outputs_text}, '
            child_text += f'{child["domain_index"]}, {child["case"]}, {child["mask"]}, {child["position"]}, {tag}⟩)'
        key = json.dumps((requests, value), sort_keys=True)
        if key not in states:
            name = f'state_{len(states)}'; states[key] = name
            tag = ['.position', '.lookup', '.publish', '.done'][phase]
            lines.append(f'def {name} : State := ⟨⟨{alternative}, 0, {value["requested_domain"]}, '
                f'{value["lookup"]}, {value["plan_cells"]}, {tag}, {array(encoded)}⟩, {child_text}⟩')
            domains = array(f'[{natural(v)}]' for v in requests)
            claim(f'PlanSound {domains} {name}.plan')
            if child is not None:
                claim(f'∀ c ∈ {name}.child.toList, ChildSound {name}.plan.groups c')
                domain = 'none' if child['domain'] is None else f'(some {child["domain"]})'
                same(f'({name}.plan.groups[{child["domain_index"]}]?).map Group.domain', domain)
        return states[key]

    for index, stream in enumerate(streams):
        fields(stream, 'owner language_owner case_count domain_requests charged_total charged_refusals '
            'incidence_charged other_charged retained_plan_cells pending_cells_at_end events')
        natural(stream['owner']); require(stream['owner'] == index, 'Reordered/duplicate stream identity')
        owner = stream['language_owner']; address(owner)
        require(owner not in owners, 'Live language identities alias'); owners[owner] = stream
        requests = sequence(stream['domain_requests'])
        require(0 < len(requests) <= 64, 'Unary request count exceeds bounds')
        domains = array(f'[{natural(v)}]' for v in requests)
        count = stream['case_count']; natural(count, 64); require(count > 0, 'No native cases')
        for key in ('charged_total', 'charged_refusals', 'incidence_charged', 'other_charged',
                    'retained_plan_cells', 'pending_cells_at_end'): natural(stream[key])
        rows = sequence(stream['events']); require(0 < len(rows) <= 256, 'Event count exceeds bounds')
        initial = dict(plan_phase=0, alternative=0, anchor=0, requested_domain=0,
                       lookup=0, plan_cells=0, groups=[], child=None)
        plan = state(initial, requests, count) + '.plan'
        expected, arrivals, retired = {}, {}, set()
        previous, charges, refusals = None, [], []

        def observe(value):
            name = state(value, requests, count)
            same(name + '.plan', plan)
            c = value['child']
            if c is not None:
                same(json.dumps(c['language_owner']), address(owner))
                key = c['child_owner']; require(key not in retired, 'Released child identity reused')
                if key not in expected:
                    require(c['arrival'] not in arrivals, 'Arrival names another child')
                    arrivals[c['arrival']] = key
                    initial_child = copy.deepcopy(value)
                    initial_child['child'].update(domain_index=0, domain=value['groups'][0][0],
                        case=0, mask=0, position=0, phase=0)
                    expected[key] = state(initial_child, requests, count) + '.child'
                same(name + '.child', expected[key])
            return name

        for row in rows:
            fields(row, 'kind result grant charged capacity cells_before cells_after before after' +
                (' resumed' if row.get('result') == 'advanced' else ''))
            require(row['result'] in ('advanced', 'work_refusal', 'capacity_refusal'), 'Unknown result')
            for key in ('grant', 'charged', 'capacity', 'cells_before', 'cells_after'): natural(row[key])
            before, after = row['before'], row['after']
            before_name = observe(before); after_name = state(after, requests, count)
            c = before['child']; phase = before['plan_phase']
            if phase < 3:
                kind = {0:'plan_position', 2:'plan_publish'}.get(phase)
                if phase == 1:
                    lookup = before['lookup']; groups = before['groups']
                    kind = 'new_domain' if lookup == len(groups) else (
                        'shared_domain' if groups[lookup][0] == before['requested_domain'] else 'plan_probe')
            else:
                require(c is not None, 'No active expansion in a captured lane')
                kind = 'release_child' if c['domain_index'] == len(before['groups']) else (
                    'scan_domain' if c['phase'] == 0 else 'incidence_position')
            require(row['kind'] == kind, 'Event label does not describe captured state')
            if kind == 'release_child': require(row['cells_before'] >= 1, 'Release underflow')
            if row['result'] == 'work_refusal':
                require(previous is None or previous['result'] == 'advanced', 'Repeated incomplete attempt')
            else:
                require(previous is not None and previous['result'] in ('work_refusal', 'capacity_refusal'),
                        'Missing retained attempt')
                same(before_name, state(previous['before'], requests, count))
                same(str(row['cells_before']), str(previous['cells_before']))
            advanced = row['result'] == 'advanced'
            if advanced: same(boolean(row['resumed']), boolean(previous['result'] == 'capacity_refusal'))
            outcome = {'advanced':'.advanced', 'work_refusal':'.workRefusal', 'capacity_refusal':'.capacityRefusal'}[row['result']]
            # Release reveals a different queue head, not a mutation of the released child.
            observed_after = after_name
            if advanced and kind == 'release_child':
                released = copy.deepcopy(after); released['child'] = None
                observed_after = state(released, requests, count)
            claim(f'(let actual := step {domains} {before_name} {row["cells_before"]} '
                f'{row["grant"]} {row["capacity"]}; (actual.outcome, actual.charged, actual.cells, actual.state)) '
                f'= ({outcome}, {row["charged"]}, {row["cells_after"]}, {observed_after})')
            plan = after_name + '.plan'
            if c is not None:
                key = c['child_owner']
                if advanced and kind == 'release_child': del expected[key]; retired.add(key)
                else: expected[key] = after_name + '.child'
            observe(after)
            if advanced and kind == 'scan_domain' and c['case'] + 1 == count:
                supports[index, c['arrival'], c['domain']] = c['outputs']
            charges.append(str(row['charged']))
            if row['result'] == 'capacity_refusal': refusals.append(str(row['charged']))
            previous = row
        require(previous['result'] == 'advanced' and previous['after']['child'] is None and not expected,
                'Incomplete expansion lifetime')
        same(f'{plan}.phase', 'PlanPhase.done')
        # Construct the permutation witness from exact canonical grouping.
        # This also checks ordering, instead of only comparing a multiset.
        claim(f'(entries {plan}.groups).Perm (original {domains})',
              f'canonical_partition {plan}.groups (original {domains}) (by decide)')
        same(f'{plan}.cells', str(stream['retained_plan_cells']))
        same(f'({array(charges)} : List Nat).sum', str(stream['incidence_charged']))
        same(str(stream['charged_total']), f'{stream["incidence_charged"]} + {stream["other_charged"]}')
        claim(f'({array(refusals)} : List Nat).sum ≤ {stream["charged_refusals"]} ∧ '
              f'{stream["charged_refusals"]} ≤ {stream["charged_total"]}')
        same(str(stream['pending_cells_at_end']), '0')

    for control in controls:
        if control.get('kind') == 'foreign_owner':
            fields(control, 'kind source attempted result charged scope')
            source, target = control['source'], control['attempted']
            require(source in owners and target in owners, 'Uncaptured language identity')
            require(control['scope'] == 'existing flatten boundary; incidence plan accepts no external child insertion',
                    'Foreign owner control scope changed')
            claim(f'{address(source)} ≠ {address(target)}')
            same(json.dumps(control['result']), json.dumps('ForeignOwner'))
            same(natural(control['charged']), '0')
        elif control.get('kind') == 'changed_domain':
            fields(control, 'kind owner arrival domain1_mask domain2_mask')
            natural(control['owner'], len(streams) - 1); natural(control['arrival'])
            for domain in (1, 2):
                outputs = supports.get((control['owner'], control['arrival'], domain))
                require(outputs is not None, 'Changed domain lacks a completed captured scan')
                same(f'DomainSupport.mask {array(natural(v) for v in outputs)} {domain}',
                     natural(control[f'domain{domain}_mask']))
        else: raise ValueError('Unknown incidence control')
    return '\n\n'.join(lines + ['end FrozenObserverExport']) + '\n', {
        'streams':len(streams), 'events':sum(len(s['events']) for s in streams), 'claims':claims}


if __name__ == '__main__':
    replay_main(emit=emit, modules=MODELS, producers=[Path(__file__).resolve()], scope=SCOPE,
                description=__doc__)
