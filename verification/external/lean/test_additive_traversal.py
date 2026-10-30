"""Additive scheduling laws, dyadic boundaries and discriminating false controls."""
from pathlib import Path
import re
import tempfile
import unittest
from check_typed_programs import ROOT, check, require_success, run

HEADER = '''import VerumExternalReplay.AdditiveTraversal
open AdditiveTraversal
set_option warningAsError true
def x := finite [9, 1] (by decide) (fun _ => 0)
def y := finite [2] (by decide) (fun _ => 0)
def z := finite [0, 3, 7] (by decide) (fun _ => 0)
'''
FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'


class AdditiveTraversalTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.AdditiveTraversal'))

    def prove(self, source, expected=None):
        with tempfile.TemporaryDirectory(prefix='verum-additive-traversal-') as directory:
            path = Path(directory) / 'check.lean'; path.write_text(HEADER + source)
            return check(path, expected)

    def test_finite_and_locally_finite_enumeration(self):
        self.prove('''
example : layer x y z 0 = product [9, 1] [2] [0, 3, 7] := by decide_cbv
example : (through x y z 0).length = 6 := by decide_cbv
example : (through x y z 3).Nodup := through_nodup x y z 3
example : dyadicLayer 0 = [0] ∧ dyadicLayer 1 = [1] ∧
    dyadicLayer 2 = [2, 3] ∧ dyadicLayer 3 = [4, 5, 6, 7] := by decide_cbv
example : (through (boundedOrdinals 2) (boundedOrdinals 3) (boundedOrdinals 5) 6).length = 30 := by decide_cbv
example (n : Nat) : through (boundedOrdinals 0) (boundedOrdinals 3) (boundedOrdinals 5) n = [] := by
  apply List.eq_nil_iff_forall_not_mem.mpr
  intro ⟨a, b, c⟩ member
  have bound := (bounded_membership 0 3 5 a b c n).mp member
  omega
example (a b c : Nat) : (a, b, c) ∈ through dyadic dyadic dyadic
    (dyadicCost a + dyadicCost b + dyadicCost c) :=
  eventually_enumerated dyadic dyadic dyadic a b c trivial trivial trivial
-- Large supports are handled symbolically; no enumeration or raised proof limit.
example : (0, 0, 17755) ∈ through (boundedOrdinals 438)
    (boundedOrdinals 6040519) (boundedOrdinals 6040519) 15 :=
  (bounded_membership 438 6040519 6040519 0 0 17755 15).mpr (by decide_cbv)
-- Lexicographically exhausting one infinite component can starve another forever.
example (n : Nat) : (0, 1, 0) ∉ product [0] [0] (List.range n) := by simp [mem_product]
''')

    def test_incorrect_partitions_and_schedules_refuse(self):
        for claim in (
            'dyadicLayer 0 = [0, 1]',
            'dyadicLayer 3 = [4, 5, 6, 7, 8]',
            'dyadicCost 17755 = 14',
            '(0, 0, 8) ∈ through dyadic dyadic dyadic 3',
            '(0, 0, 0) ∈ through (boundedOrdinals 0) dyadic dyadic 2',
            '(through x y z 0).length = 8',
            '(product [0, 0] [0] [0]).Nodup',
            '(9, 2, 7) ∈ layer x y z 1',
        ):
            with self.subTest(claim=claim):
                result = self.prove('example : ' + claim + ' := by decide_cbv', FALSE)
                self.assertEqual(sum(d['severity'] == 'error' for d in result['diagnostics']), 1)

    def test_general_laws_have_checked_axioms(self):
        source = (ROOT / 'VerumExternalReplay/AdditiveTraversal.lean').read_text()
        names = re.findall(r'^theorem (\w+)', source, re.M)
        self.assertEqual(len(names), 14)
        result = self.prove('\n'.join('#print axioms AdditiveTraversal.' + name for name in names))
        rosters = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
        self.assertEqual(len(rosters), len(names))
        for row in rosters:
            self.assertTrue(row.endswith(('does not depend on any axioms',
                'depends on axioms: [propext]', 'depends on axioms: [propext, Quot.sound]')), row)


if __name__ == '__main__':
    unittest.main()
