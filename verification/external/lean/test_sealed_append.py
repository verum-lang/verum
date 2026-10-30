"""Exact nominal append boundaries, finite prefix induction and existing evaluator reuse."""
from pathlib import Path
import re
import tempfile
import unittest
from check_typed_programs import ROOT, check, require_success, run

HEADER = '''import VerumExternalReplay.SealedAppend
open SealedAppend
set_option warningAsError true
def owner (key : Nat) : Nat := if key = 14 then 8 else 7
def rows : Nat → List Nat
  | 10 => [3,4]
  | 11 => [3,4,5]
  | 12 => [3,4,5]
  | 13 => [3,9,5]
  | 14 => [3,4,5]
  | 15 => [3]
  | 16 => [3,4]
  | 17 => [3,4,5,6]
  | _ => []
def boundary : Boundary Nat Nat := ⟨7, some 10, 11, 2⟩
def witness : Verified owner rows := ⟨boundary, by decide_cbv⟩
def nextWitness : Verified owner rows := ⟨⟨7, some 11, 17, 3⟩, by decide_cbv⟩
def update (s e : Nat) : Except Nat Nat := if e = 4 then .error 44 else .ok (s+e)
'''
FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'


class SealedAppendTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.SealedAppend'))

    def prove(self, source, expected=None):
        with tempfile.TemporaryDirectory(prefix='verum-sealed-append-') as directory:
            path = Path(directory) / 'check.lean'
            path.write_text(HEADER + source)
            return check(path, expected)

    def test_nominal_admission_and_exact_execution(self):
        self.prove('''
example : admit (some witness) boundary = .reuse := by decide_cbv
example : admit (origin := owner) (events := rows) none boundary = .scan := rfl
example : verify owner rows ⟨7, some 10, 10, 2⟩ = true := by decide_cbv
example : verify owner rows ⟨7, none, 11, 0⟩ = true := by decide_cbv
example : rows 11 = rows 12 ∧ admit (some witness) {boundary with next := 12} = .refuse := by decide_cbv
example : rows 10 = rows 16 ∧ admit (some witness) {boundary with previous := some 16} = .refuse := by decide_cbv
example : Trace owner rows 10 17 :=
  .step (.step (.refl 10) witness rfl) nextWitness rfl
example : rows 10 <+: rows 17 :=
  trace_prefix (.step (.step (.refl 10) witness rfl) nextWitness rfl)
example : SharedUpdates.runFrom update 0 (rows 11) 0 =
    .refused 1 3 4 44 := by decide_cbv
example : SharedUpdates.runFrom update 0 (rows 11) 0 =
    SharedUpdates.continueFrom update 2 [5] (SharedUpdates.runFrom update 0 (rows 10) 0) :=
  continued_execution witness update 0 0
''')

    def test_invalid_checks_and_foreign_boundaries_refuse(self):
        for claim in (
            'verify owner rows {boundary with next := 13} = true',
            'verify owner rows {boundary with next := 14} = true',
            'verify owner rows {boundary with next := 15} = true',
            'verify owner rows {boundary with prefixLength := 1} = true',
            'admit (some witness) {boundary with previous := some 16} = .reuse',
            'admit (some witness) {boundary with next := 12} = .reuse',
            'admit (some witness) {boundary with source := 8} = .reuse',
            'admit (some witness) {boundary with previous := none} = .reuse',
            'admit (some witness) {boundary with previous := some 11} = .reuse',
            'admit (some witness) {boundary with next := 12} = .scan',
            'SharedUpdates.runFrom update 0 (rows 11) 0 = .complete 8',
        ):
            with self.subTest(claim=claim):
                result = self.prove('example : ' + claim + ' := by decide_cbv', FALSE)
                self.assertEqual(sum(d['severity'] == 'error' for d in result['diagnostics']), 1)

    def test_general_laws_have_checked_axioms(self):
        source = (ROOT / 'VerumExternalReplay/SealedAppend.lean').read_text()
        names = re.findall(r'^theorem (\w+)', source, re.M)
        self.assertEqual(len(names), 17)
        result = self.prove('\n'.join('#print axioms SealedAppend.' + name for name in names))
        rosters = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
        self.assertEqual(len(rosters), len(names))
        for row in rosters:
            self.assertTrue(row.endswith(('does not depend on any axioms',
                'depends on axioms: [propext]', 'depends on axioms: [propext, Quot.sound]')), row)


if __name__ == '__main__':
    unittest.main()
