#!/usr/bin/env python3
"""Replay actual joint initialization and transitions; acquisition admission is separate."""
from pathlib import Path
import check_lifecycle_export as lifecycle
from observer_export_replay import array, fields, natural, require, sequence, main as replay_main


def receipt(item):
    fields(item, 'owner authority count index look comparison_count')
    for number in item.values(): natural(number)
    return item


def emit(data):
    fields(data, 'schema definition candidate definition_receipt initialization transition history replay')
    require(data['schema'] == 'synarc-joint-observer-v1', 'Unknown joint export schema')
    definition = data['definition']
    require(lifecycle.validate_adapter({'definitions': [definition], 'joint': data}) == 'AcquiredV2',
            'Joint production replay requires explicit AcquiredV2')
    base = dict(schema='synarc-observer-lifecycle-v1', definitions=[definition],
                ownership=[], naming=[], installation=[], refusals=[])
    source, counts = lifecycle.emit(base)
    require(counts['claims'] == 0, 'Unexpected base observations')
    lines = [source.removesuffix('end FrozenObserverExport\n')]
    names = []
    def claim(equation, proof='rfl'):
        name = 'j' + str(len(names)); names.append(name)
        lines.append(f'theorem {name} : {equation} := by\n  {proof}')
    def same(a, b): claim(f'({natural(a)} : Nat) = {natural(b)}')
    def distinct(a, b): claim(f'({natural(a)} : Nat) ≠ {natural(b)}', 'decide')

    candidate = data['candidate']
    fields(candidate, 'owner definition grammar index syntax_indices family_count evidence_look comparison_count provenance')
    natural(candidate['owner'])  # Nominal token; no semantic-equivalence inference.
    same(candidate['definition'], definition['identity'])
    grammar = candidate['grammar']; fields(grammar, 'owner authority count component_counts')
    natural(grammar['owner'])
    origin = receipt(candidate['provenance']); sealed = receipt(data['definition_receipt'])
    for field in origin: same(origin[field], sealed[field])
    same(origin['authority'], grammar['authority'])
    same(origin['count'], grammar['count']); same(candidate['family_count'], grammar['count'])
    for field, supplied in [('index', 'index'), ('look', 'evidence_look'), ('comparison_count', 'comparison_count')]:
        same(origin[field], candidate[supplied])
    sizes = sequence(grammar['component_counts']); indices = sequence(candidate['syntax_indices'])
    require(len(sizes) == len(indices) == 3, 'Joint syntax requires three component names')
    for size, index in zip(sizes, indices):
        claim(f'({natural(index)} : Nat) < {natural(size)}', 'decide')
    product = ' * '.join(natural(n) for n in sizes)
    count, index, look, comparisons = (natural(candidate[k]) for k in
        ('family_count', 'index', 'evidence_look', 'comparison_count'))
    claim(f'({product} : Nat) = {count}')
    claim(f'({index} : Nat) < {count} ∧ {count} ≤ 2^53', 'decide')
    claim(f'({comparisons} : Nat) = {count} * ({look} + 1) * ({look} + 2)')
    claim(f'({comparisons} : Nat) ≤ 2^53', 'decide')

    prefix = 'd' + natural(definition['identity']); cap = natural(definition['semantics']['max_trace'])
    initialization = data['initialization']; fields(initialization, 'program report observation')
    require(initialization['observation'] is None, 'Detailed initialization trace needs an explicit model')
    lines.append(f'def joint_initial : FiniteCode.Code Op Value := {lifecycle.code(initialization["program"])}')
    lines.append(f'def joint_execution := FiniteExecution.execute (apply {cap}) joint_initial ⟨none, 2^64-1⟩ []')
    claim(f'joint_execution.observation = {lifecycle.report(initialization["report"])}',
          'unfold joint_execution joint_initial\n  reduce_execution')
    claim(f'FiniteCode.run (fun op args => (apply {cap} op args).toOption) joint_initial [] = some {prefix}_initial',
          f'unfold joint_initial {prefix}_initial\n  reduce_finite_code')

    transition = data['transition']; fields(transition, 'input before fork_before fork_after live_after')
    register, _, _ = lifecycle.cut_registry({definition['identity']: definition}, claim)
    before, fork_before, fork_after, live, replay = (
        transition['before'], transition['fork_before'], transition['fork_after'], transition['live_after'], data['replay'])
    for cut in (before, fork_before, fork_after, live, replay): register(cut)
    claim(f'ObserverOwnership.fork ({lifecycle.cut(before)} : ObserverOwnership.Cut Nat Nat Nat) '
          f'{natural(fork_before["owner"])} = {lifecycle.cut(fork_before)}')
    distinct(before['owner'], fork_before['owner'])
    same(fork_before['owner'], fork_after['owner']); same(before['owner'], live['owner'])
    for old in (before, fork_before): distinct(replay['owner'], old['owner'])
    for position, new in enumerate((fork_after, live, replay)):
        for old in (before, *(fork_after, live, replay)[:position]): distinct(new['state'], old['state'])

    history = data['history']; lifecycle.history(history)
    rows = sequence(history['events']); require(rows, 'Joint transition requires a retained physical event')
    events = []
    for row in rows:
        source, values = lifecycle.input_row(row)  # Gaps cannot supply an observed transition.
        require(len(row['values']) == 1 and row['values'][0].get('kind') == 'Event',
                'Native joint history requires one complete Event per row')
        event = row['values'][0]['value']
        event_source = {key: event[key] for key in ('epoch', 'raw_space', 'owner')}
        event_source['shape'] = 'Event'
        claim(f'({source} : Source) = {lifecycle.source(event_source)}')
        claim(f'({lifecycle.wide(row["interface"]["epoch"])} : Nat) = {lifecycle.wide(history["origin"]["epoch"])}')
        if row['interface']['owner'] is not None:
            claim(f'({lifecycle.lineage(row["interface"]["owner"])} : Lineage) = {lifecycle.lineage(history["origin"]["lineage"])}')
        events.append(f'({source}, {values})')
    source, values = lifecycle.input_row(transition['input']); actual = f'({source}, {values})'
    claim(f'({events[-1]} : Source × List Value) = {actual}')

    def fold(name, entries, initial, result):
        lines.append(f'def {name} := PartialObservers.fold '
            f'(fun state (event : Source × List Value) => run {cap} {prefix}_update '
            f'{prefix}_update_binding state event.1 event.2) {array(entries)} {initial}')
        initial_definition = f' {prefix}_initial' if initial == f'{prefix}_initial' else ''
        proof = f'unfold {name}{initial_definition} {prefix}_update {prefix}_update_binding\n'
        if not entries: proof += '  rfl'
        else:
            proof += f'  reduce_observer_fold {len(entries)}'
        claim(f'{name} = some {lifecycle.value(result)}', proof)
    fold('joint_before', events[:-1], f'{prefix}_initial', before['value'])
    fold('joint_transition', [actual], lifecycle.value(before['value']), fork_after['value'])
    fold('joint_replay', events, f'{prefix}_initial', replay['value'])
    claim(f'({lifecycle.value(fork_after["value"])} : Value) = {lifecycle.value(live["value"])}')
    claim(f'({lifecycle.value(live["value"])} : Value) = {lifecycle.value(replay["value"])}')
    lines += ['#print axioms ' + name for name in names] + ['end FrozenObserverExport']
    return '\n\n'.join(lines) + '\n', {'joint_producers': 1, 'history_events': len(rows), 'claims': len(names)}


def main():
    replay_main(emit=emit, modules=lifecycle.MODELS,
        producers=[Path(__file__).resolve(), Path(lifecycle.__file__).resolve()], description=__doc__,
        scope='Finite joint initializer/report and before/fork/live/receiver replay; retained nominal receipt '
              'consistency, count/index bounds and paid-look arithmetic. Not syntax-index-to-program '
              'enumeration, admission statistics, default enrollment for arbitrary histories, floating-point '
              'fees, physical provenance or universal native ownership. Original work budgets are not exported.')


if __name__ == '__main__':
    main()
