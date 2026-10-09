#!/usr/bin/env python3
"""Exercise the production prepass gate against exact and mutated source copies."""
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
GATE = Path("scripts/ci/check_bake_prepass_parity.py")
CODEGEN = Path("crates/verum_vbc/src/codegen/mod.rs")
BOOTSTRAP = Path("crates/verum_compiler/src/pipeline/stdlib_bootstrap.rs")


class BakePrepassBoundaryTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="verum-prepass-gate-")
        self.addCleanup(temporary.cleanup)
        self.repo = Path(temporary.name)
        for relative in (GATE, CODEGEN, BOOTSTRAP):
            destination = self.repo / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes((ROOT / relative).read_bytes())

    def replace_once(self, relative, before, after):
        path = self.repo / relative
        source = path.read_text(encoding="utf-8")
        self.assertEqual(source.count(before), 1, f"fixture anchor: {before}")
        path.write_text(source.replace(before, after, 1), encoding="utf-8")

    def run_gate(self):
        # These are source-gate inputs, not Rust programs to compile. Running
        # the copied production script preserves its real path discovery and
        # command-line exit status without touching either production source.
        return subprocess.run(
            [sys.executable, str(self.repo / GATE)],
            cwd=self.repo, text=True, stdout=subprocess.PIPE,
            stderr=subprocess.PIPE, timeout=10,
        )

    def assert_refused(self, diagnostic):
        result = self.run_gate()
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn(diagnostic, result.stdout)

    def test_current_sources_are_classified(self):
        result = self.run_gate()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("pre-pass(es) classified", result.stdout)

    def test_new_shared_collector_is_refused(self):
        signature = (
            "pub fn collect_unit_declarations(&mut self, files: &[&Module]) "
            "-> CodegenResult<()> {"
        )
        self.replace_once(
            CODEGEN, signature,
            signature + "\n        self.collect_unclassified_regression(files);",
        )
        self.assert_refused("UNCLASSIFIED pre-pass `collect_unclassified_regression`")

    def test_new_grouped_collector_is_refused(self):
        signature = "pub fn run_unit_declaration_prepasses(&mut self, files: &[&Module]) {"
        self.replace_once(
            CODEGEN, signature,
            signature + "\n        self.collect_unclassified_nested(files);",
        )
        self.assert_refused("UNCLASSIFIED pre-pass `collect_unclassified_nested`")

    def test_removed_prepass_leaves_a_stale_entry(self):
        call = "self.collect_count_module_owners(std::slice::from_ref(item), &owner);"
        self.replace_once(CODEGEN, call, "")
        self.assert_refused("STALE COVERAGE entry `collect_count_module_owners`")

    def test_direct_bootstrap_collector_is_refused(self):
        call = "codegen.collect_unit_declarations(ast_modules)"
        self.replace_once(
            BOOTSTRAP, call,
            "codegen.collect_unclassified_bootstrap(ast_modules);\n        " + call,
        )
        self.assert_refused("the bake calls `codegen.collect_unclassified_bootstrap(...)` directly")


if __name__ == "__main__":
    unittest.main()
