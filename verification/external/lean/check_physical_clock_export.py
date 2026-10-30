#!/usr/bin/env python3
"""Replay source-bound native clock handoffs, without claiming world reset or periodicity."""
from pathlib import Path
from observer_export_replay import (MAX64, array, boolean, fields, natural, require,
                                   sequence, variant, main as replay_main)

MODELS = ['PhysicalClock']
SCOPE = ('Finite physical-clock and projected Organism restart correspondence. '
         'Origins are names from retained native Arc equality; constructor authority '
         'and truthful physical capture remain source/provenance obligations. '
         'Injected arithmetic states and omitted pre-handoff execution are explicit. '
         'No world-reset, finite-factor, allocator, liveness or whole-Rust refinement proof.')
PHASES = {'ready', 'inFlight', 'terminalPending'}
KINDS = {'Death': 'death', 'Time': 'time', 'Completed': 'completed', 'Ended': 'ended'}
REFUSALS = {name[0].upper() + name[1:]: name for name in
            ['missingEpisode', 'ageOverflow', 'pendingOutcome', 'pendingTerminal',
             'undeclaredTime', 'foreignPoint']}
LIFECYCLE = {name[0].upper() + name[1:]: name for name in
             ['pendingBoundary', 'unknownDelivery', 'pendingPhysicalTime', 'pendingTerminalClosure']}


def option(value, encode):
    return 'none' if value is None else '(some ' + encode(value) + ')'


def status(value, reasons=REFUSALS):
    require(type(value) is str, 'Expected typed native status')
    if value == 'Ok(())':
        return '.ok ()'
    require(value.startswith('Err(') and value.endswith(')') and value[4:-1] in reasons,
            'Unknown native refusal')
    return '.error .' + reasons[value[4:-1]]


def emit(data):
    fields(data, 'protocol case maximum initialization origin_count sample_count steps')
    require(data['protocol'] == 'synarc-physical-clock-v1', 'Unknown clock protocol')
    require(type(data['case']) is str and data['case'], 'Missing case identity')
    require(type(data['maximum']) is int and data['maximum'] == MAX64, 'Expected exact u64 maximum')
    initialization = data['initialization']
    require(initialization in ('ordinary', 'injected-age-boundary', 'injected-episode-boundary',
                              'actual-dispatch-boundary-with-injected-bookkeeping'),
            'Unknown initialization scope')
    natural(data['origin_count']); natural(data['sample_count'])
    rows = sequence(data['steps'])
    require(rows, 'Empty native lifecycle')
    origins, samples, live, retired, receipts = set(), set(), {}, set(), set()
    lines = ['import VerumExternalReplay.PhysicalClock', 'open PhysicalClock',
             'set_option warningAsError true', 'namespace FrozenObserverExport',
             'def maximum : Nat := 18446744073709551615',
             'def Projection := Option (List Nat) × List Nat × Option (Nat × Nat) × Nat',
             'def resetProjection (p : Projection) : Projection := (none, [], none, p.2.2.2)']
    claims = 0

    def claim(equation, proof='rfl'):
        nonlocal claims
        lines.append(f'theorem c{claims} : {equation} := by\n  {proof}')
        claims += 1

    def origin(value):
        encoded = natural(value)
        require(value < data['origin_count'], 'Origin outside declared names')
        origins.add(value)
        return encoded

    def time(value):
        fields(value, 'episode age')
        return '⟨' + natural(value['episode']) + ',' + natural(value['age']) + '⟩'

    def point(value):
        fields(value, 'origin time')
        return '⟨' + origin(value['origin']) + ',' + time(value['time']) + '⟩'

    def clock(value):
        fields(value, 'origin episode age phase')
        require(type(value['phase']) is str and value['phase'] in PHASES, 'Unknown clock phase')
        return '(⟨' + origin(value['origin']) + ',' + option(value['episode'], natural) + ',' + \
            natural(value['age']) + ',.' + value['phase'] + '⟩ : Clock Nat)'

    def receipt(value):
        fields(value, 'point sample transition successor')
        kind, argument = variant(value['transition'])
        require((kind == 'tick' and argument is None) or
                (kind == 'terminal' and type(argument) is str and argument in KINDS),
                'Unknown physical transition')
        if value['sample'] is not None:
            natural(value['sample'])
            require(value['sample'] < data['sample_count'], 'Sample outside declared names')
            samples.add(value['sample'])
        transition = '.tick' if kind == 'tick' else '(.terminal .' + KINDS[argument] + ')'
        return '(⟨' + point(value['point']) + ',' + option(value['sample'], natural) + ',' + \
            transition + ',' + option(value['successor'], time) + '⟩ : Receipt Nat)'

    def journal(value):
        if value is None:
            return 'none'
        fields(value, 'pending_boundary history_known')
        return '(some ⟨' + boolean(value['pending_boundary']) + ',' + boolean(value['history_known']) + '⟩)'

    def payload(value):
        fields(value, 'clock journal failure plan pending_goal calls')
        def failure(text):
            require(type(text) is str and len(text.encode()) <= 4096, 'Invalid failure projection')
            return array([str(byte) for byte in text.encode()])
        def goal(pair):
            require(len(sequence(pair)) == 2, 'Invalid pending goal')
            return '(' + natural(pair[0]) + ',' + natural(pair[1]) + ')'
        return '((' + option(value['failure'], failure) + ',' + \
            array([natural(x, 255) for x in sequence(value['plan'])]) + ',' + \
            option(value['pending_goal'], goal) + ',' + natural(value['calls']) + ') : Projection)'

    for index, row in enumerate(rows):
        fields(row, 'operation input before after result')
        op, arg, result = row['operation'], row['input'], row['result']
        before = row['before']['clock'] if op == 'begin_run' else row['before']
        after = row['after']['clock'] if op == 'begin_run' else row['after']
        previously_named = set(origins)
        b, a = clock(before), clock(after)
        owner = before['origin']
        require(owner not in retired, 'Retired clock origin reused')
        if owner in live:
            require(live[owner] == before, 'Disconnected before-state')
        elif index == 0 and initialization != 'ordinary':
            # An exported checkpoint is not a proof of the omitted prefix.
            if initialization == 'injected-age-boundary':
                require(before['episode'] == 0 and before['age'] == MAX64 and before['phase'] == 'ready',
                        'Invalid injected age boundary')
            elif initialization == 'injected-episode-boundary':
                require(before['episode'] == MAX64 and before['age'] == 0 and before['phase'] == 'ready',
                        'Invalid injected episode boundary')
        else:
            require(before['episode'] == 0 and before['age'] == 0 and before['phase'] == 'ready',
                    'Undeclared initial clock state')

        if op in ('capture', 'unconfirmed_capture'):
            fields(arg, 'point outcome' if op == 'capture' else 'point foreign_action')
            selected = point(arg['point'])
            claim(f'point maximum {b} = .ok {selected}')
            claim(f'start maximum {b} {selected} = ({a}, .ok ())')
            if op == 'capture':
                fields(result, 'receipt')
                actual = receipt(result['receipt'])
                outcome, body = variant(arg['outcome'])
                if outcome in ('step', 'refused'):
                    require(body is None, 'Unexpected nonterminal payload')
                    transition = '.tick'
                else:
                    require(outcome == 'terminal', 'Unknown observed outcome')
                    fields(body, 'kind resumption')
                    require(body['kind'] in KINDS and body['resumption'] in ('Stopped', 'Resumed'),
                            'Unknown terminal observation')
                    transition = '(.terminal .' + KINDS[body['kind']] + ')'
                claim(f'observe maximum {selected} {option(result["receipt"]["sample"], natural)} '
                      f'{transition} = some {actual}')
                require(actual not in receipts, 'Duplicate live receipt')
                receipts.add(actual)
            else:
                boolean(arg['foreign_action']); fields(result, 'receipt calls')
                require(result['receipt'] is None, 'Unconfirmed delivery issued a receipt')
                claim(f'(1 : Nat) = {natural(result["calls"])}')
        elif op == 'commit':
            fields(arg, 'receipt'); actual = receipt(arg['receipt'])
            require(actual in receipts, 'Receipt is absent or already consumed')
            require(type(result) is dict, 'Expected commit result')
            tag, value = variant(result) if set(result) == {'ok'} else ('error', None)
            if tag == 'ok':
                require(value is None, 'Unexpected commit payload')
                claim(f'commit {b} {actual} = some ({a}, .ok ())')
                receipts.remove(actual)
            else:
                fields(result, 'error retained')
                require(result['error'] == 'ForeignPoint', 'Unexpected commit refusal')
                kept = receipt(result['retained'])
                claim(f'commit {b} {actual} = some ({a}, .error (.foreignPoint, {kept}))')
        elif op in ('restart', 'begin_run'):
            fields(arg, 'receipt_before'); fields(result, 'status receipt_after')
            if result['status'] == 'Ok(())':
                require(after['origin'] != owner and after['origin'] not in previously_named,
                        'Successful restart reused an origin')
                claim(f'({after["origin"]} : Nat) ≠ {owner}', 'decide_cbv')
            old = option(arg['receipt_before'], receipt)
            kept = option(result['receipt_after'], receipt)
            claim(f'({old} : Option (Receipt Nat)) = {kept}')
            if arg['receipt_before'] is not None:
                owned = receipt(arg['receipt_before'])
                if index == 0 and initialization == 'actual-dispatch-boundary-with-injected-bookkeeping':
                    receipts.add(owned)
                    claim(f'owns {b} {owned}', 'decide_cbv')
                require(owned in receipts, 'Unknown retained receipt')
            expected = status(result['status'], LIFECYCLE if op == 'begin_run' else REFUSALS)
            if op == 'restart':
                claim(f'restart maximum {after["origin"]} {b} = ({a}, {expected})')
            else:
                bp, ap = payload(row['before']), payload(row['after'])
                bj, aj = journal(row['before']['journal']), journal(row['after']['journal'])
                claim(f'({bj} : Option JournalGate) = {aj}')
                claim(f'beginRun maximum {after["origin"]} {bj} {b} {bp} resetProjection = '
                      f'(({a}, {ap}), {expected})')
        elif op == 'close_terminal':
            require(arg is None, 'Unexpected close input')
            claim(f'closeTerminal maximum {b} = ({a}, {status(result)})')
        elif op in ('point_refused', 'dispatch_refused'):
            require(arg is None, 'Unexpected refused-dispatch input')
            if op == 'dispatch_refused':
                fields(result, 'error calls'); reason = result['error']
                claim(f'(0 : Nat) = {natural(result["calls"])}')
            else:
                reason = result
            require(type(reason) is str and reason in REFUSALS, 'Unknown point refusal')
            claim(f'point maximum {b} = .error .{REFUSALS[reason]}')
            claim(f'{a} = {b}')
        else:
            raise ValueError('Unknown clock operation')

        if after['origin'] != owner:
            require(op in ('restart', 'begin_run') and result['status'] == 'Ok(())'
                    and after['origin'] not in previously_named,
                    'Restart did not allocate a fresh origin')
            retired.add(owner); live.pop(owner, None)
        live[after['origin']] = after

    require(len(origins) == data['origin_count'] and sorted(origins) == list(range(len(origins))),
            'Incomplete origin naming table')
    require(len(samples) == data['sample_count'] and sorted(samples) == list(range(len(samples))),
            'Incomplete sample naming table')
    lines.append('end FrozenObserverExport')
    return '\n\n'.join(lines) + '\n', {'claims': claims, 'native_steps': len(rows)}


if __name__ == '__main__':
    replay_main(emit=emit, modules=MODELS, producers=[Path(__file__).resolve()],
                scope=SCOPE, description=__doc__)
