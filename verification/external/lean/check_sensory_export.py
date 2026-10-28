#!/usr/bin/env python3
"""Replay frozen sensory lifecycle operations and complete physical event folds."""
from pathlib import Path
from acquired_export_codec import SENSORY
import check_lifecycle_export as lifecycle
import observer_trace_proofs
from observer_export_replay import array, fields, natural, require, sequence, main as replay_main


def emit(data):
    fields(data, 'schema definitions ownership naming installation refusals physical_replay')
    require(data['schema'] == 'synarc-acquired-sensory-lifecycle-v1', 'Unknown sensory lifecycle schema')
    definitions = sequence(data['definitions'])
    # This schema links the physical replay through the first installation;
    # it carries no independent definition selector or physical owner IDs.
    require(len(definitions) == 1 and definitions[0]['semantics']['kind'] == 'AcquiredV3',
            'Physical sensory replay requires one explicit AcquiredV3 definition')
    base = {key: data[key] for key in ('definitions', 'ownership', 'naming', 'installation', 'refusals')}
    base['schema'] = 'synarc-observer-lifecycle-v1'
    source, counts = lifecycle.emit(base, decoder=SENSORY, installation_format='retained-histories')
    installations = sequence(data['installation'])
    require(installations and installations[0]['kind'] == 'success', 'Missing initial successful installation')
    installed = installations[0]
    require(installed['installed']['definition'] == definitions[0]['identity'], 'Physical definition mismatch')
    replay = data['physical_replay']
    fields(replay, 'events installed_before live_after replay_after')
    events = sequence(replay['events'])
    cut = int(natural(installed['cut']))
    require(cut < len(events), 'Physical replay must contain the installed prefix and actual continuation')
    prefix = 'd' + natural(definitions[0]['identity'])
    cap = natural(definitions[0]['semantics']['max_trace'])
    lines = ['import VerumExternalReplay.SensoryPort', source.removesuffix('end FrozenObserverExport\n')]
    names = []

    def claim(equation, proof='rfl'):
        name = 'sensory_' + str(len(names)); names.append(name)
        lines.append(f'theorem {name} : {equation} := by\n  {proof}')

    encoded = []
    for row in events:
        interface, values = SENSORY.input_row(row)
        require(len(row['values']) == 1 and row['values'][0]['kind'] == 'Event',
                'Physical history requires the complete captured event')
        event = SENSORY.event(row['values'][0]['value'])
        # Prove coherence on actual physical rows only. Refusal observations
        # above retain forged/mismatched metadata exactly as supplied.
        claim(f'SensoryPort.eventSource ({event}) = ({interface} : Source)')
        encoded.append(f'({interface}, {values})')
    lines.append('def physical_events : List (Source × List Value) := ' + array(encoded))
    step = f'(fun state (event : Source × List Value) => run {cap} {prefix}_update '
    step += f'{prefix}_update_binding state event.1 event.2)'
    lines.append(f'def physical_step := {step}')
    for field in ('installed_before', 'live_after', 'replay_after'):
        lines.append(f'def physical_{field} : Value := {SENSORY.value(replay[field])}')
    claim('physical_installed_before = ' + SENSORY.value(installed['installed']['value']))
    claim(f'(physical_events.take {cut}).map some = '
          f'({SENSORY.history(installed["history"])} : ObserverInstallation.History '
          f'(Lineage × Nat × Nat) (Source × List Value)).events.take {cut}')
    # The same full source/event values feed both complete and suffix folds.
    unfold = (f'unfold physical_step {prefix}_update {prefix}_update_binding\n'
              '  unfold AcquiredObserver.run\n'
              '  simp [FiniteCode.bind, List.mapM, List.mapM.loop]\n  reduce_finite_code')
    for rows, initial, final in (
            ('physical_events', f'{prefix}_initial', 'physical_replay_after'),
            (f'(physical_events.drop {cut})', 'physical_installed_before', 'physical_live_after')):
        reductions = 'List.drop, ' if rows.startswith('(') else ''
        claim(f'PartialObservers.fold physical_step {rows} {initial} = some {final}',
              f'unfold physical_events {initial} {final}\n'
              f'  simp only [{reductions}PartialObservers.fold]\n  {unfold}')
    claim('physical_live_after = physical_replay_after')
    lines += ['#print axioms '+name for name in names] + ['end FrozenObserverExport']
    source, shared = observer_trace_proofs.share_traces(data, '\n\n'.join(lines)+'\n', decoder=SENSORY)
    return source, {**counts, 'physical_events': len(events), 'physical_continuation': len(events)-cut,
                    'claims': counts['claims']+len(names)+shared}


if __name__ == '__main__':
    replay_main(emit=emit, modules=lifecycle.MODELS + ['TraceReduction', 'SensoryPort'],
        producers=[Path(__file__).resolve(), *lifecycle.PRODUCERS,
                   Path(observer_trace_proofs.__file__).resolve()],
        scope='Exact finite sensory lifecycle operations, full physical event folds and supplied semantic states. Physical replay has no separate owner identities; it does not certify device capture timing, allocator ownership, grammar enumeration, general executions or residual arena integration.',
        description=__doc__)
