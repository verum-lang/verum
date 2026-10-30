"""Ordered forest laws, malformed caches and exact predecessor retention."""
from pathlib import Path
import re
import tempfile
import unittest
from check_typed_programs import ROOT, check, require_success, run

HEADER = '''import VerumExternalReplay.PersistentEvents
open PersistentEvents
set_option warningAsError true
def a : Tree Nat := .leaf [10,11,12]
def b : Tree Nat := .leaf [13]
def c : Tree Nat := .leaf [14,15]
def d : Tree Nat := .leaf [16,17,18]
def oldRoots := [join a b,c]
def newRoots := [join (join a b) (join c d)]
def oldSnapshot : Snapshot Nat := ⟨oldRoots,6⟩
def newSnapshot : Snapshot Nat := ⟨newRoots,9⟩
def badChild : Tree Nat := .branch 99 a b
def badRoots := [join badChild c]
def update (s : Nat) (e : Nat) : Except Nat Nat :=
  if e = 14 then .error e else .ok (s + e)
theorem appendTrace : Trace (begin oldRoots [16,17,18]) ⟨newRoots,none⟩ :=
  .step (.step (.step (.refl _) (.merge [join a b] c d))
    (.merge [] (join a b) (join c d))) (.insert [] (join (join a b) (join c d)))
'''
FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'


class PersistentEventsTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.PersistentEvents'))

    def prove(self, source, expected=None):
        with tempfile.TemporaryDirectory(prefix='verum-persistent-events-') as directory:
            path = Path(directory) / 'check.lean'
            path.write_text(HEADER + source)
            return check(path, expected)

    def test_heterogeneous_chunks_and_endpoints(self):
        self.prove('''
example : oldSnapshot.Valid ∧ newSnapshot.Valid := by decide_cbv
example : flatten newRoots = [10,11,12,13,14,15,16,17,18] := by decide_cbv
example : length oldRoots = 6 ∧ length newRoots = 9 := by decide_cbv
example : oldSnapshot.get 5 = some 15 ∧ oldSnapshot.get 6 = none := by decide_cbv
example : newSnapshot.get 6 = some 16 ∧ newSnapshot.get 9 = none := by decide_cbv
example : get ([] : List (Tree Nat)) 0 = none := by decide_cbv
example : get [.leaf [],a,.leaf [],b] 3 = some 13 := by decide_cbv
example : (begin oldRoots []).pending = none := by decide_cbv
example : flatten (oldRoots.take 1) ++ flatten (oldRoots.drop 1) = flatten oldRoots :=
  copy_partition _ _
example : flatten newRoots = flatten oldRoots ++ [16,17,18] :=
  completed_append _ _ _ appendTrace
example : get newRoots 3 = get oldRoots 3 :=
  old_index_preserved _ _ _ (by decide_cbv) appendTrace 3 (by decide_cbv)
example : get newRoots (length oldRoots + 1) = some 17 :=
  suffix_index _ _ _ (by decide_cbv) appendTrace 1
example : SharedUpdates.runFrom update 0 (flatten newRoots) 0 =
    .refused 4 46 14 14 := by decide_cbv
''')

    def test_forks_keep_original_events_and_nominal_slots(self):
        self.prove('''
example : flatten [join (join a b) (join c (.leaf [99]))] =
    flatten oldRoots ++ [99] := by decide_cbv
example : flatten oldRoots = [10,11,12,13,14,15] := by decide_cbv
example : (join (.leaf [(7,1),(7,2)]) (.leaf [(7,3)])).events =
    [(7,1),(7,2),(7,3)] := by decide_cbv
example : (⟨[.leaf (flatten oldRoots)],6⟩ : Snapshot Nat).get 4 = oldSnapshot.get 4 :=
  snapshot_same_events _ _ (by decide_cbv) (by decide_cbv) (by decide_cbv) _
''')

    def test_corrupt_caches_reordering_and_premature_publication_rejected(self):
        claims = [
            'badChild.Valid',
            'Valid badRoots',
            '(⟨oldRoots,1⟩ : Snapshot Nat).Valid',
            '(⟨oldRoots,9⟩ : Snapshot Nat).Valid',
            '(⟨oldRoots,1⟩ : Snapshot Nat).get 2 = oldSnapshot.get 2',
            'get badRoots 4 = (flatten badRoots)[4]?',
            'length oldRoots = oldRoots.length',
            'get oldRoots 6 = some 16',
            'get newRoots 6 = some 15',
            '(join c (join a b)).events = flatten oldRoots',
            'flatten [join a b] = flatten oldRoots',
            'flatten (oldRoots ++ [c]) = flatten oldRoots',
            '(⟨[join a b],some (join c d)⟩ : Carry Nat).events = flatten [join a b]',
            'flatten newRoots = flatten oldRoots',
            'flatten newRoots = flatten oldRoots ++ [16,17,17]',
            '(join (.leaf [(7,1)]) (.leaf [(7,2)])).events = [(7,1),(7,1)]',
            'SharedUpdates.runFrom update 0 (flatten newRoots) 0 = .complete 126',
        ]
        for claim in claims:
            with self.subTest(claim=claim):
                result = self.prove('example : ' + claim + ' := by decide_cbv', FALSE)
                self.assertEqual(sum(d['severity'] == 'error' for d in result['diagnostics']), 1)

    def test_general_laws_have_checked_axioms(self):
        source = (ROOT / 'VerumExternalReplay/PersistentEvents.lean').read_text()
        names = re.findall(r'^theorem (\w+)', source, re.M)
        self.assertEqual(len(names), 23)
        result = self.prove('\n'.join('#print axioms PersistentEvents.' + name for name in names))
        rosters = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
        self.assertEqual(len(rosters), len(names))
        for row in rosters:
            self.assertTrue(row.endswith(('does not depend on any axioms',
                'depends on axioms: [propext]', 'depends on axioms: [propext, Quot.sound]')), row)


if __name__ == '__main__':
    unittest.main()
