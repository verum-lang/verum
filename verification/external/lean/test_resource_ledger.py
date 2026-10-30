"""Fresh kernel checks of the resource laws and exact 64-bit native witness."""
import copy
import hashlib
import json
from pathlib import Path
import re
import tempfile
import unittest

from check_resource_ledger_export import emit
from check_typed_programs import ROOT, require_success, run
from observer_export_replay import load

FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'
MODULES = ('ResourceLedger', 'ResourceLedgerControls', 'ResourceLedgerAxioms')


class ResourceLedgerProofTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # No shared build products: every run checks the captured source afresh.
        cls.directory = tempfile.TemporaryDirectory(prefix='verum-resource-ledger-')
        cls.addClassCleanup(cls.directory.cleanup)
        cls.snapshot = Path(cls.directory.name)
        paths = ['lean-toolchain', 'lakefile.toml']
        paths += [f'VerumExternalReplay/{name}.lean' for name in MODULES]
        paths += ['fixtures/resource-ledger-v1.json',
                  'fixtures/resource-ledger-v1.provenance.json']
        for name in paths:
            destination = cls.snapshot / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes((ROOT / name).read_bytes())
        cls.sources = {name: (cls.snapshot / f'VerumExternalReplay/{name}.lean').read_text()
                       for name in MODULES}
        for name in MODULES:
            output = cls.snapshot / f'.lake/build/lib/lean/VerumExternalReplay/{name}.olean'
            output.parent.mkdir(parents=True, exist_ok=True)
            require_success(run('lake', 'env', 'lean', '-M', '1024',
                                '-o', str(output), f'VerumExternalReplay/{name}.lean',
                                timeout=60, cwd=cls.snapshot))
        cls.data = load(cls.snapshot / 'fixtures/resource-ledger-v1.json')

    def prove(self, source, *, failure=False):
        path = self.snapshot / 'Check.lean'
        path.write_text(source)
        result = run('lake', 'env', 'lean', '-M', '1024', '--json', str(path),
                     timeout=60, cwd=self.snapshot)
        self.assertEqual(result.stderr, '')
        diagnostics = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
        errors = [d for d in diagnostics if d['severity'] == 'error']
        if failure:
            self.assertEqual(result.returncode, 1, diagnostics)
            self.assertEqual(len(errors), 1, diagnostics)
            self.assertTrue(errors[0]['data'].startswith(FALSE), errors)
        else:
            self.assertEqual(result.returncode, 0, diagnostics)
            self.assertEqual(errors, [], diagnostics)
        self.assertFalse(any(d['severity'] == 'warning' for d in diagnostics), diagnostics)
        return [d['data'] for d in diagnostics if d['severity'] == 'information']

    def assert_axioms(self, rows, names):
        self.assertEqual(len(rows), len(names), rows)
        for name, row in zip(names, rows):
            self.assertIn(row, [f"'{name}' does not depend on any axioms",
                                f"'{name}' depends on axioms: [propext]",
                                f"'{name}' depends on axioms: [propext, Quot.sound]"])

    def test_all_model_and_control_proofs_have_audited_axioms(self):
        names = [f'{module}.{name}' for module in MODULES[:2]
                 for name in re.findall(r'^theorem (\w+)', self.sources[module], re.M)]
        guards = re.findall(r'^#print axioms (\w+\.\w+)',
                            self.sources['ResourceLedgerAxioms'], re.M)
        self.assertEqual(len(names), 70)
        self.assertCountEqual(guards, names)
        source = ('import VerumExternalReplay.ResourceLedgerAxioms\n'
                  'set_option warningAsError true\n' +
                  '\n'.join('#print axioms ' + name for name in names))
        self.assert_axioms(self.prove(source), names)

    def test_frozen_trace_and_exact_machine_bound(self):
        fixture = self.snapshot / 'fixtures/resource-ledger-v1.json'
        provenance = load(self.snapshot / 'fixtures/resource-ledger-v1.provenance.json')
        self.assertEqual(hashlib.sha256(fixture.read_bytes()).hexdigest(),
                         provenance['export_sha256'])
        source, counts = emit(self.data)
        self.assertEqual(counts, {'claims': 16, 'native_snapshots': 8})
        names = ['FrozenObserverExport.' + name
                 for name in re.findall(r'^theorem (\w+)', source, re.M)]
        # These are semantic equalities, not a search for the maximum's spelling.
        source += '''
open ResourceLedger FrozenObserverExport
theorem exactMachineBound : s0.maximum = 18446744073709551615 := rfl
theorem computedWordBound : total s0.slots + 16 ≤ 18446744073709551615 :=
  Nat.le_of_ble_eq_true rfl
'''
        names += ['exactMachineBound', 'computedWordBound']
        source += '\n'.join('#print axioms ' + name for name in names)
        rows = self.prove(source)
        self.assert_axioms(rows, names)
        self.assertEqual(rows[-1], "'computedWordBound' does not depend on any axioms")

    def test_each_corrupted_observable_is_a_false_proposition(self):
        changes = {'held': 33, 'peak': 47, 'limit': 49,
                   'classes': [0, 31, 0, 0, 0], 'reservations': 2}
        for field, value in changes.items():
            with self.subTest(field=field):
                altered = copy.deepcopy(self.data)
                altered['events'][4][field] = value
                source, _ = emit(altered)
                self.prove(source, failure=True)

    def test_false_model_controls_fail_outside_message_guards(self):
        controls = re.findall(r'^example : .+$', self.sources['ResourceLedgerControls'], re.M)
        self.assertEqual(len(controls), 5)
        header = ('import VerumExternalReplay.ResourceLedgerControls\n'
                  'open ResourceLedger ResourceLedgerControls\n'
                  'set_option warningAsError true\n')
        for control in controls:
            with self.subTest(claim=control):
                self.prove(header + control, failure=True)


if __name__ == '__main__':
    unittest.main()
