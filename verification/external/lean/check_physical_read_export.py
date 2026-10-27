#!/usr/bin/env python3
"""Replay retained native reads/commits at observed physical boundaries, not arbitrary executions."""
from pathlib import Path
import check_lifecycle_export as lifecycle
from observer_export_replay import array, fields, natural, require, sequence, main as replay_main


def physical(item):
    fields(item, 'raw pose life_age episode runtime_raw runtime_pose')
    for key in ('raw', 'life_age', 'episode', 'runtime_raw'): natural(item[key])
    for key in ('pose', 'runtime_pose'):
        lifecycle.option(item[key], lambda n: natural(n, 255))
    return item


def emit(data):
    fields(data, 'schema definitions cases')
    require(data['schema'] == 'synarc-physical-observer-read-v1', 'Unknown physical read schema')
    require(lifecycle.validate_adapter(data) == 'AcquiredV2', 'Physical read replay requires AcquiredV2')
    base = dict(schema='synarc-observer-lifecycle-v1', definitions=data['definitions'],
                ownership=[], naming=[], installation=[], refusals=[])
    source, _ = lifecycle.emit(base)
    lines = [source.removesuffix('end FrozenObserverExport\n')]
    names = []
    def claim(equation, proof='rfl'):
        name = 'p' + str(len(names)); names.append(name)
        lines.append(f'theorem {name} : {equation} := by\n  {proof}')
    def same(a, b): claim(f'({natural(a)} : Nat) = {natural(b)}')
    def optional(a, b):
        claim(f'({lifecycle.option(a, natural)} : Option Nat) = {lifecycle.option(b, natural)}')
    def same_cut(a, b):
        register(a); register(b)
        claim(f'({lifecycle.cut(a)} : ObserverOwnership.Cut Nat Nat Nat) = {lifecycle.cut(b)}')
    def ports(actual, expected):
        for key in ('raw', 'episode', 'life_age'): same(actual[key], expected[key])
        optional(actual['pose'], expected['pose'])
    definitions = {d['identity']: d for d in data['definitions']}
    register, _, states = lifecycle.cut_registry(definitions, claim)
    rows = sequence(data['cases']); require(rows, 'No physical observations')
    reads = commits = terminals = 0
    previous = None
    for i, case in enumerate(rows):
        fields(case, 'before after action native witness outcome table')
        before, after = case['before'], case['after']
        retained_before = set(states)
        retained_before.add(before['cut']['state'])
        for snapshot in (before, after):
            fields(snapshot, 'physical cut namespace attempts ordinal')
            p = physical(snapshot['physical']); register(snapshot['cut'])
            lifecycle.namespace(snapshot['namespace'])
            natural(snapshot['attempts']); natural(snapshot['ordinal'])
            same(p['runtime_raw'], p['raw']); optional(p['runtime_pose'], p['pose'])
        same(after['attempts'], before['attempts'] + 1)
        same(after['ordinal'], before['ordinal'] + 1)
        if previous is not None:
            same_cut(previous['cut'], before['cut'])
            same(previous['ordinal'], before['ordinal']); same(previous['attempts'], before['attempts'])
            ports(before['physical'], previous['physical'])
        previous = after
        for key in ('owner', 'capacity'): same(before['namespace'][key], after['namespace'][key])
        claim(f'({lifecycle.wide(before["namespace"]["epoch"])} : Nat) = '
              f'{lifecycle.wide(after["namespace"]["epoch"])}')
        keys = lifecycle.namespace(before['namespace'])
        table_name = f'keys_{i}_0'
        lines.append(f'def {table_name} : List (Nat × Value) := {keys}')
        claim(f'{table_name}.Nodup', 'decide')
        capacity = natural(before['namespace']['capacity'])
        witness, table = case['witness'], case['table']
        fields(witness, 'ordinal lineage raw situation action episode life_age pose epoch inputs')
        lifecycle.lineage(witness['lineage']); lifecycle.wide(witness['epoch'])
        fields(table, 'source action successor')
        natural(case['action'], 255); natural(table['source']); natural(table['action'], 255)
        lifecycle.option(table['successor'], natural)
        same(witness['ordinal'], before['ordinal']); ports(witness, before['physical'])
        same(witness['action'], case['action']); same(table['action'], case['action'])
        same(witness['situation'], table['source'])
        claim(f'({lifecycle.wide(witness["epoch"])} : Nat) = '
              f'{lifecycle.wide(before["namespace"]["epoch"])}')
        inputs = {}
        for entry in sequence(witness['inputs']):
            require(type(entry) is list and len(entry) == 2, 'Malformed layer input')
            lifecycle.wide(entry[0]); natural(entry[1])
            require(entry[0] not in inputs, 'Duplicate layer input epoch')
            inputs[entry[0]] = entry[1]
        current = before['cut']; commit_count = pre_reads = post_reads = 0
        for j, observation in enumerate(sequence(case['native'])):
            require(type(observation) is dict, 'Expected native observation')
            kind = observation.get('kind')
            require(kind in ('Read', 'Commit'), 'Unknown native observation')
            if kind == 'Read':
                fields(observation, 'kind view query cut value execution name')
                view = observation['view']; fields(view, 'raw input episode life_age pose under')
                lifecycle.option(view['under'], natural); natural(view['input'])
                supplied = observation['query']; phase = 'read'
                same_cut(observation['cut'], current)
                expected_ports = before['physical'] if commit_count == 0 else after['physical']
                ports(view, expected_ports)
                if commit_count == 0:
                    pre_reads += 1; same(observation['name'], table['source'])
                else:
                    post_reads += 1
                expected_value = observation['value']; reads += 1
            else:
                fields(observation, 'kind input receipt execution')
                require(commit_count == 0, 'Multiple commits for one physical action')
                receipt = observation['receipt']; fields(receipt, 'before after')
                same_cut(receipt['before'], current); register(receipt['after'])
                claim(f'({natural(receipt["after"]["state"])} : Nat) ≠ {natural(current["state"])}', 'decide')
                # The prepared state must be fresh among every retained state.
                claim(f'({array(natural(n) for n in sorted(retained_before))} : List Nat).contains '
                      f'{natural(receipt["after"]["state"])} = false')
                claim(f'ObserverOwnership.commit ({lifecycle.cut(current)} : ObserverOwnership.Cut Nat Nat Nat) '
                      f'⟨{lifecycle.cut(receipt["before"])}, {natural(receipt["after"]["state"])}⟩ = '
                      f'⟨{lifecycle.cut(receipt["after"])}, .ok '
                      f'⟨{lifecycle.cut(receipt["before"])}, {lifecycle.cut(receipt["after"])}⟩⟩')
                supplied = observation['input']; phase = 'update'
                expected_value = receipt['after']['value']; commits += 1
            source_row, values = lifecycle.input_row(supplied)
            require(len(supplied['values']) == 1 and supplied['values'][0].get('kind') ==
                    ('Query' if phase == 'read' else 'Event'), 'Expected one complete native port row')
            row = supplied['values'][0]['value']
            row_source = {k: row[k] for k in ('epoch', 'raw_space', 'owner')}
            row_source['shape'] = 'Query' if phase == 'read' else 'Event'
            claim(f'({source_row} : Source) = {lifecycle.source(row_source)}')
            if phase == 'read':
                ports(row, view); same(row['input'], view['input']); optional(row['action'], None)
            else:
                ports(row, witness); same(row['action'], witness['action'])
                require(row['epoch'] in inputs, 'Witness missing update input epoch')
                same(row['input'], inputs[row['epoch']])
                if row['owner'] is not None:
                    claim(f'({lifecycle.lineage(row["owner"])} : Lineage) = {lifecycle.lineage(witness["lineage"])}')
            definition = definitions[current['definition']]
            prefix = 'd' + natural(current['definition']); cap = natural(definition['semantics']['max_trace'])
            invocation = f'physical_{i}_{j}'
            lines.append(f'def {invocation} := ObserverAdmission.evaluate (apply {cap}) .{phase} '
                f'{prefix}_{phase} {prefix}_{phase}_binding {lifecycle.value(current["value"])} '
                f'{source_row} {values} ⟨none, 2^64-1, 0, 0, false⟩')
            claim(f'observe {invocation} = .ok ({lifecycle.value(expected_value)}, '
                  f'{lifecycle.report(observation["execution"])})',
                  f'unfold {invocation} {prefix}_{phase} {prefix}_{phase}_binding\n  reduce_admission')
            if phase == 'update':
                current = observation['receipt']['after']; commit_count += 1
            else:
                new_table = f'keys_{i}_{j+1}'
                pair = f'({natural(view["input"])}, {lifecycle.value(expected_value)})'
                interning = f'ExactNames.intern (min {capacity} (2^56)) {table_name} {pair}'
                claim(f'({interning}).2 = some {natural(observation["name"])}')
                lines.append(f'def {new_table} := ({interning}).1'); table_name = new_table
                if commit_count:
                    outcome = case['outcome']; lifecycle.value(outcome)
                    require(outcome['kind'] == 'Outcome', 'Expected actual typed outcome')
                    if outcome['value']['kind'] == 'Next': same(observation['name'], table['successor'])
        require(commit_count == 1 and pre_reads > 0, 'Missing actual commit or prior read')
        same_cut(current, after['cut'])
        claim(f'{table_name} = ({lifecycle.namespace(after["namespace"])} : List (Nat × Value))')
        lifecycle.value(case['outcome'])
        require(case['outcome']['kind'] == 'Outcome', 'Expected typed physical outcome')
        outcome = case['outcome']['value']
        if outcome['kind'] == 'Next':
            require(post_reads > 0, 'Missing actual successor read')
            same(outcome['key'], table['successor'])
        else:
            require(outcome['kind'] == 'End', 'Unsupported physical refusal case')
            require(table['successor'] is None, 'Terminal outcome was stored as Next')
            terminals += 1
    lines += ['#print axioms ' + name for name in names] + ['end FrozenObserverExport']
    return '\n\n'.join(lines) + '\n', dict(cases=len(rows), reads=reads, commits=commits,
                                          terminals=terminals, claims=len(names))


def main():
    replay_main(emit=emit, modules=lifecycle.MODELS,
        producers=[Path(__file__).resolve(), Path(lifecycle.__file__).resolve()], description=__doc__,
        scope='Finite retained successful Read/Committed objects, exact query/update/report replay, '
              'owner/state continuity, physical prior/successor ports, exact namespace and terminal table boundary. '
              'Not original work-budget admission, lower-prefix projection fidelity, arbitrary Habitat fidelity, statistical acquisition, '
              'retirement, allocator universality or infinite executions.')


if __name__ == '__main__':
    main()
