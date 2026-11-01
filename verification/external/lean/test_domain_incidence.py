"""Frozen incidence replay; corrupted semantics must fail in the Lean kernel."""
import gzip
import hashlib
import json
from pathlib import Path
import re
import shutil
import tempfile
import unittest
from check_domain_incidence_export import emit, MODELS
from check_typed_programs import ROOT, check, require_success, run

FALSE = 'Tactic `decide` proved that the proposition'


def fixture():
    raw = gzip.decompress((ROOT/'fixtures/domain-incidence-native.json.gz').read_bytes())
    provenance = json.loads((ROOT/'fixtures/domain-incidence-native.provenance.json').read_text())
    if hashlib.sha256(raw).hexdigest() != provenance['export_sha256']:
        raise ValueError('Native fixture identity changed')
    return json.loads(raw)


class DomainIncidence(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory(prefix='verum-domain-incidence-')
        cls.root = Path(cls.temporary.name)
        for file in ['lakefile.toml', 'lean-toolchain'] + [f'VerumExternalReplay/{m}.lean' for m in MODELS]:
            target = cls.root/file; target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT/file, target)
        for module in MODELS:
            target = cls.root/f'.lake/build/lib/lean/VerumExternalReplay/{module}.olean'
            target.parent.mkdir(parents=True, exist_ok=True)
            require_success(run('lake', 'env', 'lean', '-M', '1024', '-o', str(target),
                f'VerumExternalReplay/{module}.lean', cwd=cls.root, timeout=60))

    @classmethod
    def tearDownClass(cls): cls.temporary.cleanup()

    def replay(self, data, expected=None):
        text, counts = emit(data)
        path = self.root/'claims.lean'; path.write_text(text)
        return check(path, expected, cwd=self.root, memory_mib=1024), counts

    def test_frozen_plan_masks_refusals_and_child_lifetimes(self):
        result, counts = self.replay(fixture())
        self.assertTrue(result['accepted'])
        self.assertEqual(counts['events'], 149)

    def test_general_partition_laws_and_model_boundary_cases(self):
        source = (self.root/'VerumExternalReplay/DomainIncidence.lean').read_text()
        names = re.findall(r'^theorem (\w+)', source, re.M)
        self.assertEqual(names, ['insert_partition', 'build_partition', 'exact_multiplicity',
                                'canonical_partition', 'zero_work'])
        path = self.root/'audit.lean'
        # These are model tests, not fabricated native observations. The v2
        # producer does not export heterogeneous arities or slab contents.
        path.write_text('import VerumExternalReplay.DomainIncidence\nopen DomainIncidence\n' +
            '\n'.join(f'#print axioms DomainIncidence.{n}' for n in names) + '''
example : original [[], [1, 2], [1]] = [(1, (1, 0)), (2, (1, 1)), (1, (2, 0))] := by decide
example : entries (build [(1, (0, 0)), (2, (1, 0)), (1, (2, 0))] []) =
    [(1, (0, 0)), (1, (2, 0)), (2, (1, 0))] := by decide
example : (entries (build [(1, (0, 0)), (1, (0, 0))] [])).count (1, (0, 0)) = 2 := by decide
example : room 18446744073709551615 1 18446744073709551615 = false := by decide
example : room 18446744073709551614 1 18446744073709551615 = true := by decide
def plan : Plan := { phase := .done, alternative := 1, groups := [⟨1, [(0, 0), (0, 1)]⟩], cells := 3 }
def child (arrival : Nat) : Child := {
  language := "language", identity := "child", arrival := arrival
  outputs := [1], cursor := 1, compatible := 1, position := 1, phase := .positions }
example : PlanSound [[1, 1]] plan := by decide
example : ChildSound plan.groups (child 0) ∧ ChildSound plan.groups (child 1) := by decide
example : (step [[1, 1]] ⟨plan, some (child 0)⟩ 10 1 10).publication = none := by decide
example : (step [[1, 1]] ⟨plan, some (child 1)⟩ 10 1 11).publication = some ⟨0, 1, 1, 1⟩ := by decide
example : (step [[1, 1]] ⟨plan, some (child 1)⟩ 10 1 10).outcome = .capacityRefusal := by decide
example : (step [[1, 1]] ⟨plan, some (child 1)⟩ 10 1 9).state = ⟨plan, some (child 1)⟩ := by decide
def emptyChild : Child := { language := "language", identity := "child", arrival := 0, outputs := [2] }
example : ChildSound plan.groups emptyChild := by decide
example : (step [[1, 1]] ⟨plan, some emptyChild⟩ 10 1 10).state.child =
    some (nextDomain { emptyChild with cursor := 1 }) := by decide
example : (step [[1, 1]] ⟨plan, some emptyChild⟩ 10 1 10).publication = none := by decide
''')
        result = check(path, cwd=self.root, memory_mib=1024)
        self.assertEqual(len(result['diagnostics']), len(names))
        for item in result['diagnostics']:
            text = item['data']
            self.assertTrue(text.endswith('does not depend on any axioms') or
                            text.endswith('depends on axioms: [propext]') or
                            text.endswith('depends on axioms: [propext, Quot.sound]'), text)

    def test_semantic_mutations_reach_kernel_rejection(self):
        mutations = ('position', 'domain', 'duplicate', 'missing_position', 'plan_cells',
            'scan', 'empty_skip', 'position_advance', 'work_state', 'capacity_charge',
            'capacity_room', 'release', 'resume', 'owner', 'cost', 'control_mask', 'foreign_owner')
        for mutation in mutations:
            data = fixture(); stream = data['streams'][0]; rows = stream['events']
            def event(kind, result='advanced'):
                return next(r for r in rows if r['kind'] == kind and r['result'] == result)
            if mutation == 'position': event('plan_publish')['after']['groups'][0][1][0][0] = 1
            elif mutation == 'domain': event('plan_publish')['after']['groups'][0][0] = 2
            elif mutation == 'duplicate': event('plan_publish')['after']['groups'][0][1].append([0, 0])
            elif mutation == 'missing_position':
                next(r for r in rows if r['kind'] == 'plan_publish' and r['result'] == 'advanced'
                     and r['after']['alternative'] == 3)['after']['groups'][0][1].pop()
            elif mutation == 'plan_cells': event('plan_publish')['after']['plan_cells'] += 1
            elif mutation == 'scan': event('scan_domain')['after']['child']['mask'] ^= 1
            elif mutation == 'empty_skip':
                row = next(r for r in rows if r['kind'] == 'scan_domain' and r['result'] == 'advanced'
                    and r['after']['child']['domain_index'] > r['before']['child']['domain_index'])
                row['after']['child'].update(domain_index=row['before']['child']['domain_index'],
                    domain=row['before']['child']['domain'])
            elif mutation == 'position_advance': event('incidence_position')['after']['child']['position'] = 0
            elif mutation == 'work_state': event('plan_position', 'work_refusal')['after']['requested_domain'] = 1
            elif mutation == 'capacity_charge': event('plan_publish', 'capacity_refusal')['charged'] = 0
            elif mutation == 'capacity_room':
                row = event('incidence_position'); row['capacity'] = row['cells_before']
            elif mutation == 'release': event('release_child')['cells_after'] += 1
            elif mutation == 'resume': event('plan_publish')['resumed'] = False
            elif mutation == 'owner': event('scan_domain')['after']['child']['language_owner'] = data['streams'][1]['language_owner']
            elif mutation == 'cost': stream['incidence_charged'] += 1
            elif mutation == 'control_mask': data['controls'][1]['domain1_mask'] ^= 1
            elif mutation == 'foreign_owner': data['controls'][0]['attempted'] = data['controls'][0]['source']
            with self.subTest(mutation=mutation): self.replay(data, FALSE)

    def test_malformed_or_truncated_exports_are_refused(self):
        mutations = [lambda d:d.update(extra=True),
            lambda d:d['streams'][0]['events'].pop(),
            lambda d:d['streams'][0]['events'][0]['before'].update(anchor=1),
            lambda d:d['streams'][0]['events'][0]['before'].update(plan_phase=9),
            lambda d:d['streams'][0]['events'][0].update(grant=True),
            lambda d:d['streams'][0]['events'][0].update(cells_before=MAX_NATIVE + 1),
            lambda d:d['streams'][1].update(language_owner=d['streams'][0]['language_owner'])]
        for i, mutate in enumerate(mutations):
            data = fixture(); mutate(data)
            with self.subTest(i=i), self.assertRaises(ValueError): emit(data)


MAX_NATIVE = 2**64 - 1
if __name__ == '__main__': unittest.main()
