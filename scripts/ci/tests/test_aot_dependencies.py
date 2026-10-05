#!/usr/bin/env python3
"""Dependency guard regressions; no compiler build or executable run required."""
import contextlib
import importlib.util
import io
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("aot_guard", Path(__file__).parents[1] / "check_aot_dependencies.py")
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)

ELF = """ELF Header:
  Type: EXEC (Executable file)
Program Headers:
  LOAD 0x0 0x0
There is no dynamic section in this file.
No version information found in this file.
"""
PE = """File: control.exe
Format: COFF-x86-64
ImageFileHeader {
  Characteristics [ (0x22)
    IMAGE_FILE_EXECUTABLE_IMAGE (0x2)
  ]
}
"""


class GuardTests(unittest.TestCase):
    def test_static_elf_dynamic_boundary(self):
        self.assertEqual(guard.forbidden(guard.parse_elf(ELF)), [])

    def test_all_linux_imports_are_checked_not_only_libc(self):
        for name in ["libc.so.6", "libc.musl-x86_64.so.1", "libpthread.so.0", "libssl.so.3", "libunknown.so"]:
            with self.subTest(name=name):
                text = ELF + f"  0x1 (NEEDED) Shared library: [{name}]\n"
                self.assertEqual(guard.forbidden(guard.parse_elf(text)), [name])

    def test_elf_loader_and_symbol_versions_are_not_exempt(self):
        text = ELF + "  INTERP 0x0\n [Requesting program interpreter: /lib64/ld-linux-x86-64.so.2]\n Name: GLIBC_2.39\n"
        denied = guard.forbidden(guard.parse_elf(text))
        self.assertEqual(len(denied), 2)
        self.assertIn("GLIBC_2.39", denied[1])

    def test_macho_exact_system_boundary(self):
        text = "control:\n\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1356.0.0)\n"
        self.assertEqual(guard.forbidden(guard.parse_macho(text)), [])
        for dependency in ["/opt/homebrew/opt/openssl@3/lib/libssl.3.dylib", "@rpath/libother.dylib", "/usr/lib/libSystem.B.dylib.evil", "/System/Library/Frameworks/Unrequested.framework/Unrequested"]:
            with self.subTest(dependency=dependency):
                self.assertEqual(guard.forbidden(guard.parse_macho(text + f"\t{dependency} (compatibility version 1.0.0, current version 1.0.0)\n")), [dependency])

    def test_pe_os_dlls_case_insensitive_and_delay_crt_rejected(self):
        text = PE + "Import {\n  Name: KERNEL32.dll\n}\nImport {\n  Name: ntdll.dll\n}\n"
        self.assertEqual(guard.forbidden(guard.parse_pe(text)), [])
        for dll in ["api-ms-win-crt-runtime-l1-1-0.dll", "MSVCP140.dll", "VCRUNTIME140.dll", "unknown.dll"]:
            with self.subTest(dll=dll):
                self.assertEqual(guard.forbidden(guard.parse_pe(text + f"DelayImport {{\n  Name: {dll}\n}}\n")), [dll])

    def test_malformed_inspection_is_never_empty_success(self):
        for parser, text in [(guard.parse_elf, "not an ELF file"),
                             (guard.parse_elf, "ELF Header:\n  Type: EXEC\n"),
                             (guard.parse_elf, ELF + "  0x1 (NEEDED) unreadable\n"),
                             (guard.parse_elf, ELF + " INTERP 0x0\n"),
                             (guard.parse_macho, ""),
                             (guard.parse_macho, "control:\nmalformed library\n"),
                             (guard.parse_pe, "Format: COFF-x86-64\n"),
                             (guard.parse_pe, PE + "Import {\n  Name: a.dll\n"),
                             (guard.parse_pe, PE + "Import {\n}\n")]:
            with self.subTest(parser=parser.__name__, text=text):
                with self.assertRaises(guard.InspectionError):
                    parser(text)

    def test_inspector_failure_diagnostics_and_empty_output(self):
        for result in [subprocess.CompletedProcess([], 1, "", "bad image"),
                       subprocess.CompletedProcess([], 0, "header", "warning: corrupt"),
                       subprocess.CompletedProcess([], 0, "", "")]:
            with self.subTest(result=result), patch.object(guard.subprocess, "run", return_value=result):
                with self.assertRaises(guard.InspectionError):
                    guard.run_inspector(["inspector", "program"])
        with patch.object(guard.subprocess, "run", side_effect=subprocess.TimeoutExpired("inspector", 60)):
            with self.assertRaises(guard.InspectionError):
                guard.run_inspector(["inspector", "program"])

    def test_missing_tool_is_an_error(self):
        with patch.object(guard, "REPO", Path("/not/a/repository")), patch.object(guard.shutil, "which", return_value=None):
            with self.assertRaises(guard.InspectionError):
                guard.tool("absent-inspector")

    def test_unknown_format_and_failed_inspection_exit_two(self):
        with tempfile.TemporaryDirectory() as d, contextlib.redirect_stderr(io.StringIO()):
            binary = Path(d) / "program"
            binary.write_bytes(b"not a binary")
            self.assertEqual(guard.main(["--artifact", str(binary)]), 2)
            binary.write_bytes(b"\x7fELF")
            with patch.object(guard, "run_inspector", side_effect=guard.InspectionError("tool failed")):
                self.assertEqual(guard.main(["--artifact", str(binary)]), 2)

    def test_inspection_selects_artifact_format_not_host(self):
        with tempfile.TemporaryDirectory() as d, patch.object(guard, "tool", return_value="inspector"):
            binary = Path(d) / "any-name"
            for magic, text, expected in [(b"\x7fELF", ELF, "ELF"), (b"MZ00", PE, "PE"),
                                          (b"\xcf\xfa\xed\xfe", "control:\n", "Mach-O")]:
                with self.subTest(expected=expected), patch.object(guard, "run_inspector", return_value=text):
                    binary.write_bytes(magic)
                    self.assertEqual(guard.inspect(binary)["format"], expected)

    def test_build_isolated_output_and_cleanup(self):
        with tempfile.TemporaryDirectory() as d:
            compiler = Path(d) / "verum"
            compiler.write_text("placeholder")
            compiler.chmod(0o755)
            roots = []
            def build(args, cwd):
                cwd = Path(cwd)
                roots.append(cwd)
                self.assertEqual(args, [str(compiler), "build", str(cwd / "smoke.vr")])
                self.assertTrue((cwd / "smoke.vr").is_file())
                binary = cwd / "target/release" / ("smoke.exe" if os.name == "nt" else "smoke")
                binary.parent.mkdir(parents=True)
                binary.write_bytes(b"\x7fELF")
                return subprocess.CompletedProcess(args, 0)
            with patch.object(guard.subprocess, "run", side_effect=build), patch.object(guard, "check", return_value=0) as inspect:
                for _ in range(2):
                    self.assertEqual(guard.build_and_check(compiler, None), 0)
                self.assertEqual(inspect.call_count, 2)
            self.assertNotEqual(roots[0], roots[1])
            self.assertTrue(all(not p.exists() for p in roots))

    def test_successful_build_without_fresh_output_cannot_reuse_stale_binary(self):
        with tempfile.TemporaryDirectory() as d:
            compiler = Path(d) / "verum"
            compiler.write_text("placeholder")
            compiler.chmod(0o755)
            for code in [0, 7]:
                with self.subTest(code=code), patch.object(guard.subprocess, "run", return_value=subprocess.CompletedProcess([], code)), patch.object(guard, "check") as inspect:
                    with self.assertRaises(guard.InspectionError):
                        guard.build_and_check(compiler, None)
                    inspect.assert_not_called()

    def test_unsupported_host_is_not_a_skip_success(self):
        with patch.object(guard.platform, "system", return_value="UnknownOS"):
            with self.assertRaises(guard.InspectionError):
                guard.build_and_check(Path("unused"), None)

    def test_check_forbidden_import_exit_one(self):
        with tempfile.TemporaryDirectory() as d, contextlib.redirect_stdout(io.StringIO()):
            binary = Path(d) / "control"
            binary.write_bytes(b"program")
            with patch.object(guard, "inspect", return_value=guard.parse_elf(ELF + "  0x1 (NEEDED) Shared library: [libc.so.6]\n")):
                self.assertEqual(guard.check(binary), 1)


if __name__ == "__main__":
    unittest.main()
