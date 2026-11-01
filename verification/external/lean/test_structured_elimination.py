"""Independent logical laws; intended invalid evidence must reach kernel rejection."""
from pathlib import Path
import re
import shutil
import tempfile
import unittest

from check_typed_programs import ROOT, check, require_success, run


class StructuredElimination(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory(prefix="verum-structured-elimination-")
        cls.root = Path(cls.temporary.name)
        for name in ["lakefile.toml", "lean-toolchain", "VerumExternalReplay/StructuredElimination.lean"]:
            target = cls.root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, target)
        output = cls.root / ".lake/build/lib/lean/VerumExternalReplay/StructuredElimination.olean"
        output.parent.mkdir(parents=True, exist_ok=True)
        require_success(run("lake", "env", "lean", "-M", "1024", "-o", str(output),
                            "VerumExternalReplay/StructuredElimination.lean", cwd=cls.root, timeout=60))

    @classmethod
    def tearDownClass(cls):
        cls.temporary.cleanup()

    def test_general_laws_and_complete_axiom_roster(self):
        source = (self.root / "VerumExternalReplay/StructuredElimination.lean").read_text()
        names = re.findall(r"^theorem (\w+)", source, re.M)
        self.assertEqual(names, ["guarded_cases", "signed_induction", "contextual_signed_induction",
                                 "nonnegative_is_not_all_integers", "decidable_contradiction"])
        path = self.root / "audit.lean"
        path.write_text("import VerumExternalReplay.StructuredElimination\n" +
                        "\n".join(f"#print axioms StructuredElimination.{name}" for name in names))
        result = check(path, cwd=self.root, memory_mib=1024)
        self.assertEqual(len(result["diagnostics"]), len(names))
        for diagnostic in result["diagnostics"]:
            text = diagnostic["data"]
            self.assertTrue(text.endswith("does not depend on any axioms") or
                            text.endswith("depends on axioms: [propext]") or
                            text.endswith("depends on axioms: [propext, Quot.sound]"), text)

    def test_omitted_obligations_are_not_proofs(self):
        controls = {
            "missing_case": "example (G : Prop) : G := by exact True.intro",
            "invented_induction_hypothesis": "example (P : Int → Prop) (n : Int) : P n := by exact True.intro",
            "ignored_contradiction_goal": "example (G : Prop) (h : ¬ G → False) : G := by exact h",
            "unsigned_instead_of_signed": (
                "example (Q : Int → Prop) (base : Q 0) "
                "(up : ∀ k, 0 ≤ k → Q k → Q (k + 1)) : ∀ k, Q k := by "
                "exact StructuredElimination.signed_induction Q base up"
            ),
            "hypothesis_at_wrong_target": (
                "example (H P : Int → Prop) (n : Int) (h : H n → P n) "
                "(next : H (n + 1)) : P n := by exact h next"
            ),
        }
        for name, source in controls.items():
            with self.subTest(name=name):
                path = self.root / f"{name}.lean"
                path.write_text("import VerumExternalReplay.StructuredElimination\n" + source + "\n")
                # A type mismatch is the intended failure; infrastructure,
                # parser, resource and import failures are never success.
                expected = "Application type mismatch" if name == "hypothesis_at_wrong_target" else "Type mismatch"
                result = check(path, expected, cwd=self.root, memory_mib=1024)
                self.assertFalse(result["accepted"])


if __name__ == "__main__":
    unittest.main()
