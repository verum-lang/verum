"""Finite binary KT bounds, exact counterexamples, and semantic refusals."""
from pathlib import Path
import re
import tempfile
import unittest
from check_typed_programs import ROOT, check, require_success, run

HEADER = '''import VerumExternalReplay.BinaryPartitionKT
open ExactMass BinaryPartitionKT
set_option warningAsError true
'''
FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'


class BinaryPartitionKTTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.BinaryPartitionKT'))

    def prove(self, source, expected=None):
        with tempfile.TemporaryDirectory(prefix='verum-binary-kt-') as directory:
            path = Path(directory) / 'check.lean'
            path.write_text(HEADER + source)
            return check(path, expected)

    def test_exact_counterexample_counts_both_outcomes_and_name_cost(self):
        self.prove('''
example : Equal (kt 4 4) ⟨35,32768,by decide⟩ := by decide_cbv
example : Equal (partition [(4,0),(0,4)]) ⟨1225,16384,by decide⟩ := by decide_cbv
example : Equal (ratio [(4,0),(0,4)]) ⟨70,1,by decide⟩ := by decide_cbv
example : Equal (binomial 4 4) ⟨70,1,by decide⟩ := by decide_cbv
example : Less ⟨20,1,by decide⟩ (ratio [(4,0),(0,4)]) := by decide_cbv
-- Four positives do not bound the log2 gain by four bits.
example : Less ⟨2^4,1,by decide⟩ (ratio [(4,0),(0,4)]) := by decide_cbv
-- Same number of positives and rows, different conditional information.
example : Equal (ratio [(2,2),(2,2)]) ⟨18,35,by decide⟩ := by decide_cbv
example : Less (ratio [(2,2),(2,2)]) one := by decide_cbv
-- Charging eight names multiplies the price factor by eight.
example : Less (ratio [(4,0),(0,4)]) ⟨8*20,1,by decide⟩ := by decide_cbv
example : Equal (ratio []) one := by decide_cbv
example : Equal (ratio [(0,0),(4,4),(0,0)]) one := by decide_cbv
''')

    def test_universal_laws_have_only_standard_foundations(self):
        source = (ROOT / 'VerumExternalReplay/BinaryPartitionKT.lean').read_text()
        names = re.findall(r'^theorem (\w+)', source, re.M)
        result = self.prove('\n'.join('#print axioms ' + name for name in names) +
                            '\n#print axioms ExactMass.divide_atMost_iff')
        rows = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
        self.assertEqual(len(rows), len(names) + 1)
        for row in rows:
            self.assertTrue(row.endswith(('does not depend on any axioms',
                                          'depends on axioms: [propext]',
                                          'depends on axioms: [propext, Quot.sound]')), row)

    def test_invalid_information_and_price_claims_refuse_semantically(self):
        for claim in [
            'AtMost (ratio [(4,0),(0,4)]) ⟨2^4,1,by decide⟩',
            'AtMost (ratio [(4,0),(0,4)]) ⟨20,1,by decide⟩',
            'Less ⟨8*20,1,by decide⟩ (ratio [(4,0),(0,4)])',
            'Less ⟨20,1,by decide⟩ (ratio [(2,2),(2,2)])',
            'Equal (ratio [(4,0),(0,4)]) (ratio [(4,0)])',
            'Equal (ratio [(4,0),(0,4)]) (ratio [(2,2),(2,2)])',
        ]:
            with self.subTest(claim=claim):
                result = self.prove('example : ' + claim + ' := by decide_cbv', FALSE)
                errors = [d for d in result['diagnostics'] if d['severity'] == 'error']
                self.assertEqual(len(errors), 1)
                self.assertIn('proposition evaluates to `false`', errors[0]['data'])


if __name__ == '__main__':
    unittest.main()
