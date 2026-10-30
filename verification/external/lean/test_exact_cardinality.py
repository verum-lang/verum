"""Finite DAG counts, completed nominal memoization and coefficient pruning."""
from pathlib import Path
import re
import tempfile
import unittest
from check_typed_programs import ROOT, check, require_success, run

HEADER = '''import VerumExternalReplay.ExactCardinality
open ExactCardinality
set_option warningAsError true
def declarations : Nat → Declaration Nat
  | 0 => ⟨1, 1, [⟨0, []⟩]⟩
  | 1 => ⟨1, 1, [⟨0, []⟩, ⟨2, [0, 0]⟩]⟩
  | 2 => ⟨1, 1, [⟨0, []⟩, ⟨2, [1, 1]⟩]⟩
  | 3 => ⟨0, 1, [⟨0, []⟩, ⟨2, [1, 1]⟩]⟩
  | 4 => ⟨0, 2, [⟨0, []⟩, ⟨2, [1, 1]⟩]⟩
  | 5 => ⟨1, 0, [⟨1, [0]⟩]⟩
  | 6 => ⟨1, 1, [⟨0, []⟩, ⟨1, [5]⟩]⟩
  | _ => ⟨0, 0, []⟩
def counts := exactCounts declarations id
'''
FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'


class ExactCardinalityTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.ExactCardinality'))

    def prove(self, source, expected=None):
        with tempfile.TemporaryDirectory(prefix='verum-exact-cardinality-') as directory:
            path = Path(directory) / 'check.lean'
            path.write_text(HEADER + source)
            return check(path, expected)

    def test_counts_and_completed_history(self):
        self.prove('''
example : [counts 0, counts 1, counts 2, counts 3, counts 4, counts 5, counts 6] =
    [1, 2, 5, 1, 11, 0, 1] := by decide_cbv
example : factor counts [1, 1] = 4 := by decide_cbv
example : History declarations [(0,1), (1,2), (2,5)] :=
  .publish (.publish (.publish .empty (.complete (by decide_cbv) (by decide_cbv) (by decide_cbv)))
    (.complete (by decide_cbv) (by decide_cbv) (by decide_cbv)))
    (.complete (by decide_cbv) (by decide_cbv) (by decide_cbv))
example : resolve declarations [(1,2)] [1,1] = some 4 := by decide_cbv
example : compute declarations [] (declarations 3) = some 1 := by decide_cbv
example : compute declarations [] (declarations 4) = none := by decide_cbv
example : compute declarations [(1,2)] (declarations 4) = some 11 := by decide_cbv
example : read declarations [] 5 = some 0 := by decide_cbv
example : lookup ([] : Cache Nat) 5 = none := rfl
example : compute declarations [] (declarations 6) = some 1 := by decide_cbv
example : compute declarations [] (declarations 5) = some 0 := by decide_cbv
example : unreachable 0 1 0 = false ∧ instructions 0 [⟨0,1⟩] = 1 := by decide_cbv
example : total 0 [⟨0,1⟩,⟨1,4⟩] 2 = 11 := by decide_cbv
example : total 0 [⟨0,1⟩,⟨1,0⟩] 2 = 3 := by decide_cbv
example : total 1 [⟨0,1⟩,⟨2,4⟩] 2 = 175 := by decide_cbv
example : lookup [(10,2)] 11 = none := by decide_cbv
example : resolve declarations [(10,2)] [1] = none := by decide_cbv
''')

    def test_wrong_counts_and_unsound_reuse_rejected(self):
        for claim in (
            'factor counts [1,1] = 2',
            'factor counts [] = 0',
            'counts 2 = 3',
            'counts 4 = 3',
            'counts 6 = 0',
            'unreachable 0 2 2 = true',
            'unreachable 0 1 0 = true',
            'instructions 0 [⟨0,1⟩] = 0',
            'total 1 [⟨0,1⟩] 2 = 2',
            'compute declarations [] (declarations 4) = some 0',
            'resolve declarations [(1,2)] [1,1] = some 2',
            'lookup [(10,2)] 11 = some 2',
            'read declarations [] 5 = none',
            'compute declarations [] (declarations 6) = none',
            'counts 1 = 1',
            'total 1 [⟨0,1⟩,⟨2,4⟩] 2 = 55',
        ):
            with self.subTest(claim=claim):
                result = self.prove('example : ' + claim + ' := by decide_cbv', FALSE)
                self.assertEqual(sum(d['severity'] == 'error' for d in result['diagnostics']), 1)

    def test_invalid_cache_publication_rejected(self):
        for source in (
            'example : Publication declarations [] [(5,0)] := '
            '.complete (by decide_cbv) (by decide_cbv) (by decide_cbv)',
            'example : Publication declarations [(0,1)] [(0,1),(0,1)] := '
            '.complete (by decide_cbv) (by decide_cbv) (by decide_cbv)',
            'example : Publication declarations [] [(1,2)] := '
            '.complete (by decide_cbv) (by decide_cbv) (by decide_cbv)',
            'example : Publication declarations [(0,1)] [(0,1),(1,1)] := '
            '.complete (by decide_cbv) (by decide_cbv) (by decide_cbv)',
        ):
            with self.subTest(source=source):
                result = self.prove(source, FALSE)
                self.assertEqual(sum(d['severity'] == 'error' for d in result['diagnostics']), 1)

    def test_general_laws_have_checked_axioms(self):
        source = (ROOT / 'VerumExternalReplay/ExactCardinality.lean').read_text()
        names = re.findall(r'^theorem (\w+)', source, re.M)
        self.assertEqual(len(names), 25)
        result = self.prove('\n'.join('#print axioms ExactCardinality.' + name for name in names))
        rosters = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
        self.assertEqual(len(rosters), len(names))
        for row in rosters:
            self.assertTrue(row.endswith(('does not depend on any axioms',
                'depends on axioms: [propext]', 'depends on axioms: [propext, Quot.sound]')), row)


if __name__ == '__main__':
    unittest.main()
