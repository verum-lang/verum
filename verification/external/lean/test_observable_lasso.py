"""Closed-lasso quotients, full primitive observable period and exact false controls."""
from pathlib import Path
import re
import tempfile
import unittest
from check_typed_programs import ROOT, check, require_success, run

HEADER = '''import VerumExternalReplay.ObservableLasso
open ObservableLasso
set_option warningAsError true
def alternating (i : Nat) : Nat := i % 2
def visible (i : Nat) : Nat := if i = 0 then 9 else i % 2
def constant (_ : Nat) : Nat := 7
theorem invisibleTransient : Candidate 3 1 alternating 2 :=
  ⟨by decide, by decide, fun _ _ => by simp [alternating]⟩
'''
FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'


class ObservableLassoTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.ObservableLasso'))

    def prove(self, source, expected=None):
        with tempfile.TemporaryDirectory(prefix='verum-observable-lasso-') as directory:
            path = Path(directory) / 'check.lean'
            path.write_text(HEADER + source)
            return check(path, expected)

    def test_transient_quotient_and_primitive_period(self):
        self.prove('''
example : (List.range 7).map (walk 3 1) = [0,1,2,1,2,1,2] := by decide_cbv
example : (List.range 7).map (word 3 1 alternating) = [0,1,0,1,0,1,0] := by decide_cbv
example : (List.range 3).map (fun i => i % 2) = [0,1,0] := by decide_cbv
example : next 3 1 2 % 2 = (2 % 2 + 1) % 2 := by decide_cbv
example : word 3 1 alternating (10^40 + 1) = 1 := by
  rw [word_canonical (by decide) invisibleTransient]; decide_cbv
example : Periodic (word 3 1 alternating) 2 :=
  candidate_period (by decide) invisibleTransient
theorem minimum : ∀ d, Candidate 3 1 alternating d → 2 ≤ d := by
  intro d h
  by_cases small : d < 2
  · have one : d = 1 := by have positive := h.1; omega
    subst d
    have contradiction := h.2.2 1 (by decide)
    change 1 = 0 at contradiction
    contradiction
  · omega
example (d : Nat) (positive : 0 < d) (h : Periodic (word 3 1 alternating) d) : 2 ≤ d :=
  minimal_candidate_primitive (by decide) invisibleTransient minimum positive h
example (d : Nat) (h : Periodic (word 3 1 alternating) d) : 2 ∣ d :=
  minimal_candidate_divides_every_period (by decide) invisibleTransient minimum h
example (actions : List Unit) :
    (ConservativeRefinement.run (cycleStep 2) actions 0).states.map alternating =
    (ConservativeRefinement.run (rawStep 3 1) actions 0).states.map alternating :=
  finite_observations_preserved (i := 0) (by decide) invisibleTransient actions (by decide)
example : Candidate 5 3 constant 1 := ⟨by decide, by decide, fun _ _ => rfl⟩
-- A nonprimitive observable period need NOT divide the concrete cycle length.
example : Periodic (word 3 1 alternating) 4 ∧ ¬ 4 ∣ 3 - 1 :=
  ⟨fun t => periodic_multiple (candidate_period (by decide) invisibleTransient) 2 t, by decide⟩
example : Periodic (word 3 1 constant) 3 ∧ ¬ 3 ∣ 3 - 1 := ⟨fun _ => rfl, by decide⟩
example : Candidate 1 0 constant 1 := ⟨by decide, by decide, fun _ _ => rfl⟩
example (p : Nat) (positive : 0 < p) (h : Periodic (word 3 1 alternating) p) :
    Candidate 3 1 alternating 2 :=
  (pure_period_iff_full_candidate (by decide)).mp ⟨p, positive, h⟩
example : ¬ ∃ p, 0 < p ∧ Periodic (word 3 1 visible) p := by
  apply (exhaustion_reflects_nonperiodicity (by decide : 1 < 3)).mp
  intro p positive bounded candidate
  have mismatch := candidate.2.2 2 (by decide)
  have options : p = 1 ∨ p = 2 := by omega
  rcases options with one | two
  · subst p; contradiction
  · subst p; contradiction
example : Periodic (word 3 1 constant) 2 :=
  pure_period_implies_tail_period (by decide) (by decide : 0 < 3) (fun _ => rfl)
example : ∀ d, Candidate 3 1 alternating d → 2 ≤ d :=
  earlier_refusals_establish_minimum (fun d _ before valid => by
    have bound := minimum d valid
    omega)
''')

    def test_missing_premises_and_false_strengthenings_rejected(self):
        for claim in (
            '0 < (0 : Nat)',
            '0 < (0 : Nat) - 0',
            '(3 : Nat) < 3',
            'next 3 3 2 < 3',
            'next 3 0 2 % 2 = (2 % 2 + 1) % 2',
            '(2 : Nat) ∣ 3 - 0',
            'visible 2 = visible (2 % 2)',
            'word 3 1 visible 2 = word 3 1 visible 0',
            '(List.range 3).map alternating = [0,0,0]',
            'next 3 1 2 = 0',
            'word 3 1 alternating 5 = word 3 1 alternating 0',
            '(4 : Nat) ∣ 3 - 1',
            '(3 : Nat) ∣ 3 - 1',
            'next 3 1 2 % 4 = (2 % 4 + 1) % 4',
        ):
            with self.subTest(claim=claim):
                result = self.prove('example : ' + claim + ' := by decide_cbv', FALSE)
                self.assertEqual(sum(d['severity'] == 'error' for d in result['diagnostics']), 1)

    def test_general_laws_have_checked_axioms(self):
        source = (ROOT / 'VerumExternalReplay/ObservableLasso.lean').read_text()
        names = re.findall(r'^theorem (\w+)', source, re.M)
        self.assertEqual(len(names), 29)
        result = self.prove('\n'.join('#print axioms ObservableLasso.' + name for name in names))
        rosters = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
        self.assertEqual(len(rosters), len(names))
        for row in rosters:
            self.assertTrue(row.endswith(('does not depend on any axioms',
                'depends on axioms: [propext]', 'depends on axioms: [propext, Quot.sound]')), row)


if __name__ == '__main__':
    unittest.main()
