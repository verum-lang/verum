#!/usr/bin/env python3
"""The `docs/by-example/` programs must compile.

CLAUDE.md points at this directory beside `grammar/verum.ebnf` for
syntax, and 22 chapters live there as real `.vr` files rather than
fenced blocks. Nothing checked them: `check_doc_examples` walks `core/`
and the website tree, and neither reaches here. Measured 2026-09-05 —
all 22 are clean, which is why this gate opens at ZERO rather than at a
ratchet. A directory that is already green is the cheapest moment to
put a gate on it.

The check is `verum check` per file, and it counts EVERY line beginning
`error`, not `error<E…>`: half of `verum_types`'s diagnostics carry no
code, so a filter anchored to the code form reads a real failure as
silence.
"""
from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
# Two corpora, same rule: a program shipped as an example must
# compile. `docs/by-example` is the tutorial series CLAUDE.md cites
# for syntax; `crates/verum_cli/examples` ships with the CLI and was
# measured at 83 errors across three of its five files on
# 2026-09-05 — nothing referenced them and nothing checked them,
# while `verum audit --bundle` read them and printed the errors
# with no source location attached.
EXAMPLE_DIRS = [REPO / "docs" / "by-example",
                REPO / "examples",
                REPO / "crates" / "verum_cli" / "examples",
                REPO / "crates" / "verum_compiler" / "examples"]


def binary() -> str:
    return os.environ.get("VERUM_BIN") or str(REPO / "target" / "debug" / "verum")


def errors(bin_path: str, f: Path, timeout: int) -> list[str]:
    try:
        p = subprocess.run([bin_path, "check", str(f)],
                           capture_output=True, text=True, timeout=timeout)
    except subprocess.TimeoutExpired:
        return [f"TIMEOUT after {timeout}s"]
    out = (p.stdout + p.stderr).split("\n")
    return [l for l in out if l.lower().startswith("error")]


SELF_TEST = [
    # (compiler output, expected error-line count) — the filter must see a
    # codeless diagnostic, which is the whole point of not using `error<E`.
    ("error<E400>: Type mismatch\n  --> x.vr:1:1", 1),
    ("error: Type inference for expression kind 'copattern body'", 1),
    ("error: compilation failed with 2 errors\nerror<E018>: Parse error", 2),
    ("Checking x.vr\nok", 0),
    # A word merely containing "error" is not a diagnostic line.
    ("terrорs are fine\nchecked 1 file", 0),
]


def self_test() -> int:
    bad = 0
    for out, want in SELF_TEST:
        got = len([l for l in out.split("\n") if l.lower().startswith("error")])
        if got != want:
            bad += 1
            print(f"FAIL {out!r} -> {got}, expected {want}", file=sys.stderr)
    if bad:
        print(f"self-test: {bad} case(s) FAILED", file=sys.stderr)
        return 1
    print(f"self-test: {len(SELF_TEST)} case(s) OK")
    return 0


# A SKIP IS A VERDICT ABOUT NOTHING. In CI these gates run with an
# explicit flag, and a missing input there is not "nothing to check" —
# it is the checkout step having failed, the docs directory having
# moved, or the binary not having been built. Reporting OK in that
# state is the shape `check_gate_verdict_carries_a_quantity` exists to
# prevent, one level up: not a verdict without a number, but a verdict
# without a subject. Six gates could do it; measured 2026-09-06.
#
# Locally, with no flag, skipping stays correct — a source-only clone
# has no website beside it and should not fail for that.
def _skip_or_fail(argv, what: str) -> int:
    """Report a missing input, and decide whether that is fatal.

    Returns the exit code AND says which of the two happened, because a
    line reading "skipped" beside a non-zero exit is a verdict that
    misdescribes itself.
    """
    import os as _os
    import sys as _sys

    fatal = any(a in argv for a in ("--check", "--ratchet")) or bool(
        _os.environ.get("CI")
    )
    if fatal:
        print(
            f"{what} — REFUSING to report OK: this run was asked to CHECK, so a "
            f"missing input is a failed checkout or an unbuilt binary, not "
            f"'nothing to do'.",
            file=_sys.stderr,
        )
        return 2
    print(f"{what}, skipped (local run; pass --check to make this fatal)")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    bin_path = binary()
    if not Path(bin_path).exists():
        return _skip_or_fail(sys.argv, f"check-by-example: no verum binary at {bin_path}")
    missing = [d for d in EXAMPLE_DIRS if not d.is_dir()]
    if missing:
        # NOT a pass. This directory is tracked, so its absence means the
        # checkout is wrong, not that there is nothing to check.
        print(f"check-by-example: FAILED — no examples at {missing}", file=sys.stderr)
        return 1

    timeout = 200
    files = sorted(f for d in EXAMPLE_DIRS for f in d.rglob("*.vr"))
    if not files:
        print("check-by-example: FAILED — the directory holds no .vr files",
              file=sys.stderr)
        return 1

    broken: list[tuple[Path, list[str]]] = []
    for f in files:
        errs = errors(bin_path, f, timeout)
        if errs:
            broken.append((f, errs))

    if broken:
        print(f"[fail] {len(broken)} of {len(files)} by-example program(s) do not compile:")
        for f, errs in broken:
            print(f"    {f.relative_to(REPO)}")
            for e in errs[:3]:
                print(f"        {e[:100]}")
        return 1

    print(f"check-by-example: {len(files)} program(s), 0 that do not compile")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
