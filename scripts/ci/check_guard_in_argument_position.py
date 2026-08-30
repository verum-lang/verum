#!/usr/bin/env python3
"""Gate: no lock guard in argument position (T0981).

    f(…, &registry.read(), …)

reads as "give me a snapshot" and means "hold the lock for the whole
call" — the temporary guard lives to the end of the statement. When the
callee takes a WRITE lock on the same lock, that is a self-deadlock on
one thread, and `parking_lot`'s RwLock is not reentrant.

It has cost this project twice. The first instance was found, fixed and
documented at length in `phases_orchestration.rs`, with a regression
test naming the exact file that reproduces it. The siblings were not
checked: `compile_orchestration.rs` deadlocked `verum check` on any
multi-file project whose imports reach the lazy loader — 25 minutes of
wall clock for 8.9 s of CPU — and `stdlib_bootstrap.rs` carried the same
call on the BAKE path, where a hang reads as "the bake is just long"
and nobody would think to sample it.

One `grep` after the first repair would have found both. This is that
grep, kept.

WHAT COUNTS. A guard passed by reference into a call:

    ok:   let snap = registry.read().clone();  f(…, &snap, …);
    bad:  f(…, &registry.read(), …)

The safe form clones and releases; clone PER ITERATION when the callee
can register something a later iteration must see.

    scripts/ci/check_guard_in_argument_position.py [--check]
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]

# `&<expr>.read()` or `&<expr>.write()` NOT followed by a method call —
# `&x.read().clone()` is the safe form and must not match.
GUARD_ARG = re.compile(r"&\s*[\w.]+\s*\.\s*(read|write|lock)\s*\(\s*\)\s*(?![.\w])")

SKIP_DIRS = {"target", ".git", "tests", "benches", "examples"}

# Known occurrences, each CHECKED: the callee does not lock the same
# object, so the held guard cannot deadlock today. They stay listed
# rather than deleted because the form is still a loaded gun — adding a
# lock inside any of these callees turns it into the compiler's bug.
#
# Verified 2026-08-30: `resolve_symbol` and `validate_new_name` take no
# lock at all; `document.rs:776` hands the guard to a caller-supplied
# closure, which is the shape to watch — the closure is not ours.
KNOWN: set[tuple[str, int]] = {
    ("crates/verum_lsp/src/rename.rs", 528),
    ("crates/verum_lsp/src/rename.rs", 532),
    ("crates/verum_lsp/src/document.rs", 776),
}


def is_comment(line: str) -> bool:
    return line.lstrip().startswith("//")


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true")
    args = ap.parse_args()

    hits: list[tuple[str, int, str]] = []
    known_seen: set[tuple[str, int]] = set()
    scanned = 0
    for path in (REPO / "crates").rglob("*.rs"):
        if any(part in SKIP_DIRS for part in path.parts):
            continue
        try:
            lines = path.read_text(errors="replace").splitlines()
        except OSError:
            continue
        scanned += 1
        for n, line in enumerate(lines, 1):
            if is_comment(line):
                continue
            if not GUARD_ARG.search(line):
                continue
            # Only argument position: the match sits inside a call's
            # parentheses, i.e. the line ends with `,` or `)` after it,
            # or the guard is followed by `,`.
            if re.search(r"&\s*[\w.]+\s*\.\s*(read|write|lock)\s*\(\s*\)\s*[,)]", line):
                rel = str(path.relative_to(REPO))
                if (rel, n) in KNOWN:
                    known_seen.add((rel, n))
                    continue
                hits.append((rel, n, line.strip()[:100]))

    # Positive control: an empty scan would make "no hits" meaningless.
    if scanned < 50:
        print(
            f"REFUSING TO PASS: scanned only {scanned} Rust files",
            file=sys.stderr,
        )
        return 2

    stale = KNOWN - known_seen
    print(
        f"guard-in-argument-position: {scanned} files scanned, "
        f"{len(known_seen)} of {len(KNOWN)} known occurrences still present"
    )
    if stale:
        # A known entry that no longer matches means the line moved or was
        # fixed. Left unnoticed, the list rots into a blanket exemption.
        print(
            "\nknown entries that no longer match (update the list):",
            file=sys.stderr,
        )
        for rel, n in sorted(stale):
            print(f"  {rel}:{n}", file=sys.stderr)
    if not hits:
        print("no NEW lock guard is passed directly into a call")
        return 1 if (stale and args.check) else 0

    print(f"\n{len(hits)} guard(s) in argument position:\n", file=sys.stderr)
    for rel, n, text in hits:
        print(f"  {rel}:{n}\n      {text}", file=sys.stderr)
    print(
        "\nA temporary guard lives to the end of the statement, i.e. for the\n"
        "whole call. If the callee locks the same object, that is a\n"
        "self-deadlock. Bind a clone instead:\n"
        "    let snap = the_lock.read().clone();\n"
        "    f(…, &snap, …);\n"
        "and clone per iteration when a later one must see what an earlier\n"
        "one registered.",
        file=sys.stderr,
    )
    return 1 if args.check else 0


if __name__ == "__main__":
    raise SystemExit(main())
