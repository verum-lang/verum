#!/usr/bin/env python3
"""
Architectural CI guard: detect host-`#[cfg(target_os)]` regressions in codegen.

Per `docs/architecture/no-libc-architecture.md` and CLAUDE.md:

    When emitting LLVM IR, every per-platform decision (syscall numbers,
    sockaddr layout, errno-fn name, socket-option constants, …) reads
    `module.get_triple()` — the **target** triple — never host
    `#[cfg(target_os = "...")]` directives.  HOST gates miscompile
    cross builds.

This script scans `crates/verum_codegen/src/llvm/` for `#[cfg(target_os = ...)]`
directives and rejects any that aren't on the allow-list.  The allow-list
covers HOST-detection helpers (e.g. `TargetPlatform::current()`) and a few
host-system-info functions; everything else must use
`target_is_linux/darwin/windows/freebsd(module)` from `target_triple.rs`.

Run:
    python3 scripts/ci/check_no_host_cfg.py [--strict]

Exit codes:
    0 — no violations
    1 — violations found (CI must fail)
    2 — script error

Usage in CI: drop into a job step as `python3 scripts/ci/check_no_host_cfg.py`
on a Linux runner.  No deps beyond stdlib.
"""

import os
import re
import sys
from pathlib import Path

# Repo root inferred from script location: scripts/ci/check_no_host_cfg.py
SCRIPT_DIR = Path(__file__).resolve().parent
REPO_ROOT = SCRIPT_DIR.parent.parent

# Target directories to scan.  These are the codegen-internal paths where
# emit decisions MUST use target-triple, not host #[cfg].
SCAN_DIRS = [
    REPO_ROOT / "crates" / "verum_codegen" / "src" / "llvm",
    REPO_ROOT / "crates" / "verum_codegen" / "src" / "mlir",
    REPO_ROOT / "crates" / "verum_compiler" / "src" / "pipeline",
]

# Allow-list: file_relative_path → reason.
# Files in this list are exempt because they're either:
#   (a) HOST-detection helpers that intentionally read the host's compile-
#       time platform (e.g. TargetPlatform::current() for diagnostics)
#   (b) JIT/REPL runtime that EXECUTES on the host (not codegen for the
#       binary being compiled) — host-cfg is correct there
#   (c) Pure-Rust runtime helpers unrelated to AOT codegen
ALLOWED_FILES = {
    # `TargetPlatform::current()` is the canonical "what host am I building
    # ON?" query — used for diagnostics, error messages, runtime defaults.
    # Distinct from the FORBIDDEN pattern which uses host-cfg to decide what
    # IR/code to emit for a given target.
    "crates/verum_codegen/src/llvm/ffi.rs": "TargetPlatform::current host detection",
    # The doc/comment block in target_triple.rs explicitly explains the
    # forbidden pattern in its own comments; the file itself uses none.
    "crates/verum_codegen/src/llvm/target_triple.rs": "documentation comments only",
    # mod.rs has anti-pattern comments only (no actual cfg directives).
    "crates/verum_codegen/src/llvm/mod.rs": "documentation comments only",
    # JIT symbol resolver loads dynamic libraries at RUNTIME on the host
    # executing the JIT — the host-cfg is correct because it queries the
    # filesystem of the machine doing the JIT, not the target the binary
    # is built for.
    "crates/verum_codegen/src/mlir/jit/symbol_resolver.rs": "JIT host library loading",
}

# Detection regex.  Matches:
#   #[cfg(target_os = "...")]
#   #[cfg(any(target_os = "...", target_os = "..."))]
#   #[cfg(not(target_os = "..."))]
#   #[cfg_attr(target_os = "...", ...)]
HOST_CFG_RE = re.compile(
    r"#\[\s*(?:cfg|cfg_attr)\s*\(\s*(?:any|not|all)?\s*\(?[^)]*target_os\s*=",
    re.MULTILINE,
)

# Lines containing only comment text don't count as violations — the codebase
# documents the anti-pattern in its own comments.
COMMENT_LINE_RE = re.compile(r"^\s*(//|\*)")


def scan_file(path: Path) -> list:
    """Return list of (line_no, line_text) violations in this file."""
    rel = path.relative_to(REPO_ROOT).as_posix()
    if rel in ALLOWED_FILES:
        return []
    try:
        content = path.read_text(encoding="utf-8")
    except (UnicodeDecodeError, OSError) as e:
        print(f"[warn] cannot read {rel}: {e}", file=sys.stderr)
        return []

    violations = []
    for line_no, line in enumerate(content.splitlines(), start=1):
        if COMMENT_LINE_RE.match(line):
            continue
        if HOST_CFG_RE.search(line):
            violations.append((line_no, line.strip()))
    return violations


def main() -> int:
    strict = "--strict" in sys.argv

    total_violations = 0
    files_with_violations = 0

    for scan_dir in SCAN_DIRS:
        if not scan_dir.exists():
            continue
        for rs in sorted(scan_dir.rglob("*.rs")):
            v = scan_file(rs)
            if v:
                files_with_violations += 1
                rel = rs.relative_to(REPO_ROOT).as_posix()
                print(f"\n{rel}:")
                for line_no, text in v:
                    print(f"  {line_no:>5}: {text}")
                    total_violations += 1

    if total_violations == 0:
        print("OK: no host-#[cfg(target_os)] violations in codegen.")
        return 0

    print(
        f"\n{'='*78}\n"
        f"FAIL: {total_violations} host-#[cfg(target_os)] violation(s) in {files_with_violations} file(s).\n"
        f"\n"
        f"Per docs/architecture/no-libc-architecture.md:\n"
        f"  Codegen MUST read `module.get_triple()` (the TARGET triple),\n"
        f"  NOT host `#[cfg(target_os = ...)]` directives.\n"
        f"\n"
        f"Use the canonical helpers from `crates/verum_codegen/src/llvm/target_triple.rs`:\n"
        f"  target_is_linux(module)\n"
        f"  target_is_darwin(module)\n"
        f"  target_is_windows(module)\n"
        f"  target_is_freebsd(module)\n"
        f"  target_is_aarch64(module)\n"
        f"  target_is_x86_64(module)\n"
        f"\n"
        f"Host #[cfg] gates miscompile cross builds — a binary built on macOS for\n"
        f"a Linux target would emit Darwin-shaped sockaddr, errno location, etc.\n"
        f"{'='*78}",
        file=sys.stderr,
    )

    return 1 if strict or total_violations > 0 else 0


if __name__ == "__main__":
    sys.exit(main())
