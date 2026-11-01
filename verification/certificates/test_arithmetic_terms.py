"""Independent Lean kernel checking of the actual portable arithmetic terms."""
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

import generate_peano_arithmetic
import lean_arithmetic_terms
from portable_terms import data, lower, RECURSIVE


class ArithmeticTerms(unittest.TestCase):
    CASES = staticmethod(generate_peano_arithmetic.cases)

    def test_exact_terms_and_false_controls(self):
        lean = shutil.which('lean')
        self.assertIsNotNone(lean, 'Lean is required; replay must not be skipped')
        cases = self.CASES()
        batches = [[case for case in cases if case[2]]]
        batches.extend([case] for case in cases if not case[2])
        with tempfile.TemporaryDirectory(prefix='verum-arithmetic-terms-') as temporary:
            for index, batch in enumerate(batches):
                with self.subTest(cases=[case[0] for case in batch]):
                    path = Path(temporary) / f'Proof{index}.lean'
                    path.write_text(lean_arithmetic_terms.source(batch))
                    result = subprocess.run([lean, '-M1536', '--json', str(path)],
                                            text=True, capture_output=True, timeout=60)
                    self.assertEqual(result.stderr, '')
                    messages = [json.loads(line) for line in result.stdout.splitlines() if line.strip()]
                    errors = [row for row in messages if row['severity'] == 'error']
                    self.assertFalse(any(row['severity'] == 'warning' for row in messages), messages)
                    if batch[0][2]:
                        self.assertEqual(result.returncode, 0, messages)
                        self.assertEqual(errors, [])
                        self.assertEqual([row['data'] for row in messages],
                            [f"'PortableArithmetic.proof{i}' does not depend on any axioms" for i in range(len(batch))])
                    else:
                        self.assertEqual(result.returncode, 1, messages)
                        self.assertEqual(len(errors), 1, messages)
                        self.assertTrue(errors[0]['data'].startswith(('Type mismatch', 'Application type mismatch')), errors)

    def test_exporter_refuses_unimplemented_or_changed_data(self):
        for term in ({'SmtProof': {}}, {'Var': -1}, {'Universe': {'Var': 'u'}},
                     lower(data('Nat', [[]])), {'Data': {'name': 'Bad', 'level': {'Concrete': 0}, 'constructors': [[{'Dependent': {'Var': 0}}]]}},
                     lower(data('Nat', [[], [RECURSIVE]], level=1))):
            with self.subTest(term=term), self.assertRaises(ValueError):
                lean_arithmetic_terms.Exporter().term(term)


if __name__ == '__main__':
    unittest.main()
