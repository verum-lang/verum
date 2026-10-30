#!/usr/bin/env python3
"""Replay the bounded MemoryAccount v1 native lifecycle, not allocator/RSS semantics."""
from pathlib import Path
from observer_export_replay import (array, fields, natural, require, sequence,
                                   main as replay_main)

MODELS = ['ResourceLedger']
SCOPE = ('Shallow owner-byte accounting for the eight operations executed by the '
         'native v1 MemoryAccount court. Checks observed quota, held bytes, peak, '
         'five class charges and positive reservation count against composed model '
         'operations. Refusal counters are retained but not modeled. No allocator, '
         'payload-content, pointer-identity, scheduler-liveness or Rust refinement proof.')
EVENTS = ['empty', 'reserve_receipts_16', 'refuse_growth_32_preserve_16',
          'grant_48', 'growth_peak_48_held_32', 'transfer_and_alias',
          'nonfinal_release', 'final_release']


def emit(data):
    fields(data, 'protocol events')
    require(data['protocol'] == 'synarc-owned-memory-lifecycle-v1', 'Unknown memory protocol')
    rows = sequence(data['events'])
    require(len(rows) == len(EVENTS), 'Incomplete v1 lifecycle')
    lines = ['import VerumExternalReplay.ResourceLedger',
             'open ResourceLedger', 'set_option warningAsError true',
             'namespace FrozenObserverExport']
    # Concrete checkpoints are independently verified by each adjacent operation.
    # Do not inline the entire record-update history at every use: that duplicates
    # prior computations and can exhaust the checker on even this short trace.
    checkpoints = [
        ('s0', 24, 0, '[]', None),
        ('s1', 24, 16, '[⟨1,16,1⟩]', 'commit s0 (reserve s0 1 16)'),
        ('s2', 24, 16, '[⟨1,16,1⟩]', 'commit s1 (reserve s1 1 32)'),
        ('s3', 48, 16, '[⟨1,16,1⟩]', 'commit s2 (grant s2 (some 48))'),
        ('grown', 48, 48, '[⟨1,16,1⟩,⟨1,32,1⟩]', 'commit s3 (reserve s3 1 32)'),
        ('s4', 48, 48, '[⟨1,16,0⟩,⟨1,32,1⟩]',
         'commit grown (release grown [] ⟨1,16,1⟩ [⟨1,32,1⟩])'),
        ('s5', 48, 48, '[⟨1,16,0⟩,⟨1,32,2⟩]',
         'commit s4 (alias s4 [⟨1,16,0⟩] ⟨1,32,1⟩ [])'),
        ('s6', 48, 48, '[⟨1,16,0⟩,⟨1,32,1⟩]',
         'commit s5 (release s5 [⟨1,16,0⟩] ⟨1,32,2⟩ [])'),
        ('s7', 48, 48, '[⟨1,16,0⟩,⟨1,32,0⟩]',
         'commit s6 (release s6 [⟨1,16,0⟩] ⟨1,32,1⟩ [])')]
    claims = 0
    for name, quota, peak, slots, operation in checkpoints:
        lines.append(f'def {name} : Ledger := '
                     f'⟨0, 18446744073709551615, some {quota}, {peak}, false, {slots}⟩')
        if operation is not None:
            if name in ('s1', 'grown'):
                before, size = ('s0', 16) if name == 's1' else ('s3', 32)
                proof = (f'  have admitted : admissible {before} {size} = true := by\n'
                         '    simp only [admissible, Bool.and_eq_true, decide_eq_true_eq]\n'
                         '    exact ⟨⟨rfl, Nat.le_of_ble_eq_true rfl⟩, rfl⟩\n'
                         f'  rw [(reserve_acceptance {before} 1 {size}).mpr admitted]\n'
                         '  rfl')
            elif name == 's2':
                proof = ('  rw [reserve_at_capacity s1 1 32 rfl '
                         '(Nat.le_of_ble_eq_true rfl) rfl]\n  rfl')
            else:
                proof = '  decide_cbv'
            lines.append(f'theorem step_{name} : {operation} = {name} := by\n{proof}')
            claims += 1
    for i, (row, event) in enumerate(zip(rows, EVENTS)):
        fields(row, 'event limit held peak classes reservations refusals')
        require(row['event'] == event, 'Unexpected v1 operation order')
        for key in ('limit', 'held', 'peak', 'reservations', 'refusals'):
            natural(row[key])
        classes = sequence(row['classes'])
        require(len(classes) == 5, 'Expected five memory classes')
        encoded = array([natural(value) for value in classes])
        checks = [f's{i}.quota = some {row["limit"]}',
                  f'total s{i}.slots = {row["held"]}',
                  f's{i}.peak = {row["peak"]}',
                  f'[classTotal 0 s{i}.slots, classTotal 1 s{i}.slots, '
                  f'classTotal 2 s{i}.slots, classTotal 3 s{i}.slots, '
                  f'classTotal 4 s{i}.slots] = {encoded}',
                  f'reservations s{i}.slots = {row["reservations"]}']
        lines.append(f'theorem snapshot{i} : ' + ' ∧ '.join(checks) + ' := by decide_cbv')
        claims += 1
    lines.append('end FrozenObserverExport')
    return '\n\n'.join(lines) + '\n', {'claims': claims, 'native_snapshots': len(rows)}


if __name__ == '__main__':
    replay_main(emit=emit, modules=MODELS, producers=[Path(__file__).resolve()],
                scope=SCOPE, description=__doc__)
