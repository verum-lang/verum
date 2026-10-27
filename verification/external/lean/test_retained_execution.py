"""Kernel proofs and adversarial controls for the retained VM model.

These controls are deliberately independent of native exports; they cannot certify
an unobserved foreign implementation or callback purity.
"""
import json
from pathlib import Path
import tempfile
import unittest
from check_typed_programs import check, require_success, run

HEADER = '''import VerumExternalReplay.RetainedExecutionControls
open RetainedExecution RetainedExecutionControls
set_option warningAsError true
'''


class RetainedExecutionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.RetainedExecutionControls'))

    def test_no_added_axioms(self):
        names = '''Trace.append Trace.refine Trace.terminal_unique charge_partition
            gate_work_sufficient gate_dispatch_sufficient admitted_accounting
            next_preserves_authority next_monotone terminal_stopped poisoned_cannot_resume
            tick_refines trace_monotone split_accounting tick_trace run_trace
            tick_accounting run_accounting trace_authority services_trace
            completed_services_unique'''.split()
        with tempfile.TemporaryDirectory(prefix='verum-retained-vm-axioms-') as directory:
            p = Path(directory) / 'audit.lean'
            p.write_text(HEADER + '\n'.join('#print axioms RetainedExecution.' + n for n in names))
            messages = [json.loads(line)['data'] for line in
                        require_success(run('lake', 'env', 'lean', '--json', str(p))).splitlines()]
            self.assertEqual(len(messages), len(names))
            for message in messages:
                if 'depends on axioms: [' in message:
                    axioms = message.split('depends on axioms: [')[1].removesuffix(']').split(', ')
                    self.assertLessEqual(set(axioms), {'propext', 'Quot.sound'})
                else:
                    self.assertTrue(message.endswith('does not depend on any axioms'))

    def test_invalid_execution_claims_rejected(self):
        claims = {
            'atomic_grants_do_not_accumulate': '(sliced .atomic 17 start).state = (whole .atomic).state',
            'unfinished_body_not_complete': '(sliced .phased 16 start).state = (whole .phased).state',
            'selected_output_not_last': '(whole .phased).state.control.frames.head?.bind (selected code) = some 8',
            'child_registers_not_root_registers': 'observation (whole .phased).state = some ⟨.complete, [7, 99], 4, 3, 1, none⟩',
            'no_dispatch_rebilling': 'third.state.usage.dispatch = 2',
            'resolved_prefix_kept': 'third.state.control.pending = some ⟨[], 0, .call (some [0]), 1, [], .reserveReturn⟩',
            'frame_inputs_kept': 'sixth.state.control.frames = [⟨[0], .call, [], []⟩, ⟨[], .root, [5], []⟩]',
            'no_parent_register_before_return': 'sixth.state.control.frames = [⟨[0], .call, [5], []⟩, ⟨[], .root, [5], [5]⟩]',
            'counter_gate_before_work': '(tick code apply .phased fullCounter ⟨0, 1⟩).status = .paused .work',
            'original_bound_retained': '(run code apply .phased 40 (initial [5] (some 1)) ⟨100, 100⟩).state.control.terminal.isSome = true',
            'failed_operand_is_not_success': 'observation (finish (single ⟨.primitive true, [.input 1], none⟩)) = some ⟨.complete, [5], 1, 1, 0, none⟩',
            'atomic_prepaid_work_not_refunded': '(finish (single ⟨.primitive true, [.input 1], none⟩) .atomic).usage.work = 3',
            'poison_cannot_resume': '(run code apply .phased 40 { start with control.poisoned := true } ⟨100, 100⟩).state = (whole .phased).state',
            'call_output_failure_path_preserved': 'observation (finish (single ⟨.call 0, [], none⟩ [.mk [] none []])) = some ⟨.undefined, [], 1, 0, 1, some ⟨[0], .missingCalleeOutput⟩⟩',
            'callbacks_cannot_be_erased': '(sliced .phased 17 start).events = []',
        }
        with tempfile.TemporaryDirectory(prefix='verum-retained-vm-negative-') as directory:
            for name, claim in claims.items():
                with self.subTest(name=name):
                    p = Path(directory) / f'{name}.lean'
                    p.write_text(HEADER + f'example : {claim} := by decide\n')
                    check(p, 'Tactic `decide` proved that the proposition')


if __name__ == '__main__':
    unittest.main()
