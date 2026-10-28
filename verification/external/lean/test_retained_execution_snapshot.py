"""Extraction-boundary controls; synthetic snapshots are not native evidence."""
import copy
import unittest
from retained_execution_snapshot import MAX64, nat, program, snapshot

SOURCE = program({'steps': [{'operation': {'call': '0'}, 'arguments': [{'input': '0'}]}],
    'output': None, 'library': [{'steps': [{'operation': {'primitive': 'add'},
    'arguments': [{'input': '0'}, {'literal': '2'}]}], 'output': '0', 'library': []}]})
BASE = {'policy': 'phased', 'originalLimit': {'kind': 'dispatches', 'max': '3'},
    'rootInputs': None, 'poisoned': True, 'counters': {'dispatch': '2', 'expanded': '1', 'calls': '1'},
    'terminal': None, 'frames': [
        {'scope': [], 'pc': '0', 'inputs': {'kind': 'root', 'values': ['5']}, 'registers': []},
        {'scope': ['0'], 'pc': '0', 'inputs': {'kind': 'call', 'values': ['5']}, 'registers': []}],
    'pending': {'storage': 'local', 'scope': ['0'], 'pc': '0', 'decoded': {'arity': '2',
        'operation': {'primitive': {'scope': ['0'], 'pc': '0', 'operation': 'add'}}},
        'arguments': ['5'], 'part': 'arguments'}}


def validate(data, context='active'):
    return snapshot(data, SOURCE, policy='phased', original_limit=3, context=context)


class SnapshotTests(unittest.TestCase):
    def test_original_observations_preserved(self):
        original = copy.deepcopy(BASE)
        self.assertIs(validate(original), original)
        self.assertEqual(original, BASE)
        retained = copy.deepcopy(BASE)
        retained['poisoned'] = False; retained['pending']['storage'] = 'machine'
        self.assertIs(validate(retained, 'idle'), retained)
        unwound = copy.deepcopy(BASE)
        # A moved local Pending is dropped by unwinding, not recovered from a
        # fabricated callback result or silently restored into the machine.
        unwound['pending'] = None
        self.assertIs(validate(unwound, 'unwound'), unwound)
        failed = copy.deepcopy(BASE)
        failed['pending']['part'] = 'primitive'
        failed['pending']['arguments'] = ['5', '2']
        failed['terminal'] = {'stop': 'undefined', 'haltPath': ['0', '0'], 'reason': {'kind': 'primitive',
            'error': {'kind': 'resource', 'declaredClassification': 'resource'}}}
        self.assertIs(validate(failed), failed)

    def test_corrupted_snapshot_rejected(self):
        controls = [
            (['policy'], 'atomic', 'Policy changed'),
            (['originalLimit', 'max'], '4', 'Original allowance changed'),
            (['poisoned'], False, 'Poison guard'),
            (['poisoned'], 1, 'Poison guard'),
            (['counters', 'calls'], '0', 'Counter partition'),
            (['counters', 'dispatch'], str(MAX64 + 1), 'out of range'),
            (['rootInputs'], ['5'], 'pending root inputs'),
            (['frames', 0, 'scope'], ['0'], 'Root scope'),
            (['frames', 0, 'inputs', 'kind'], 'call', 'ownership kind'),
            (['frames', 1, 'pc'], '1', 'PC differs'),
            (['frames', 1, 'scope'], ['1'], 'no source owner'),
            (['frames', 1, 'inputs', 'values'], ['6'], 'Callee inputs'),
            (['pending', 'storage'], 'invented', 'Unknown pending storage'),
            (['pending', 'pc'], '1', 'Pending PC/scope'),
            (['pending', 'scope'], [], 'Pending PC/scope'),
            (['pending', 'decoded', 'arity'], '1', 'arity changed'),
            (['pending', 'decoded', 'operation', 'primitive', 'operation'], 'identity', 'primitive identity'),
            (['pending', 'decoded', 'operation', 'primitive', 'scope'], [], 'primitive identity'),
            (['pending', 'arguments'], ['6'], 'argument prefix changed'),
            (['pending', 'arguments'], ['5', '2', '9'], 'prefix exceeds'),
            (['pending', 'part'], 'primitive', 'before all arguments'),
            (['pending', 'part'], 'start', 'Unstarted instruction'),
            (['terminal'], {'stop': 'budget', 'reason': {'kind': 'allowance'}, 'haltPath': ['0', '0']}, 'pause, not a machine terminal'),
            (['terminal'], {'stop': 'complete', 'reason': None, 'haltPath': None}, 'retains pending instruction'),
            (['terminal'], {'stop': 'undefined', 'reason': None, 'haltPath': ['0', '0']}, 'Invalid terminal reason'),
            (['terminal'], {'stop': 'undefined', 'reason': {'kind': 'missing_callee_output'}, 'haltPath': ['0']}, 'halt path differs'),
        ]
        for path, replacement, error in controls:
            with self.subTest(path=path):
                changed = copy.deepcopy(BASE); target = changed
                for key in path[:-1]: target = target[key]
                target[path[-1]] = replacement
                with self.assertRaisesRegex(ValueError, error): validate(changed)

    def test_context_and_schema_cannot_be_guessed(self):
        with self.assertRaisesRegex(ValueError, 'Unknown service context'): validate(BASE, 'guess')
        with self.assertRaisesRegex(ValueError, 'Local pending escaped'): validate(BASE, 'unwound')
        for path in ([], ['pending'], ['frames', 0], ['counters'], ['pending', 'decoded']):
            value = copy.deepcopy(BASE); target = value
            for key in path: target = target[key]
            target['extra'] = None
            with self.assertRaisesRegex(ValueError, 'Expected fields'): validate(value)
        for value in (0, True, '-1', '+1', '01', '1.0', '', str(MAX64 + 1)):
            with self.subTest(value=value), self.assertRaises(ValueError): nat(value)
        self.assertEqual(nat(str(MAX64)), MAX64)
        value = copy.deepcopy(BASE)
        value['pending']['arguments'] = ['5', '2']; value['pending']['part'] = 'enter_call'
        with self.assertRaisesRegex(ValueError, 'Phase disagrees'): validate(value)
        value['pending']['part'] = 'primitive'
        self.assertIs(validate(value), value)


if __name__ == '__main__':
    unittest.main()
