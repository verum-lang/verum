"""Synthetic protocol controls, kept separate from original native evidence."""
import copy
import gzip
import json
from pathlib import Path
import tempfile
import unittest
from check_typed_programs import check, require_success, run
from check_retained_execution_export import emit, SEMANTICS


def empty_export():
    limit = {'kind': 'unlimited'}
    initial = {'policy': 'phased', 'originalLimit': limit, 'rootInputs': ['3'], 'poisoned': False,
        'counters': {'dispatch': '0', 'expanded': '0', 'calls': '0'}, 'terminal': None, 'frames': [], 'pending': None}
    active = copy.deepcopy(initial); active['poisoned'] = True
    entered = copy.deepcopy(active); entered['rootInputs'] = None
    entered['frames'] = [{'scope': [], 'pc': '0', 'inputs': {'kind': 'root', 'values': ['3']}, 'registers': []}]
    terminal = copy.deepcopy(entered); terminal['terminal'] = {'stop': 'complete', 'reason': None, 'haltPath': None}
    final = copy.deepcopy(terminal); final['poisoned'] = False
    def event(kind, state, **payload): return {'kind': 'machine', 'event': {'kind': kind, **payload}, 'state': copy.deepcopy(state)}
    trace = [event('service_enter', initial), event('service_active', active),
        event('charge_request', active, units='1', admitted=None),
        {'kind': 'meter', 'units': '1', 'before': '1', 'after': '0', 'ok': True},
        event('charge_result', active, units='1', admitted=True), event('root_enter', entered),
        event('instruction_request', entered, scope=[], pc='0'), event('instruction_result', entered, scope=[], pc='0', decoded=None),
        event('terminal', terminal), event('service_returned', terminal), event('service_leave', final)]
    association = {'program': '1', 'semantics': '2', 'initialInputOwner': '3', 'policy': 'phased', 'originalLimit': limit, 'normalInitialState': True}
    case = {'id': 'empty', 'adapter': 'owned-program-u64-v1', 'fixture': '1',
        'authority': {'program': '1', 'semantics': '2', 'input': '3', 'machine': '4'},
        'association': association, 'policy': 'phased', 'originalLimit': limit,
        'seed': {'kind': 'normal', 'dispatch': '0', 'expanded': '0', 'calls': '0', 'frames': [], 'pending': None},
        'initial': initial, 'services': [{'id': '0', 'budgetOwner': '5', 'budgetKind': 'bounded_work_meter_with_dispatch_grant',
            'grantDispatch': '0', 'before': initial, 'after': final, 'meter': {'before': '1', 'after': '0'},
            'returned': {'kind': 'complete'}, 'trace': trace}],
        'terminalReport': {'stop': 'complete', 'registers': [], 'dispatch': '0', 'expanded': '0', 'calls': '0', 'halt': None}, 'finalPause': None}
    return {'schema': 'synarc-shared-vm-native-v1', 'scope': 'machine', 'semantics': SEMANTICS,
        'fixtures': [{'id': '1', 'program': {'steps': [], 'output': None, 'library': []}, 'inputs': ['3']}],
        'cases': [case], 'associations': []}


class NativeExecutionTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.RetainedNative'))

    def proof(self, data, expected=None):
        text, counts = emit(data)
        with tempfile.TemporaryDirectory(prefix='verum-native-protocol-') as directory:
            p = Path(directory) / 'proof.lean'; p.write_text(text)
            check(p, expected)
        return counts

    def test_empty_native_shaped_protocol(self):
        self.assertEqual(self.proof(empty_export())['claims'], 5)

    def test_guard_and_abort_axioms(self):
        names = ['active_injective', 'abort_retains_poison', 'abort_cannot_resume']
        with tempfile.TemporaryDirectory(prefix='verum-native-axioms-') as directory:
            p = Path(directory) / 'audit.lean'
            p.write_text('import VerumExternalReplay.RetainedNative\n' + '\n'.join('#print axioms RetainedNative.' + n for n in names))
            messages = [json.loads(line)['data'] for line in require_success(run('lake', 'env', 'lean', '--json', str(p))).splitlines()]
            self.assertEqual(len(messages), len(names))
            for message in messages:
                self.assertTrue(message.endswith('depends on axioms: [propext]') or
                                message.endswith('depends on axioms: [propext, Quot.sound]'))

    def test_corrupt_observations_rejected(self):
        edits = [
            (['scope'], 'invocation', 'scope/schema'),
            (['cases', 0, 'authority', 'program'], '6', 'source ownership'),
            (['cases', 0, 'authority', 'machine'], '1', 'owner was reused'),
            (['cases', 0, 'services', 0, 'budgetOwner'], '1', 'budget identity'),
            (['cases', 0, 'services', 0, 'trace', 1, 'state', 'poisoned'], False, 'Poison guard'),
            (['cases', 0, 'services', 0, 'trace', 3, 'after'], '1', 'consumed wrong work'),
            (['cases', 0, 'services', 0, 'trace', 3, 'ok'], False, 'admission changed'),
            (['cases', 0, 'services', 0, 'trace', 6, 'event', 'pc'], '1', 'Unexpected native event'),
            (['cases', 0, 'services', 0, 'trace', 9, 'state', 'poisoned'], False, 'Poison guard'),
            (['cases', 0, 'services', 0, 'meter', 'after'], '1', 'meter balance'),
            (['cases', 0, 'services', 0, 'returned'], {'kind': 'unwind'}, 'service result'),
        ]
        for path, replacement, error in edits:
            with self.subTest(path=path):
                data = copy.deepcopy(empty_export()); target = data
                for key in path[:-1]: target = target[key]
                target[path[-1]] = replacement
                with self.assertRaisesRegex(ValueError, error): emit(data)
        data = empty_export(); del data['cases'][0]['services'][0]['trace'][6]
        with self.assertRaisesRegex(ValueError, 'Unexpected native event'): emit(data)

    def test_incorrect_report_rejected_by_kernel(self):
        data = empty_export(); data['cases'][0]['terminalReport']['registers'] = ['9']
        self.proof(data, 'Tactic `rfl` failed:')

    def test_native_callback_corruption_rejected_by_kernel(self):
        fixture = Path(__file__).parent / 'fixtures/shared-vm-native-v1.json.gz'
        original = json.loads(gzip.decompress(fixture.read_bytes()))
        for kind, key in [('primitive_result', 'result'), ('operand_resolved', 'value')]:
            with self.subTest(kind=kind):
                data = copy.deepcopy(original)
                data['cases'] = [next(c for c in data['cases'] if c['id'] == 'phased_whole')]
                data['associations'] = []
                event = next(e['event'] for e in data['cases'][0]['services'][0]['trace']
                             if e['kind'] == 'machine' and e['event']['kind'] == kind)
                if key == 'result': event[key]['ok'] = str(int(event[key]['ok']) + 1)
                else: event[key] = str(int(event[key]) + 1)
                self.proof(data, 'Tactic `rfl` failed:')

    def test_native_guard_and_unwind_corruption_rejected(self):
        fixture = Path(__file__).parent / 'fixtures/shared-vm-native-v1.json.gz'
        original = json.loads(gzip.decompress(fixture.read_bytes()))
        data = copy.deepcopy(original)
        case = next(c for c in data['cases'] if c['id'] == 'phased_whole')
        entry = next(e for e in case['services'][0]['trace']
                     if e['kind'] == 'machine' and e['event']['kind'] == 'service_active')
        entry['state']['poisoned'] = False
        with self.assertRaisesRegex(ValueError, 'Poison guard'): emit(data)
        data = copy.deepcopy(original)
        case = next(c for c in data['cases'] if c['id'] == 'primitive_unwind')
        service = next(s for s in case['services'] if s['returned']['kind'] == 'unwind')
        service['meter']['after'] = str(int(service['meter']['after']) + 1)
        with self.assertRaisesRegex(ValueError, 'Unwind refunded'): emit(data)


if __name__ == '__main__': unittest.main()
