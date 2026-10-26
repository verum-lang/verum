"""Extraction refuses ambiguous encodings; corrupted observations reach Lean rejection."""
import copy
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from check_observer_export import ROOT, batches, emit, load, unique_object, value
from check_typed_programs import check, sha256

FIXTURE = ROOT / 'fixtures/observer-rust-boundary-v1.json'


def model_controls(data):
    """Synthetic model inputs for extractor tests; never Rust execution evidence."""
    nat = lambda n: {'Nat': n}
    literal = lambda n: {'Literal': nat(n)}
    def program(op, args):
        return {'steps': [{'operation': {'Primitive': op}, 'arguments': args}],
                'output': None, 'library': []}
    def report(stop, regs, dispatch, expanded, reason=None):
        return {'stop': stop, 'registers': regs, 'dispatch_attempts': dispatch,
                'expanded_attempts': expanded, 'call_attempts': 0,
                'halt': None if reason is None else {'path': [0], 'reason': reason}}
    add = program('CheckedAdd', [literal(2), literal(3)])
    budget_stop = report('Budget', [], 0, 0, {'kind': 'Allowance'})
    machines = [
        {'name': 'model-selected', 'kind': 'machine', 'code': add, 'inputs': [],
         'dispatch_limit': 'Unlimited', 'work_limit': 1000, 'work_used': 5,
         'execution': report('Complete', [nat(5)], 1, 1), 'meter_error': None,
         'root': {'status': 'Selected', 'value': nat(5)}},
        {'name': 'model-budget', 'kind': 'machine', 'code': add, 'inputs': [],
         'dispatch_limit': {'Dispatches': 0}, 'work_limit': 1000, 'work_used': 1,
         'execution': budget_stop, 'meter_error': None, 'root': {'status': 'Unattempted'}},
        {'name': 'model-hole', 'kind': 'machine', 'code': {'view': 'declared-arity-operand-hole',
         'program': program('CheckedAdd', [literal(1)]), 'declared_arity': 2}, 'inputs': [],
         'dispatch_limit': 'Unlimited', 'work_limit': 1000, 'work_used': 5,
         'execution': report('Undefined', [], 1, 1, {'kind': 'MissingOperand', 'argument': 1}),
         'meter_error': None, 'root': {'status': 'Unattempted'}},
    ]
    d = copy.deepcopy(data['programs'][0]['description'])
    d['initial'] = nat(0)
    interface = {'descriptor': d['descriptor'], 'ports': ['Key', 'LifeAge']}
    for phase in ('read', 'update'):
        d[phase] = program('PackByte', [{'Input': 0}, {'Input': 1}])
        d[phase + '_binding'] = {**interface, 'width': 2, 'slots': [{'External': 0}, {'External': 1}]}
    complete = report('Complete', [nat(258)], 1, 1)
    common = {'kind': 'observer', 'description': d, 'supplied_interface': interface,
              'inputs': [nat(1), nat(2)]}
    observers = [
        {**common, 'name': 'model-shared', 'dispatch_limit': {'Dispatches': 1}, 'work_limit': 1000,
         'work_used': 17, 'dispatches': 1, 'before': nat(0), 'after': nat(0),
         'read': {'value': nat(258), 'execution': complete}, 'update_refusal': {
             'kind': 'Execution', 'phase': 'Update', 'report': budget_stop,
             'meter_error': None, 'counter_capacity': False}},
        {**common, 'name': 'model-root-work', 'dispatch_limit': 'Unlimited', 'work_limit': 10,
         'work_used': 10, 'dispatches': 1, 'state': nat(0), 'reference': {
             'value': nat(258), 'execution': complete, 'work_used': 11,
             'dispatch_limit': 'Unlimited', 'work_limit': 1000},
         'refusal': {'kind': 'Work', 'phase': 'Read', 'point': 'RootOutput',
                     'error': {'used': 10, 'limit': 10, 'requested': 1}, 'execution': complete}},
        {'name': 'model-invalid-slot', 'kind': 'binding-construction', 'width': 2,
         'slots': [{'External': 2}], 'refusal': {'kind': 'InvalidSlot', 'slot': 0, 'input': 2, 'width': 2}},
    ]
    return {'machine': machines, 'observer': observers}


def control_source(data):
    source = emit(data)[0]
    # Only synthetic edge claims are replayed here. The actual frozen baseline
    # has its own complete replay gate; do not relabel this as foreign evidence.
    parts = [b for b in source.strip().split('\n\n') if not
             b.startswith(('theorem p', 'theorem primitive_', '#print axioms p', '#print axioms primitive_'))]
    return '\n\n'.join(parts) + '\n'


class ExtractionTests(unittest.TestCase):
    def setUp(self):
        self.data = load(FIXTURE)

    def test_frozen_fixture_identity(self):
        receipt = load(FIXTURE.with_suffix('.provenance.json'))
        self.assertEqual(sha256(FIXTURE), receipt['export_sha256'])
        _, counts = emit(self.data)
        self.assertEqual(counts, {'programs': 4, 'trace_rows': 20,
                                  'primitive_cases': 14, 'claims': 74})

    def test_failed_run_replaces_stale_success(self):
        with tempfile.TemporaryDirectory(prefix='observer-report-control-') as tmp:
            report = Path(tmp) / 'report.json'
            report.write_text(json.dumps({'status': 'verified', 'run_id': 'old'}))
            result = subprocess.run([sys.executable, str(ROOT / 'check_observer_export.py'),
                '--export', str(FIXTURE), '--sha256', '0' * 64,
                '--provenance', str(FIXTURE.with_suffix('.provenance.json')),
                '--output', str(Path(tmp) / 'claims.lean'), '--report', str(report)],
                capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 1)
            status = json.loads(report.read_text())
            self.assertEqual(status['status'], 'failed')
            self.assertNotEqual(status['run_id'], 'old')
            self.assertIn('Frozen export hash mismatch', status['error'])

    def test_batches_cover_each_claim_once(self):
        text, counts = emit(self.data)
        names = []
        for part, group in batches(text):
            self.assertLessEqual(len(group), 8)
            self.assertEqual(part.count('theorem '), len(group))
            self.assertEqual(part.count('#print axioms '), len(group))
            self.assertEqual(part.count('def p'), 8)
            names.extend(group)
        self.assertEqual(len(names), counts['claims'])
        self.assertEqual(len(set(names)), counts['claims'])

    def test_ambiguous_numbers_and_values_are_refused(self):
        for obj in [{'Nat': True}, {'Nat': -1}, {'Nat': 2**64}, {'Nat': 1.0},
                    {'Bool': 1}, {'Pair': []}, {'Nat': 0, 'Bool': False}]:
            with self.subTest(obj=obj), self.assertRaises(ValueError):
                value(obj)
        with self.assertRaises(ValueError):
            json.loads('{"Nat":1,"Nat":2}', object_pairs_hook=unique_object)

    def test_unknown_semantics_and_fields_are_refused(self):
        for name, val in [('semantics', 'FutureVersion'), ('schema_version', True),
                          ('schema_version', 2), ('unmodeled_effect', 'write')]:
            data = copy.deepcopy(self.data)
            data['programs'][0]['description'][name] = val
            with self.subTest(name=name), self.assertRaises(ValueError):
                emit(data)

    def test_invalid_binding_and_diagnostic_are_refused(self):
        self.data['programs'][0]['description']['read_binding']['slots'][0] = {'External': 4}
        with self.assertRaises(ValueError):
            emit(self.data)
        self.data = load(FIXTURE)
        self.data['primitives'][1]['error'] = 'Budget'
        with self.assertRaisesRegex(ValueError, 'Unknown refusal'):
            emit(self.data)

    def test_independent_read_descriptor_is_preserved(self):
        d = self.data['programs'][0]['description']
        d['read_binding']['descriptor']['input_epoch'] += 1
        # The export format supplies observations under the definition's own
        # interface. Equality of the two interface descriptors is not a premise.
        emit(self.data)

    def test_corrupted_observations_are_rejected_by_kernel(self):
        data = copy.deepcopy(self.data)
        data['programs'] = data['programs'][:1]
        data['programs'][0]['executions'] = data['programs'][0]['executions'][:1]
        data['primitives'] = data['primitives'][1:2]
        changes = {
            'wrong_initial': lambda d: d['programs'][0]['description']['initial']['Pair'][0].update(Nat=8),
            'wrong_read': lambda d: d['programs'][0]['executions'][0]['read']['Pair'][0].update(Nat=0),
            'wrong_binding': lambda d: d['programs'][0]['description']['read_binding']['slots'].__setitem__(1, {'External': 1}),
            'wrong_error': lambda d: d['primitives'][0].update(error='ZeroDivisor'),
        }
        with tempfile.TemporaryDirectory(prefix='observer-export-controls-') as tmp:
            for name, mutate in changes.items():
                candidate = copy.deepcopy(data)
                mutate(candidate)
                path = Path(tmp) / (name + '.lean')
                path.write_text(emit(candidate)[0])
                with self.subTest(name=name):
                    check(path, 'Tactic `rfl` failed:')

    def test_operational_model_extraction_and_tampering(self):
        data = copy.deepcopy(self.data)
        data['controls_schema'] = 'synarc-observer-execution-controls-v1'
        data['controls'] = model_controls(data)
        changes = {
            'wrong_work': lambda d: d['controls']['machine'][0].update(work_used=4),
            'wrong_stop': lambda d: d['controls']['machine'][1]['execution'].update(stop='Undefined'),
            'hole_as_primitive_error': lambda d: d['controls']['machine'][2]['execution']['halt'].update(
                reason={'kind': 'Primitive', 'error': {'kind': 'Arity', 'operation': 'CheckedAdd', 'expected': 2, 'actual': 1}}),
            'reset_shared_allowance': lambda d: d['controls']['observer'][0].update(dispatches=2),
            'root_work_refund': lambda d: d['controls']['observer'][1].update(work_used=9),
        }
        with tempfile.TemporaryDirectory(prefix='observer-operational-controls-') as tmp:
            path = Path(tmp) / 'model.lean'
            path.write_text(control_source(data))
            check(path)
            for name, mutate in changes.items():
                candidate = copy.deepcopy(data)
                mutate(candidate)
                path = Path(tmp) / (name + '.lean')
                path.write_text(control_source(candidate))
                with self.subTest(name=name):
                    check(path, 'Tactic `rfl` failed:')

    def test_ambiguous_control_limits_and_views_are_refused(self):
        data = copy.deepcopy(self.data)
        data['controls_schema'] = 'synarc-observer-execution-controls-v1'
        data['controls'] = model_controls(data)
        data['controls']['observer'][0]['dispatch_limit'] = 1
        with self.assertRaises(ValueError):
            emit(data)
        data['controls'] = model_controls(data)
        data['controls']['machine'][2]['code']['view'] = 'unrecognized-executable-view'
        with self.assertRaises(ValueError):
            emit(data)


if __name__ == '__main__':
    unittest.main()
