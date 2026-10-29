"""General finite-history risk laws and discriminating probability-tree controls."""
from pathlib import Path
import re
import tempfile
import unittest
from check_typed_programs import ROOT, check, require_success, run

HEADER = '''import VerumExternalReplay.FiniteRisk
open ExactMass FiniteRisk
set_option warningAsError true
inductive Node where
  | root | left | right | a | b | c | d
  deriving DecidableEq
open Node
def children : Node → List Node
  | root => [left, right]
  | left => [a, b]
  | right => [c, d]
  | h => [h]
def p : Node → Fraction
  | root => one
  | left | right => unit 2 (by decide)
  | _ => unit 4 (by decide)
def q : Node → Fraction
  | root | left | a => one
  | _ => zero
def qb : Node → Fraction
  | root | left | b => one
  | _ => zero
def quarter := unit 4 (by decide)
def half := unit 2 (by decide)
def hit := crosses quarter q p
theorem p_flow : Flow children p := by intro h; cases h <;> decide_cbv
theorem q_flow : Subflow children q := by intro h; cases h <;> decide_cbv
theorem qb_flow : Subflow children qb := by intro h; cases h <;> decide_cbv
'''
FALSE = '`decide_cbv` failed: the proposition evaluates to `false`'


class FiniteRiskTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.FiniteRisk'))

    def prove(self, source, expected=None):
        with tempfile.TemporaryDirectory(prefix='verum-finite-risk-') as directory:
            path = Path(directory) / 'check.lean'
            path.write_text(HEADER + source)
            return check(path, expected)

    def test_arbitrary_horizon_and_exact_first_crossing(self):
        self.prove('''
example (horizon : Nat) : AtMost (eventMass children p hit horizon root) quarter :=
  envelope_crossing_bound children p q p quarter p_flow q_flow root
    (le_refl _) (fun _ => le_refl _) horizon
example (horizon : Nat) : Equal (eventMass children p hit horizon root)
    (stopped children p hit horizon root) :=
  event_is_first_hit children p p_flow hit horizon root
example : Equal (eventMass children p hit 0 root) zero := by decide_cbv
example : Equal (eventMass children p hit 1 root) zero := by decide_cbv
example : Equal (eventMass children p hit 2 root) quarter := by decide_cbv
example : Equal (eventMass children p hit 4 root) quarter := by decide_cbv
-- A root hit is counted once, despite all subsequent opportunities.
example : Equal (eventMass children p (fun _ => true) 4 root) one := by decide_cbv
-- A zero-mass history can satisfy the non-strict threshold without division.
example : crosses zero (fun _ : Node => zero) (fun _ => zero) a = true := by decide_cbv
example : Equal (eventMass children (fun _ => zero)
    (crosses zero (fun _ => zero) (fun _ => zero)) 3 root) zero := by decide_cbv
''')

    def test_family_allocation_and_correlated_events(self):
        self.prove('''
def predictors (i : Bool) := if i then qb else q
example (horizon : Nat) : AtMost (eventMass children p
    (anyHit ([false, true].map (fun i => crosses (mul half half) (predictors i) p)))
    horizon root) half := by
  apply envelope_family_bound children p predictors (fun _ => p) (fun _ => half)
    [false, true] half p_flow root
  · intro i _; cases i <;> first | exact q_flow | exact qb_flow
  · intro i _; cases i <;> exact le_refl _
  · intro _ _ h; exact le_refl (p h)
  · decide_cbv
example : Equal (eventMass children p
    (anyHit [crosses quarter q p, crosses quarter qb p]) 2 root) half := by decide_cbv
-- Duplicated, perfectly correlated events are allowed, and still cost two allocations.
example (n : Nat) : AtMost (eventMass children p (anyHit [hit, hit]) n root)
    (sum [eventMass children p hit n root, eventMass children p hit n root]) :=
  event_union_bound children p [hit, hit] n root
example : Equal (eventMass children p (anyHit [hit, hit]) 2 root) quarter := by decide_cbv
example : Equal (eventMass children p (anyHit []) 3 root) zero := by decide_cbv
''')

    def test_all_universal_laws_have_only_standard_foundations(self):
        source = (ROOT / 'VerumExternalReplay/FiniteRisk.lean').read_text()
        names = re.findall(r'^theorem (\w+)', source, re.M)
        self.assertEqual(len(names), 10)
        result = self.prove('\n'.join('#print axioms FiniteRisk.' + n for n in names))
        rows = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
        self.assertEqual(len(rows), len(names))
        for row in rows:
            self.assertTrue(row.endswith(('does not depend on any axioms',
                                          'depends on axioms: [propext]',
                                          'depends on axioms: [propext, Quot.sound]')), row)

    def test_invalid_flow_envelope_allocation_and_stopping_claims_refuse(self):
        claims = {
            'inflated_initial_capital': 'AtMost (add (q root) (q root)) one',
            'missing_child_budget': 'AtMost (sum [one, one]) (q root)',
            'underestimated_null_envelope': 'AtMost (p left) quarter',
            'discarded_negative_branch': 'Equal (sum [p a]) (p left)',
            'repeated_crossing_is_not_first_hit':
                'Equal (add (eventMass children p hit 2 root) (eventMass children p hit 3 root)) '
                '(eventMass children p hit 3 root)',
            'unbudgeted_family': 'AtMost (sum [half, half, half]) one',
            'unpaid_second_test': 'AtMost (eventMass children p '
                '(anyHit [hit, crosses quarter qb p]) 2 root) quarter',
            'correlation_is_not_independence': 'Equal (eventMass children p '
                '(anyHit [hit, hit]) 2 root) (mul quarter quarter)',
            'risk_cannot_be_zero': 'AtMost (eventMass children p hit 2 root) zero',
            'horizon_includes_last_observation': 'Equal (eventMass children p hit 2 root) zero',
            'equality_is_a_crossing': 'hit a = false',
            'null_zero_threshold_is_defined':
                'crosses zero (fun _ : Node => zero) (fun _ => zero) a = false',
        }
        for name, claim in claims.items():
            with self.subTest(name=name):
                result = self.prove('example : ' + claim + ' := by decide_cbv', FALSE)
                errors = [d for d in result['diagnostics'] if d['severity'] == 'error']
                self.assertEqual(len(errors), 1)
                self.assertIn('proposition evaluates to `false`', errors[0]['data'])


if __name__ == '__main__':
    unittest.main()
