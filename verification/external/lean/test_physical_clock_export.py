"""Replay actual frozen native clock bytes and reject altered ownership reports."""
import copy
import hashlib
from pathlib import Path
import re
import tarfile
import unittest
from check_physical_clock_export import emit
from lean_test_fixture import IsolatedLeanTest
from observer_export_replay import load, require


class NativePhysicalClockTests(IsolatedLeanTest):
    MODULES = ('PhysicalClock',)
    FIXTURES = ('fixtures/physical-clock-native-v1.json',
                'fixtures/physical-clock-native-v1.tar.gz')

    @classmethod
    def setUpClass(cls):
        super().setUpClass()
        manifest = load(cls.snapshot / cls.FIXTURES[0])
        archive = cls.snapshot / cls.FIXTURES[1]
        require(hashlib.sha256(archive.read_bytes()).hexdigest() == manifest['archive_sha256'],
                'Frozen native archive changed')
        destination = cls.snapshot / 'native'
        destination.mkdir()
        with tarfile.open(archive, 'r:gz') as stream:
            members = stream.getmembers()
            require(len(members) == len(manifest['members']) and
                    {m.name for m in members} == set(manifest['members']), 'Archive roster mismatch')
            for member in members:
                require(member.isfile() and Path(member.name).name == member.name and
                        member.size <= 16 * 1024 * 1024, 'Invalid native archive member')
                content = stream.extractfile(member).read()
                require(hashlib.sha256(content).hexdigest() == manifest['members'][member.name],
                        'Frozen native member changed')
                (destination / member.name).write_bytes(content)
        provenance = load(destination / 'provenance.json')
        stages = {stage['name']: stage for stage in provenance['stages']}
        for name in ('native-physical-time', 'native-execution'):
            require(name in stages and stages[name]['status'] == 'passed' and
                    stages[name]['returncode'] == 0 and stages[name]['counts']['passed'] > 0 and
                    stages[name]['counts']['failed'] == 0, 'Native clock stage did not pass')
        cls.cases = {}
        for name, expected in provenance['exports'].items():
            require(manifest['members'].get(name) == expected, 'Native provenance mismatch')
            cls.cases[name.removesuffix('.json')] = load(destination / name)
        require(len(cls.cases) == 16, 'Incomplete native court roster')

    def test_frozen_native_handoffs_and_every_claim_axiom(self):
        total = {'claims': 0, 'native_steps': 0}
        for name, data in self.cases.items():
            with self.subTest(case=name):
                source, counts = emit(data)
                names = ['FrozenObserverExport.' + n for n in re.findall(r'^theorem (\w+)', source, re.M)]
                source += '\n' + '\n'.join('#print axioms ' + n for n in names)
                self.assert_axioms(self.prove(source), names)
                for key in total:
                    total[key] += counts[key]
        self.assertEqual(total, {'claims': 136, 'native_steps': 65})

    def test_corrupted_native_observations_fail_in_the_kernel(self):
        changes = [
            ('organism-unknown-untracked', 0, ('after', 'plan'), []),
            ('organism-unknown-tracked', 0, ('after', 'failure'), None),
            ('organism-uncommitted-untracked', -1, ('after', 'calls'), 2),
            ('step', 1, ('result', 'status'), 'Err(PendingTerminal)'),
            ('step', 2, ('result', 'retained', 'point', 'origin'), 1),
            ('completed', -1, ('after', 'episode'), 3),
            ('episode-limit', -1, ('result',), 'AgeOverflow'),
            ('age-limit', 0, ('result', 'calls'), 1),
        ]
        for name, index, path, value in changes:
            with self.subTest(case=name, path=path):
                data = copy.deepcopy(self.cases[name])
                target = data['steps'][index]
                for key in path[:-1]:
                    target = target[key]
                target[path[-1]] = value
                source, _ = emit(data)
                self.prove(source, failure=True, expected_error='Tactic `rfl` failed')

    def test_restart_origin_must_be_fresh_even_when_equal_to_its_predecessor(self):
        data = copy.deepcopy(self.cases['step'])
        data['steps'][-1]['after']['origin'] = data['steps'][-1]['before']['origin']
        data['origin_count'] -= 1
        with self.assertRaisesRegex(ValueError, 'Successful restart reused an origin'):
            emit(data)

    def test_consumed_receipt_cannot_be_reintroduced(self):
        data = copy.deepcopy(self.cases['step'])
        duplicate = copy.deepcopy(data['steps'][3])
        duplicate['before'] = copy.deepcopy(duplicate['after'])
        data['steps'].insert(4, duplicate)
        with self.assertRaisesRegex(ValueError, 'Receipt is absent or already consumed'):
            emit(data)

    def test_native_state_continuity_is_required(self):
        data = copy.deepcopy(self.cases['step'])
        data['steps'][3]['before']['age'] = 2
        with self.assertRaisesRegex(ValueError, 'Disconnected before-state'):
            emit(data)

    def test_exact_bounds_and_complete_names_are_required(self):
        for field, value, error in [('maximum', 255, 'exact u64'),
                                    ('origin_count', 4, 'Incomplete origin'),
                                    ('sample_count', 2, 'Incomplete sample')]:
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, error):
                emit({**self.cases['step'], field: value})


if __name__ == '__main__':
    unittest.main()
