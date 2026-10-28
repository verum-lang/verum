"""Kernel controls for the generic lifecycle, separate from native evidence."""
from pathlib import Path
import tempfile
import unittest
from check_typed_programs import check, require_success, run


class AffineServiceTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.AffineService'))

    def test_laws_and_axioms(self):
        laws = ('poison_precedes_consumption poisoned_budget_refuses consumption_preserves_result '
                'enter_iff consumed_never_enters consumed_retry_inert poisoned_retry_inert '
                'failed_stays_consumed returned_stays_consumed retained_keeps_owner').split()
        text = 'import VerumExternalReplay.AffineService\n' + '\n'.join(
            f'#print axioms AffineService.{name}' for name in laws)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'audit.lean'; path.write_text(text)
            result = check(path)
        roster = [d['data'] for d in result['diagnostics'] if d['severity'] == 'information']
        self.assertEqual(len(roster), len(laws))
        self.assertTrue(all(s.endswith('does not depend on any axioms') or
                            s.endswith('depends on axioms: [propext]') for s in roster), roster)

    def test_false_lifecycle_claims_rejected(self):
        claims = [
            'gate (settle (.failed "Resource")) false = .enter',
            'gate (settle (.returned : Outcome String)) false = .poisoned',
            'gate (⟨true, some .returned⟩ : State String) false = .consumed .returned',
            'gate (settle (.retained : Outcome String)) true = .enter',
            '(serve (settle (.returned : Outcome String)) false 2 '
                '(fun n => (.returned, n + 1, ["publish"]))).effects = ["publish"]',
            '(serve (settle (.failed "Resource")) false 2 '
                '(fun n => (.returned, n + 1, ["charge"]))).payload = 3',
        ]
        with tempfile.TemporaryDirectory() as directory:
            for n, proposition in enumerate(claims):
                with self.subTest(proposition=proposition):
                    path = Path(directory) / f'negative_{n}.lean'
                    path.write_text('import VerumExternalReplay.AffineService\nopen AffineService\n'
                                    f'example : {proposition} := by rfl\n')
                    check(path, 'Tactic `rfl` failed:')


if __name__ == '__main__': unittest.main()
