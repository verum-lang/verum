"""Universal transducer laws and semantic controls; no native runtime evidence."""
from pathlib import Path
import re
import tempfile
import unittest
from check_typed_programs import ROOT, check, require_success, run


class PhysicalTransducersTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.PhysicalTransducers'))

    def test_universal_foundations(self):
        names = re.findall(r'^theorem (\w+)',
            (ROOT / 'VerumExternalReplay/PhysicalTransducers.lean').read_text(), re.M)
        with tempfile.TemporaryDirectory(prefix='verum-physical-laws-') as directory:
            source = Path(directory) / 'audit.lean'
            source.write_text('import VerumExternalReplay.PhysicalTransducers\n' +
                '\n'.join('#print axioms PhysicalTransducers.' + n for n in names))
            result = check(source)
        observed = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
        self.assertEqual(len(observed), len(names))
        for row in observed:
            self.assertTrue(row.endswith(('does not depend on any axioms',
                'depends on axioms: [propext]')), row)

    def test_semantic_false_controls(self):
        statements = {
            'service_changes_state': 'step (X := Bool) (fun _ x => x) false (.service ()) = true',
            'physical_observation_erased': 'run (G := Unit) (fun _ x => x) [.observation true] false = false',
            'physical_order_ignored': 'fold (fun (_ : Bool) x => x) [false, true] false = fold (fun _ x => x) [true, false] false',
        }
        with tempfile.TemporaryDirectory(prefix='verum-physical-controls-') as directory:
            for name, statement in statements.items():
                with self.subTest(name=name):
                    source = Path(directory) / (name + '.lean')
                    source.write_text('import VerumExternalReplay.PhysicalTransducers\n'
                        'open PhysicalTransducers\nset_option warningAsError true\n'
                        f'example : {statement} := by decide\n')
                    result = check(source, 'Tactic `decide` proved that the proposition')
                    self.assertEqual(sum(d['severity'] == 'error' for d in result['diagnostics']), 1)


if __name__ == '__main__': unittest.main()
