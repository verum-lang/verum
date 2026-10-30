"""Fresh kernel checks of the resource laws and exact 64-bit native witness."""
import copy
import hashlib
import re
import unittest

from check_resource_ledger_export import emit
from lean_test_fixture import IsolatedLeanTest
from observer_export_replay import load

MODULES = ('ResourceLedger', 'ResourceLedgerControls', 'ResourceLedgerAxioms')


class ResourceLedgerProofTests(IsolatedLeanTest):
    MODULES = MODULES
    FIXTURES = ('fixtures/resource-ledger-v1.json',
                'fixtures/resource-ledger-v1.provenance.json')

    @classmethod
    def setUpClass(cls):
        super().setUpClass()
        cls.data = load(cls.snapshot / cls.FIXTURES[0])

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
