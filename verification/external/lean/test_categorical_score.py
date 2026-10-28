"""Exact scorer controls, including support, strict boundaries and full readings."""
from pathlib import Path
import tempfile
import unittest
from check_typed_programs import check, require_success, run

HEADER='import VerumExternalReplay.CategoricalScore\nopen CategoricalScore\nset_option warningAsError true\n'

class CategoricalScoreTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake','build','VerumExternalReplay.CategoricalScore'))

    def test_boundaries_and_support(self):
        source=HEADER+'''
theorem empty_score : agrees [] ⟨one, one, 0⟩ = true := by decide
theorem declared_support : agrees [⟨2, [⟨0,0⟩]⟩] ⟨one, ⟨1,2⟩, 1⟩ = true := by decide
theorem invalid_label : score [⟨2, [⟨0,2⟩]⟩] = none := by decide
theorem invalid_alphabet : score [⟨0, []⟩] = none := by decide
theorem empty_context : agrees [⟨2, []⟩] ⟨one, one, 0⟩ = true := by decide
theorem independent_children : agrees [⟨2, [⟨0,0⟩,⟨1,1⟩]⟩] ⟨⟨2,1⟩, one, 2⟩ = true := by decide
theorem same_reading : agrees [⟨2, [⟨0,0⟩,⟨0,1⟩]⟩] ⟨one, ⟨1,2⟩, 2⟩ = true := by decide
theorem compression_tie : purchased ⟨⟨20,1⟩,⟨401,1⟩,1⟩ 20 20 = false := by decide
theorem predictive_tie : purchased ⟨⟨21,1⟩,⟨400,1⟩,1⟩ 20 20 = false := by decide
theorem purchase : purchased ⟨⟨21,1⟩,⟨401,1⟩,1⟩ 20 20 = true := by decide
theorem invalid_denominator : purchased ⟨⟨21,0⟩,⟨401,1⟩,1⟩ 20 20 = false := by decide
theorem no_joint_growth : confirmed ⟨⟨21,1⟩,⟨401,1⟩,1⟩ ⟨⟨21,1⟩,⟨402,1⟩,2⟩ 20 20 = false := by decide
'''
        names=[s.split()[1] for s in source.splitlines() if s.startswith('theorem ')]
        source+='\n'.join('#print axioms '+n for n in names)
        with tempfile.TemporaryDirectory() as d:
            p=Path(d)/'score.lean';p.write_text(source)
            rows=[r['data'] for r in check(p)['diagnostics'] if r['severity']=='information']
        self.assertEqual(len(rows),len(names))
        self.assertTrue(all(r.endswith('does not depend on any axioms') or
                            r.endswith('depends on axioms: [propext]') for r in rows),rows)

if __name__=='__main__':unittest.main()
