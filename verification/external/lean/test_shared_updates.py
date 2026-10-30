"""Pure shared-update laws; altered prefixes and hidden effects must not qualify."""
from pathlib import Path
import re
import tempfile
import unittest
from check_typed_programs import ROOT, check, require_success, run

HEADER = '''import VerumExternalReplay.SharedUpdates
open SharedUpdates
set_option warningAsError true
def step (state event : Nat) : Except Nat Nat :=
  if event = 0 then .error 7 else .ok (state * 10 + event)
def hostStep (counter : Nat) : Nat × Nat := (counter + 1, counter)
def separateHost := let a := hostStep 0; let b := hostStep a.1; (a.2, b.2)
def sharedHost := let a := hostStep 0; (a.2, a.2)
'''
FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'


class SharedUpdateTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.SharedUpdates'))

    def prove(self, source, expected=None):
        with tempfile.TemporaryDirectory(prefix='verum-shared-updates-') as directory:
            path = Path(directory) / 'check.lean'
            path.write_text(HEADER + source)
            return check(path, expected)

    def test_general_factorization_and_exact_refusal(self):
        self.prove('''
example (other : Nat → Nat → Except Nat Nat)
    (same : ∀ s e, step s e = other s e) (events : List Nat)
    (read : Nat → R) :
    observe read (runFrom step 0 events 0) = observe read (runFrom other 0 events 0) :=
  separate_reads step other same 0 0 rfl events read
example (suffix : List Nat) :
    runFrom step 0 ([2, 3] ++ 0 :: suffix) 0 = .refused 2 23 0 7 :=
  first_refusal step [2, 3] suffix 0 23 0 7 (by decide_cbv) rfl
example (suffix : List Nat) :
    runFrom step 0 ([2, 3, 0] ++ suffix) 0 = .refused 2 23 0 7 :=
  refusal_retained step [2, 3, 0] suffix 0 23 0 7 2 (by decide_cbv)
example : observe (fun n => n + 10) (runFrom step 0 [2, 3] 0) = some 33 := by decide_cbv
example : observe (fun n => n == 23) (runFrom step 0 [2, 3] 0) = some true := by decide_cbv
example : separateHost = (0, 1) ∧ sharedHost = (0, 0) := by decide_cbv
''')

    def test_false_correspondence_is_rejected(self):
        for claim in (
            'runFrom step 0 [2, 3, 0] 0 = .refused 3 23 0 7',
            'runFrom step 0 [2, 3, 0] 0 = .refused 2 23 0 8',
            'runFrom step 0 [2, 3, 0, 9] 0 = .complete 239',
            'runFrom step 0 [2, 3] 0 = runFrom step 0 [3, 2] 0',
            'runFrom step 0 [2, 3] 0 = runFrom step 0 [2, 3] 1',
            'runFrom step 0 [2, 3] 0 = runFrom (fun s e => Except.ok (s + e)) 0 [2, 3] 0',
            'observe (fun n => n + 10) (runFrom step 0 [2, 3] 0) = some 23',
            'separateHost = sharedHost',
        ):
            with self.subTest(claim=claim):
                result = self.prove('example : ' + claim + ' := by decide_cbv', FALSE)
                self.assertEqual(sum(d['severity'] == 'error' for d in result['diagnostics']), 1)

    def test_universal_proofs_have_no_unchecked_axioms(self):
        source = (ROOT / 'VerumExternalReplay/SharedUpdates.lean').read_text()
        names = re.findall(r'^theorem (\w+)', source, re.M)
        self.assertEqual(len(names), 6)
        result = self.prove('\n'.join('#print axioms SharedUpdates.' + name for name in names))
        rosters = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
        self.assertEqual(len(rosters), len(names))
        for row in rosters:
            self.assertTrue(row.endswith(('does not depend on any axioms',
                'depends on axioms: [propext]', 'depends on axioms: [propext, Quot.sound]')), row)


if __name__ == '__main__':
    unittest.main()
