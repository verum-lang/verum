"""Kernel controls for retained scans; these are not native runtime observations."""
from pathlib import Path
import json
import tempfile
import unittest
from check_typed_programs import check, require_success, run

HEADER = 'import VerumExternalReplay.RetainedNames\nopen RetainedNames\nset_option warningAsError true\n'


class RetainedNameTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        require_success(run('lake', 'build', 'VerumExternalReplay.RetainedNames'))

    def proof(self, equation, expected=None):
        with tempfile.TemporaryDirectory(prefix='verum-retained-names-') as directory:
            p = Path(directory) / 'check.lean'
            tactic = 'unfold Checked; decide' if equation.startswith('Checked ') else 'decide'
            p.write_text(HEADER + f'example : {equation} := by {tactic}\n')
            check(p, expected)

    def test_model_has_no_added_axioms(self):
        names = ('initial checked_append advance_checked end_absent first_index unpaid_preserves '
                 'pending_preserves named_projects capacity_preserves named_sound '
                 'named_preserves_unique sequential_injective').split()
        with tempfile.TemporaryDirectory(prefix='verum-retained-axioms-') as directory:
            p = Path(directory) / 'audit.lean'
            p.write_text(HEADER + '\n'.join('#print axioms RetainedNames.' + name for name in names))
            result = require_success(run('lake', 'env', 'lean', '--json', str(p)))
            messages = [json.loads(line)['data'] for line in result.splitlines()]
            self.assertEqual(len(messages), len(names))
            for message in messages:
                if 'depends on axioms: [' in message:
                    axioms = message.split('depends on axioms: [')[1].removesuffix(']').split(', ')
                    self.assertLessEqual(set(axioms), {'propext', 'Quot.sound'})
                else:
                    self.assertTrue(message.endswith('does not depend on any axioms'))

    def test_native_shaped_steps(self):
        for equation in (
            'step 3 [257] 1 0 false = ([257], .pending 0)',
            'step 3 [257] 1 0 true = ([257], .pending 1)',
            # A sibling appended the sought key after our previous grant.
            'step 2 [257, 1] 1 1 true = ([257, 1], .named 1)',
            'step 3 [257, 44] 1 1 true = ([257, 44], .pending 2)',
            'step 3 [257, 44] 1 2 true = ([257, 44, 1], .named 2)',
            'step 2 [257, 44] 1 2 true = ([257, 44], .capacity)',
            'step 2 [257, 44] 44 1 true = ([257, 44], .named 1)',
            'step 3 [(0, 257), (1, 1)] (0, 1) 2 true = ([(0, 257), (1, 1), (0, 1)], .named 2)',
            'Checked [257, 44, 1] 1 2',
        ):
            with self.subTest(equation=equation): self.proof(equation)

    def test_invalid_resumptions_rejected(self):
        for equation in (
            # Skipping a sibling's new entry or changing the key invalidates
            # the checked prefix; counters alone cannot authorize resume.
            'Checked [257, 1] 1 2',
            'Checked [257, 44] 257 1',
            'Checked [257] 1 2',
            'step 3 [257, 1] 1 1 true = ([257, 1, 1], .named 2)',
            'step 2 [257, 44] 44 1 true = ([257, 44], .capacity)',
            'step 3 [257] 1 0 false = ([257], .pending 1)',
            'step 3 [(0, 257), (1, 1)] (0, 1) 2 true = ([(0, 257), (1, 1)], .named 1)',
        ):
            with self.subTest(equation=equation):
                self.proof(equation, 'Tactic `decide` proved that the proposition')


if __name__ == '__main__':
    unittest.main()
