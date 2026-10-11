"""Fail-closed accounting for retained SemVer comparison observations."""
import importlib.util
from pathlib import Path
import unittest


PATH = Path(__file__).resolve().parents[1] / "check_semver_publication.py"
SPEC = importlib.util.spec_from_file_location("semver_publication", PATH)
CONTRACT = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CONTRACT)
CASES = [dict(id="exact", track="publication", requirement="=1.2.3", accepts=True,
              candidates=[dict(version="1.2.3", matches=True), dict(version="1.2.4", matches=False)])]
GOOD = "semver-contract|begin|1\nparse|exact|1\nmatch|exact|0|1\nmatch|exact|1|0\nsemver-contract|end|1\n"


class ComparisonAccounting(unittest.TestCase):
    def test_complete_positive(self):
        actual = CONTRACT.parse_output(GOOD, CASES)
        self.assertEqual(CONTRACT.differences(actual, CASES), [])
        self.assertEqual(len(actual["exact"]["matches"]), 2)

    def test_wrong_match_is_semantic_failure(self):
        actual = CONTRACT.parse_output(GOOD.replace("match|exact|1|0", "match|exact|1|1"), CASES)
        self.assertEqual(CONTRACT.differences(actual, CASES), [
            dict(case="exact", axis="matching", version="1.2.4", expected=False, actual=True)])

    def test_rejected_positive_is_acceptance_failure(self):
        actual = CONTRACT.parse_output("semver-contract|begin|1\nparse|exact|0\nsemver-contract|end|1\n", CASES)
        self.assertEqual(CONTRACT.differences(actual, CASES), [
            dict(case="exact", axis="acceptance", expected=True, actual=False)])

    def test_incomplete_duplicate_and_unknown_rows_are_runner_errors(self):
        invalid = [
            "", GOOD.replace("semver-contract|end|1\n", ""),
            GOOD.replace("match|exact|1|0\n", ""),
            GOOD.replace("parse|exact|1\n", "parse|exact|1\nparse|exact|1\n"),
            GOOD.replace("match|exact|1|0", "match|exact|0|0"),
            GOOD.replace("match|exact|1|0", "match|exact|2|0"),
            GOOD.replace("match|exact|1|0", "match|foreign|1|0"),
            GOOD.replace("match|exact|1|0", "match||1|0"),
            GOOD.replace("match|exact|1|0", "match|exact|1|maybe"),
            GOOD.replace("match|exact|1|0", "candidate-error|exact|1"),
            GOOD.replace("parse|exact|1\n", ""),
            GOOD.replace("parse|exact|1", "parse|exact|0"),
            GOOD + "unexpected output\n",
        ]
        for output in invalid:
            with self.subTest(output=output), self.assertRaises(ValueError):
                CONTRACT.parse_output(output, CASES)

    def test_zero_coverage_is_never_success(self):
        with self.assertRaises(ValueError):
            CONTRACT.parse_output("semver-contract|begin|0\nsemver-contract|end|0\n", [])

    def test_extensions_are_separate_from_publication_reference(self):
        cases = [dict(CASES[0], track="extension")]
        accepted = CONTRACT.parse_output(GOOD, cases)
        self.assertEqual(CONTRACT.differences(accepted, cases), [])
        self.assertEqual(CONTRACT.differences(accepted, cases, reference=True), [
            dict(case="exact", axis="acceptance", expected=False, actual=True)])

    def test_acceptance_fixture_is_independent_of_version_matching(self):
        generated = CONTRACT.verum_source([dict(CASES[0], candidates=[])])
        self.assertIn("parse_requirement(requirement)", generated)
        self.assertNotIn("matches_default", generated)
        self.assertNotIn("SemVer.parse", generated)
        self.assertNotIn("List<Text>", generated)

    def test_explicit_entry_points_preserve_parse_only_and_matching_shapes(self):
        for entry in ("parse_cargo", "parse_extended", "parse"):
            for candidates in ([], CASES[0]["candidates"]):
                with self.subTest(entry=entry, matching=bool(candidates)):
                    generated = CONTRACT.verum_source([dict(CASES[0], candidates=candidates)], entry)
                    self.assertIn(f"{{{entry} as parse_requirement", generated)
                    self.assertEqual("matches_default" in generated, bool(candidates))
        with self.assertRaises(ValueError):
            CONTRACT.verum_source(CASES, "private_parser")

    def test_corpus_has_required_independent_axes(self):
        corpus = CONTRACT.read_cases(CONTRACT.CORPUS)
        identifiers = {case["id"] for case in corpus["cases"]}
        self.assertTrue({"bare_zero", "caret_zero_minor", "gt_minor", "exact_major", "operator_spaces",
                         "prerelease_and", "build_ignored", "empty", "overflow_major", "overflow_minor",
                         "overflow_patch", "leading_zero_prerelease", "extension_or"} <= identifiers)
        self.assertTrue(any(case["accepts"] for case in corpus["cases"]))
        self.assertTrue(any(not case["accepts"] for case in corpus["cases"]))
        generated = CONTRACT.verum_source(CASES)
        self.assertIn("SemVer.parse(&versions[index])", generated)
        self.assertIn("parse_requirement(requirement)", generated)
        self.assertIn("matches_default(&constraint, &version)", generated)
        self.assertIn("semver-contract|end|1", generated)


if __name__ == "__main__":
    unittest.main()
