"""Audit unconditional rational lemmas, not the still-open full KT bound.

Core Rat arithmetic imports standard Classical.choice through exact integer
division. This suite declares that dependency explicitly; it does not widen
the stricter native finite-replay axiom policy.
"""
from pathlib import Path
import tempfile
import unittest
from check_typed_programs import check, require_success, run

HEADER = 'import VerumExternalReplay.RefinementFeasibility\nopen RefinementFeasibility\n'


class RefinementFeasibilityTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.RefinementFeasibility'))

    def test_universal_lemma_axioms(self):
        names = ('cubic_step coefficient_nonneg cubic_lower gap_identity '
                 'cubic_gap_positive normalized_gap').split()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'axioms.lean'
            path.write_text(HEADER + '\n'.join(f'#print axioms {name}' for name in names))
            result = check(path)
        roster = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
        self.assertEqual(len(roster), len(names))
        for entry in roster:
            self.assertTrue(entry.endswith('depends on axioms: [propext, Classical.choice, Quot.sound]'), entry)

    def test_boundary_and_exact_rational_controls(self):
        text = HEADER + '''
example : cubic 0 (1/2) = 1 := by (try unfold cubic); grind
example : cubic 4 (1/2) = 0 := by (try unfold cubic); grind
example : cubic 5 (1/2) = -1/4 := by (try unfold cubic); grind
example : cubic 8 1 ≤ (0 : Rat)^8 := by (try unfold cubic); grind
example : cubic 13 0 = (1 : Rat)^13 := by (try unfold cubic); grind
example : (3 : Rat) < 4 := by have h := normalized_gap 0; grind
example : (5 : Rat) < (3 : Rat)*(3/2)^2*(3/4) := by have h := normalized_gap 1; grind
example (n : Nat) : cubic n (1/2) ≤ (1/2 : Rat)^n :=
  by
    have h := cubic_lower n (1/2) (by grind) (by grind)
    rw [show (1-(1/2 : Rat)) = 1/2 by grind] at h
    exact h
'''
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'boundaries.lean'; path.write_text(text); check(path)

    def test_wrong_sign_coefficient_and_domain_are_rejected(self):
        claims = [
            '(1/2 : Rat)^4 ≤ cubic 4 (1/2)',
            '(3 : Rat) < (2-2)*(1+1)^2*(1-1)^0',
            'cubic 13 3 ≤ (1-3 : Rat)^13',
            '(3 : Rat)*(3/2)^2*cubic 1 (1/4)-5 = '
                '(2^6+2^2*(5*2-6)*(2-1)+11*2-6)/(2^8)',
        ]
        with tempfile.TemporaryDirectory() as directory:
            for n, proposition in enumerate(claims):
                with self.subTest(proposition=proposition):
                    path = Path(directory) / f'negative_{n}.lean'
                    path.write_text(HEADER + f'example : {proposition} := by (try unfold cubic); grind\n')
                    check(path, '`grind` failed')


if __name__ == '__main__': unittest.main()
