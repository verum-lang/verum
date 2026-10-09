#!/usr/bin/env python3
"""Exercise the real tracked-file reference gate in isolated Git repositories."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
GATE = ROOT / "scripts/ci/check_no_internal_refs.sh"
PRIVATE_DIR = "internal"


def private_path(tail):
    # Assemble adversarial fixture content without adding a forbidden reference
    # to this tracked test source or an exemption to the gate.
    return "/".join((PRIVATE_DIR, tail))


class InternalReferenceBoundaryTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="verum-reference-gate-")
        self.addCleanup(temporary.cleanup)
        self.repo = Path(temporary.name)
        self.env = os.environ.copy()
        for key in ("GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR"):
            self.env.pop(key, None)
        self.env.update(GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1")
        self.run_command(["git", "init", "-q"]).check_returncode()
        self.fixture = self.repo / "fixture.md"
        self.fixture.write_text("fixture\n")
        self.run_command(["git", "add", "fixture.md"]).check_returncode()

    def run_command(self, command):
        return subprocess.run(
            command, cwd=self.repo, env=self.env, text=True,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=10,
        )

    def assert_gate(self, contents, forbidden):
        self.fixture.write_text(contents + "\n")
        result = self.run_command(["bash", str(GATE)])
        self.assertEqual(result.returncode, int(forbidden), result.stdout + result.stderr)
        if forbidden:
            self.assertIn(f"FORBIDDEN {PRIVATE_DIR}/-directory references", result.stderr)
            self.assertIn("fixture.md:1:", result.stderr)
        else:
            self.assertIn("check-internal-refs: OK", result.stdout)
            self.assertIn("check-memory-refs: OK", result.stdout)

    def test_actual_directory_paths_are_refused(self):
        path = private_path("design.md")
        cases = {
            "relative file": path,
            "current directory": "./" + path,
            "parent directory": "../" + path,
            "absolute file": "/repo/" + path,
            "directory without extension": private_path("specs/draft"),
            "JSON file": private_path("manifest.json"),
            "Markdown relative": f"[design]({path})",
            "Markdown absolute": f"[design](/repo/{path})",
            "Markdown angle target": f"[design](</repo/{path}>)",
            "HTML relative": f'<a href="{path}">design</a>',
            "HTML absolute": f"<a href='/repo/{path}'>design</a>",
            "inline code": f"`{path}`",
            "URL path": f"https://example.invalid/{path}",
            "nested under a suffixed directory": f"vendor-internal/{path}",
        }
        for label, contents in cases.items():
            with self.subTest(case=label):
                self.assert_gate(contents, forbidden=True)

    def test_filename_prefixes_do_not_create_a_private_directory(self):
        prefixes = ("pin-project-", "vendor-", "-", "module_", "module.", "x")
        for prefix in prefixes:
            for tail in ("guide.md", "build/output"):
                with self.subTest(prefix=prefix, tail=tail):
                    self.assert_gate(prefix + private_path(tail), forbidden=False)
        # Preserve the existing prose exception: no recognized extension or
        # further directory separator follows this word.
        self.assert_gate(private_path("protected"), forbidden=False)

    def test_inherited_receipt_arguments_are_allowed_verbatim(self):
        evidence = ROOT / "docs/architecture/evidence/registry-stage20-build"
        for filename in ("archive-inspector-build.json", "metadata-inspector-build.json"):
            with self.subTest(receipt=filename):
                raw = (evidence / filename).read_bytes()
                command = json.loads(raw)["command"]
                arguments = [arg for arg in command if "pin-project-internal/" in arg]
                self.assertEqual(len(arguments), 1, filename)
                # The exact retained dependency string, with no redaction or
                # path normalization, becomes the tracked fixture's content.
                self.assert_gate(arguments[0], forbidden=False)
                self.assertEqual((evidence / filename).read_bytes(), raw)


if __name__ == "__main__":
    unittest.main()
