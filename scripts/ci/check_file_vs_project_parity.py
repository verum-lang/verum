#!/usr/bin/env python3
"""Gate: one source, one verdict — `verum check FILE` vs `verum check` in a project.

The same file gets two answers depending on which command runs. Measured
instances, each a separate pool row:

  * a stdlib CONTEXT resolves in a single file and is `E605: undefined
    context` inside a project (T0974) — the project path received the
    metadata and skipped the registration the single-file path does;
  * variant constructors did the same until T0865 — bare `Ok(x)` was
    "not a function: Ok" in a project, 110 of 334 registry errors;
  * 535 of 2 561 `core/` files fail `verum check` while the bake, which
    compiles the same sources, is green (T0755).

Three symptoms, one shape: two drivers over one language, and a step
present in one of them.

This gate writes a handful of small programs, checks each BOTH ways, and
fails when the two verdicts differ. It is deliberately tiny — the point
is the comparison, not coverage.

    scripts/ci/check_file_vs_project_parity.py [--verum PATH] [--keep]
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]

# (name, source). Each must be a complete program.
CASES: list[tuple[str, str]] = [
    (
        "stdlib_context",
        """mount core.context.standard.{Logger};

fn emit() using [Logger] {
    Logger.info("hello");
}

fn main() {
    print("ok");
}
""",
    ),
    (
        "variant_ctor",
        """fn wrap(n: Int) -> Maybe<Int> {
    Some(n)
}

fn main() {
    match wrap(1) {
        Some(v) => print(f"got {v}"),
        None    => print("none"),
    }
}
""",
    ),
    (
        "refinement_in_signature",
        """type Pos is n: Int where n > 0;

fn bump(p: Pos) -> Int {
    p + 1
}

fn main() {
    print(f"{bump(1)}");
}
""",
    ),
]

MANIFEST = """[cog]
name = "parity_probe"
version = "0.1.0"
edition = "2026"
"""


def find_verum(explicit: str | None) -> Path | None:
    if explicit:
        p = Path(explicit)
        return p if p.is_file() else None
    for rel in ("target/release/verum", "target/debug/verum"):
        p = REPO / rel
        if p.is_file():
            return p
    return None


def run(verum: Path, args: list[str], cwd: Path) -> tuple[int, str]:
    proc = subprocess.run(
        [str(verum), *args],
        cwd=cwd,
        capture_output=True,
        text=True,
        timeout=300,
    )
    return proc.returncode, (proc.stdout + proc.stderr)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--verum")
    ap.add_argument("--keep", action="store_true", help="leave the temp tree in place")
    args = ap.parse_args()

    verum = find_verum(args.verum)
    if verum is None:
        print(
            "no verum binary at target/{release,debug}/verum — "
            "pass one with --verum PATH. NOT CHECKED.",
            file=sys.stderr,
        )
        return 0

    root = Path(tempfile.mkdtemp(prefix="verum-parity-"))
    mismatches: list[str] = []
    checked = 0
    try:
        for name, source in CASES:
            # Each case gets its OWN project: several files declaring
            # `main` in one project currently run an arbitrary one of
            # them (T0979), which would confound this comparison.
            proj = root / name
            (proj / "src").mkdir(parents=True)
            (proj / "verum.toml").write_text(MANIFEST)
            src = proj / "src" / "main.vr"
            src.write_text(source)

            file_rc, file_out = run(verum, ["check", str(src)], root)
            proj_rc, proj_out = run(verum, ["check"], proj)
            checked += 1

            if (file_rc == 0) != (proj_rc == 0):
                worse = "project" if proj_rc != 0 else "file"
                detail = (proj_out if worse == "project" else file_out).strip()
                first = next(
                    (ln for ln in detail.splitlines() if "error" in ln), detail[:120]
                )
                mismatches.append(
                    f"  {name}: file rc={file_rc}, project rc={proj_rc} "
                    f"— {worse} rejects\n      {first.strip()[:120]}"
                )
    finally:
        if not args.keep:
            shutil.rmtree(root, ignore_errors=True)

    # Positive control: with no case actually run, "no mismatches" would
    # be meaningless.
    if checked != len(CASES):
        print(
            f"REFUSING TO PASS: ran {checked} of {len(CASES)} cases",
            file=sys.stderr,
        )
        return 2

    print(f"file-vs-project parity: {checked} programs checked both ways")
    if not mismatches:
        print("one source, one verdict")
        return 0
    print(f"\n{len(mismatches)} program(s) get DIFFERENT verdicts:\n", file=sys.stderr)
    for m in mismatches:
        print(m, file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
