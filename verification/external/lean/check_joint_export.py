#!/usr/bin/env python3
"""Replay actual joint initialization and transitions; acquisition admission is separate."""
from pathlib import Path
import json
import re
import check_lifecycle_export as lifecycle
from observer_export_replay import array, fields, natural, require, sequence, main as replay_main


def decimal(value):
    require(type(value) is str and re.fullmatch(r'0|[1-9][0-9]*', value) is not None,
            'Expected canonical exact decimal')
    return value


def exact_integer(value):
    # Match the native serializer: small JSON integers, otherwise canonical
    # decimal objects. Never pass through float or a machine-sized rank.
    if type(value) is int:
        return natural(value, 2**53)
    fields(value, 'exact_decimal')
    result = decimal(value['exact_decimal'])
    require(int(result) > 2**53, 'Noncanonical small exact-decimal object')
    return result


def name_prior(value):
    require(type(value) is str, 'Expected explicit native naming prior')
    uniform = re.fullmatch(r'Uniform \{ original_cardinality: (0|[1-9][0-9]*) \}', value)
    if uniform:
        return 'Uniform', uniform[1]
    kraft = re.fullmatch(r'KraftProgramV1 \{ bits: (0|[1-9][0-9]*) \}', value)
    require(kraft is not None, 'Unknown naming prior')
    return 'KraftProgramV1', natural(int(kraft[1]))


def receipt(item, modern=False):
    fields(item, 'owner authority count index look comparison_count' + (' name_prior' if modern else ''))
    for key in ('owner', 'authority'): natural(item[key])
    integer = exact_integer if modern else natural
    for key in ('index', 'look', 'comparison_count'): integer(item[key])
    if item['count'] is not None or not modern: integer(item['count'])
    if modern: name_prior(item['name_prior'])
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
    require(type(candidate) is dict, 'Expected candidate')
    modern = 'structural_code_words' in candidate
    fields(candidate, 'owner definition grammar index syntax_indices family_count evidence_look comparison_count provenance' +
           (' structural_code_words' if modern else ''))
    natural(candidate['owner'])  # Nominal token; no semantic-equivalence inference.
    same(candidate['definition'], definition['identity'])
    grammar = candidate['grammar']
    fields(grammar, 'owner authority count component_counts' + (' source_order admission_profile' if modern else ''))
    natural(grammar['owner']); natural(grammar['authority'])
    origin = receipt(candidate['provenance'], modern); sealed = receipt(data['definition_receipt'], modern)
    integer = exact_integer if modern else natural
    def equal_integer(a, b):
        claim(f'({integer(a)} : Nat) = {integer(b)}')
    def equal_count(a, b):
        if a is None or b is None:
            require(modern and a is None and b is None, 'Inconsistent unbounded family')
        else: equal_integer(a, b)
    same(origin['owner'], sealed['owner']); same(origin['authority'], sealed['authority'])
    equal_count(origin['count'], sealed['count'])
    for key in ('index', 'look', 'comparison_count'): equal_integer(origin[key], sealed[key])
    same(origin['authority'], grammar['authority'])
    equal_count(origin['count'], grammar['count']); equal_count(candidate['family_count'], grammar['count'])
    for field, supplied in [('index', 'index'), ('look', 'evidence_look'), ('comparison_count', 'comparison_count')]:
        equal_integer(origin[field], candidate[supplied])
    sizes = sequence(grammar['component_counts']); indices = sequence(candidate['syntax_indices'])
    require(len(sizes) == len(indices) == 3, 'Joint syntax requires three component names')
    for i in indices: natural(i)
    structural = modern and grammar['source_order'] == 'structural-odometer-v1'
    words = []
    if structural:
        require(sizes == [None, None, None], 'Structural catalogues do not export component cardinalities')
        words = sequence(candidate['structural_code_words'])
        require(len(words) == 3, 'Expected all three structural code words')
        for word in words:
            require(sequence(word), 'Empty structural code word')
            for bit in word: natural(bit, 1)
        encoded = array(array('true' if bit else 'false' for bit in word) for word in words)
        # Retain the full words. No invented grammar or word-to-Program witness:
        # this export does not include the catalogues needed for that proof.
        lines.append(f'def joint_structural_words : List (List Bool) := {encoded}')
        claim('joint_structural_words.length = 3')
    else:
        if modern:
            require(grammar['source_order'] == 'legacy-indexed-v1', 'Unknown joint source order')
            require(candidate['structural_code_words'] is None, 'Indexed source cannot carry structural words')
        for size, i in zip(sizes, indices):
            claim(f'({natural(i)} : Nat) < {natural(size)}', 'decide')
        product = ' * '.join(natural(n) for n in sizes)
        claim(f'({product} : Nat) = {integer(candidate["family_count"])}')
    index, look, comparisons = (integer(candidate[k]) for k in ('index', 'evidence_look', 'comparison_count'))
    count = None if candidate['family_count'] is None else integer(candidate['family_count'])
    if modern:
        prior, parameter = name_prior(origin['name_prior'])
        name_prior(sealed['name_prior'])
        claim(f'({json.dumps(origin["name_prior"])} : String) = {json.dumps(sealed["name_prior"])}')
        profile = grammar['admission_profile']
        require(type(profile) is str, 'Expected explicit admission profile')
        if profile != 'LegacyChi2V1':
            risk = re.fullmatch(r'ExactPredictiveV1 \{ risk: Mass \{ denominator: (0|[1-9][0-9]*) \} \}', profile)
            require(risk is not None, 'Unknown admission profile')
            claim(f'(0 : Nat) < {risk[1]}', 'decide')
        lines.append(f'def joint_admission_profile : String := {json.dumps(profile)}')
        if prior == 'Uniform':
            require(count is not None, 'Uniform naming requires its original cardinality')
            claim(f'({parameter} : Nat) = {count}')
            denominator = count
        else:
            require(structural and count is None, 'Kraft naming requires structural words and no uniform count')
            lengths = ' + '.join(str(len(word)) for word in words)
            claim(f'({parameter} : Nat) = {lengths}')
            denominator = f'(2 ^ ({lengths}))'
    else:
        require(count is not None, 'Legacy source requires finite cardinality')
        denominator = count
        claim(f'({count} : Nat) ≤ 2^53', 'decide')
        claim(f'({comparisons} : Nat) ≤ 2^53', 'decide')
    if count is not None: claim(f'({index} : Nat) < {count}', 'decide')
    claim(f'({comparisons} : Nat) = {denominator} * ({look} + 1) * ({look} + 2)')

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
    return '\n\n'.join(lines) + '\n', {'joint_producers': 1, 'history_events': len(rows), 'structural_words': len(words),
        'structural_program_bindings': 0, 'claims': len(names)}


def main():
    replay_main(emit=emit, modules=lifecycle.MODELS,
        producers=[Path(__file__).resolve(), Path(lifecycle.__file__).resolve()], description=__doc__,
        scope='Finite joint initializer/report and before/fork/live/receiver replay; retained nominal receipt '
              'consistency, exact Uniform/Kraft charge arithmetic and retained structural words. Not structural-word '
              'decoding/materialization or syntax-index-to-program enumeration, admission statistics, default enrollment, floating-point '
              'fees, physical provenance or universal native ownership. Original work budgets are not exported.')


if __name__ == '__main__':
    main()
