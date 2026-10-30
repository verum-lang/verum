"""Check original finite-product naming, not native lifecycle/VM correspondence."""
from pathlib import Path
import re
import tempfile
import unittest
from check_typed_programs import ROOT, check, require_success, run

HEADER = '''import VerumExternalReplay.RectangularNames
open RectangularNames
set_option warningAsError true
def sizes : Triple := ⟨2, 3, 5⟩
'''
FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'


class RectangularNamesTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.RectangularNames'))

    def prove(self, source, expected=None):
        with tempfile.TemporaryDirectory(prefix='verum-rectangular-names-') as directory:
            path = Path(directory) / 'check.lean'
            path.write_text(HEADER + source)
            return check(path, expected)

    def test_binary_decoder_and_original_names(self):
        self.prove('''
example (n p : Triple) (valid : Valid n p) : decode n (rank n p) = some p :=
  decode_rank n p valid
example (n : Triple) (index : Nat) (bound : index < n.i * n.u * n.r) :
  ∃ p, decode n index = some p ∧ Valid n p ∧ rank n p = index :=
  decode_in_range n index bound
example (n p t : Triple) (hp : Valid n p) (ht : Valid n t)
    (same : rank n p = rank n t) : p = t := rank_injective n p t hp ht same
example : (List.range 8).map (decode sizes) =
  [some ⟨0,0,0⟩, some ⟨0,0,1⟩, some ⟨0,1,0⟩, some ⟨0,1,1⟩,
   some ⟨1,0,0⟩, some ⟨1,0,1⟩, some ⟨1,1,0⟩, some ⟨1,1,1⟩] := by decide_cbv
example : ([⟨2,3,5⟩, ⟨5,2,3⟩, ⟨3,5,2⟩, ⟨1,1,1⟩, ⟨0,2,3⟩,
    ⟨2,0,3⟩, ⟨2,3,0⟩] : List Triple).all (fun n =>
      (List.range (n.i * n.u * n.r)).all (fun index =>
        match decode n index with
        | none => false
        | some p => decide (Valid n p) && (rank n p == index))) = true := by decide_cbv
example : rank sizes ⟨1,2,4⟩ = 29 ∧ decode sizes 30 = none := by decide_cbv
example : ([⟨0,0,0⟩, ⟨0,0,1⟩, ⟨0,1,0⟩, ⟨1,0,0⟩] : List Triple).map
    (rank sizes) = [0,1,2,4] := by decide_cbv
example : rank ⟨438,6040519,6040519⟩ ⟨0,0,17755⟩ = 138075130950 := by decide_cbv
example : decode ⟨438,6040519,6040519⟩ 138075130950 = some ⟨0,0,17755⟩ := by decide_cbv
example : rank ⟨2^90,2^91,2^92⟩ ⟨2^85,0,1⟩ < 2^90 * 2^91 * 2^92 :=
  rank_bound _ _ (by decide_cbv)
''')

    def test_wrong_ties_bounds_and_service_names_refuse(self):
        for claim in (
            'rank sizes ⟨1,0,0⟩ = 3',
            'rank sizes ⟨1,1,1⟩ = 3',
            'rank sizes ⟨1,2,4⟩ = 69',
            'decode sizes 4 = some ⟨0,1,1⟩',
            'decode sizes 2 = some ⟨1,0,0⟩',
            'decode sizes 29 = none',
            'decode sizes 30 = some ⟨1,2,4⟩',
            'decode ⟨0,3,5⟩ 0 = some ⟨0,0,0⟩',
            'decode ⟨2,0,5⟩ 0 = some ⟨0,0,0⟩',
            'decode ⟨2,3,0⟩ 0 = some ⟨0,0,0⟩',
        ):
            with self.subTest(claim=claim):
                result = self.prove('example : ' + claim + ' := by decide_cbv', FALSE)
                self.assertEqual(sum(d['severity'] == 'error' for d in result['diagnostics']), 1)

    def test_general_laws_have_checked_axioms(self):
        source = (ROOT / 'VerumExternalReplay/RectangularNames.lean').read_text()
        names = re.findall(r'^theorem (\w+)', source, re.M)
        self.assertEqual(len(names), 16)
        result = self.prove('\n'.join('#print axioms RectangularNames.' + name for name in names))
        rosters = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
        self.assertEqual(len(rosters), len(names))
        for row in rosters:
            self.assertTrue(row.endswith(('does not depend on any axioms',
                'depends on axioms: [propext]', 'depends on axioms: [propext, Quot.sound]')), row)


if __name__ == '__main__':
    unittest.main()
