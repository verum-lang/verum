"""Replay original publication fork/read/commit observations, never inferred events."""
import check_lifecycle_export as lifecycle
from observer_export_replay import array, fields, natural, require, sequence


def emit(data, definition, prefix, cap, claim):
    events = sequence(data['publication_events'])
    require(events, 'Missing actual publication trace')
    register, owners, states = lifecycle.cut_registry({definition['identity']: definition}, claim)
    initial = data['installation_cut']; register(initial)
    current = initial
    tail = data['current_history']['events'][data['installation_origin_cut']:]
    commits = reads = forks = 0
    awaiting_read = False

    def same_cut(a, b):
        claim(f'({lifecycle.cut(a)} : ObserverOwnership.Cut Nat Nat Nat) = {lifecycle.cut(b)}')
        claim(f'({lifecycle.value(a["value"])} : Value) = {lifecycle.value(b["value"])}')

    def execute(phase, cut, row, result, report):
        src, values = lifecycle.input_row(row)
        binding = definition[phase + '_binding']
        require(row['interface'] == binding['interface'], 'Publication input has foreign interface')
        kind = 'Event' if phase == 'update' else 'Query'
        require(len(row['values']) == 1 and row['values'][0]['kind'] == kind,
                'Publication requires its complete typed input')
        payload = row['values'][0]['value']
        require(all(payload[k] == row['interface'][k] for k in ('owner', 'epoch', 'raw_space')),
                'Publication input payload/source mismatch')
        value = lifecycle.value(cut['value'])
        claim(f'run {cap} {prefix}_{phase} {prefix}_{phase}_binding {value} {src} {values} = '
              f'some {lifecycle.value(result)}', '(conv => lhs; cbv) <;> rfl')
        # No native work budget is supplied. Check the complete body's exposed
        # report under the model ceiling; do not invent native meter readings.
        encoded_report = lifecycle.report(report)
        require(report['stop'] == 'Complete', 'Publication must retain completed body report')
        claim(f'((FiniteCode.bind {natural(binding["width"])} {prefix}_{phase}_binding.slots {value} {values}).map '
              f'(fun inputs => (FiniteExecution.execute (apply {cap}) {prefix}_{phase} '
              f'⟨none, FiniteExecution.capacity⟩ inputs).observation)) = some {encoded_report}',
              '(conv => lhs; cbv) <;> rfl')

    for event in events:
        require(type(event) is dict and 'kind' in event, 'Expected publication event')
        kind = event['kind']
        if kind == 'Fork':
            fields(event, 'kind before after')
            require(forks == 0, 'Duplicate publication fork')
            before, after = event['before'], event['after']
            register(before); same_cut(before, current)
            claim(f'({array(natural(o) for o in owners)} : List Nat).contains {natural(after["owner"])} = false', 'decide')
            claim(f'ObserverOwnership.fork ({lifecycle.cut(before)} : ObserverOwnership.Cut Nat Nat Nat) '
                  f'{natural(after["owner"])} = {lifecycle.cut(after)}')
            register(after); current = after
            forks += 1; awaiting_read = True
        elif kind == 'Read':
            fields(event, 'kind query cut value execution')
            require(forks == 1, 'Read precedes publication fork')
            register(event['cut']); same_cut(event['cut'], current)
            execute('read', current, event['query'], event['value'], event['execution'])
            reads += 1; awaiting_read = False
        else:
            require(kind == 'Commit', 'Unknown publication event')
            require(not awaiting_read, 'Missing first publication read')
            fields(event, 'kind input receipt execution'); fields(event['receipt'], 'before after')
            before, after = event['receipt']['before'], event['receipt']['after']
            register(before); same_cut(before, current)
            require(commits < len(tail), 'Commit exceeds retained current history')
            src, values = lifecycle.input_row(event['input']); expected_src, expected_values = lifecycle.input_row(tail[commits])
            claim(f'(({src}, {values}) : Source × List Value) = ({expected_src}, {expected_values})')
            execute('update', before, event['input'], after['value'], event['execution'])
            claim(f'({array(natural(s) for s in states)} : List Nat).contains {natural(after["state"])} = false', 'decide')
            register(after)
            # Replay the successful receipt boundary; this does not manufacture
            # an observed Preparing cursor or a refused commit observation.
            preparation = '⟨' + lifecycle.cut(before) + ', ' + natural(after['state']) + '⟩'
            claim(f'ObserverOwnership.commit ({lifecycle.cut(current)} : ObserverOwnership.Cut Nat Nat Nat) '
                  f'{preparation} = ⟨{lifecycle.cut(after)}, .ok ⟨{lifecycle.cut(before)}, {lifecycle.cut(after)}⟩⟩')
            current = after; commits += 1
    require(forks == 1 and reads > 0 and not awaiting_read, 'Missing actual publication fork or read')
    require(commits == len(tail), 'Missing native commits for current history tail')
    register(data['current_cut']); same_cut(current, data['current_cut'])
    return {'native_owner_transition_witnesses': 1 + commits, 'publication_reads': reads,
            'publication_commits': commits, 'publication_events': len(events)}
