#!/usr/bin/env python3
"""Gate: a table inside a gate must not have the same key twice.

2026-09-03: two sessions recorded `phase_context_validation` in the same
dict literal in `check_entry_points_agree_on_their_phases.py`, minutes
apart. Python keeps the last and drops the first without a word. One of
the two explanations became dead text that still READS as live — and
worse, an edit to it would change nothing while looking like a change.

Nothing catches this: not the interpreter, not a linter that is not
run here, not review, and not the gate itself, whose behaviour is
unchanged by the duplicate because the surviving row happened to say
the same thing.

The lesson generalises past that one file. Gates in this directory are
mostly a table plus a loop, the table is the part humans edit, and the
duplicate is invisible exactly where the record is meant to be
authoritative. So: every dict and set literal in every gate here is
parsed with `ast`, and a repeated constant key fails.

WHY `ast` AND NOT A REGEX: a key can be written on any line, in any
quoting style, with a comment between it and its value. The parser is
the thing that decides what a duplicate is, so the parser is what this
asks. Non-constant keys (a variable, an f-string) are skipped — they
are not comparable without evaluating them, and a gate that guesses is
worse than one that says what it covers.
"""

from __future__ import annotations

import argparse
import ast
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
GATES = REPO / "scripts" / "ci"


def literal_key(node: ast.expr):
    """The comparable value of a key node, or None if not a constant."""
    if isinstance(node, ast.Constant):
        return (type(node.value).__name__, node.value)
    if isinstance(node, ast.Tuple):
        parts = [literal_key(e) for e in node.elts]
        return ("tuple", tuple(parts)) if all(p is not None for p in parts) else None
    return None


def duplicates(path: Path) -> list[tuple[int, str]]:
    try:
        tree = ast.parse(path.read_text(errors="ignore"), filename=str(path))
    except SyntaxError as e:
        return [(e.lineno or 0, f"could not parse: {e.msg}")]
    found: list[tuple[int, str]] = []
    for node in ast.walk(tree):
        if isinstance(node, ast.Dict):
            keys = [(k, literal_key(k)) for k in node.keys if k is not None]
        elif isinstance(node, ast.Set):
            keys = [(k, literal_key(k)) for k in node.elts]
        else:
            continue
        seen: dict = {}
        for knode, kval in keys:
            if kval is None:
                continue
            if kval in seen:
                found.append((
                    getattr(knode, "lineno", 0),
                    f"{kval[1]!r} appears twice (first at line {seen[kval]}) — "
                    "the earlier one is silently discarded",
                ))
            else:
                seen[kval] = getattr(knode, "lineno", 0)
    return found


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", type=Path, default=GATES)
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument("--check", action="store_true")
    args = ap.parse_args()

    if args.self_test:
        ok = True
        probe = Path("/tmp/zzq_dupkey_probe.py")
        # A file WITH a duplicate must be caught…
        probe.write_text('D = {\n "a": 1,\n "b": 2,\n "a": 3,\n}\n')
        hits = duplicates(probe)
        if len(hits) != 1 or "'a'" not in hits[0][1]:
            print(f"self-test FAIL: duplicate not caught ({hits})")
            ok = False
        # …and one without must not be.
        probe.write_text('D = {\n "a": 1,\n "b": 2,\n}\nS = {"x", "y"}\n')
        if duplicates(probe):
            print("self-test FAIL: clean file reported a duplicate")
            ok = False
        # A non-constant key must be skipped, not guessed at.
        probe.write_text('k = "a"\nD = {k: 1, k: 2}\n')
        if duplicates(probe):
            print("self-test FAIL: a variable key was treated as comparable")
            ok = False
        # Tuple keys are comparable and must be caught.
        probe.write_text('D = {("a", 1): 0, ("a", 1): 2}\n')
        if len(duplicates(probe)) != 1:
            print("self-test FAIL: a duplicate tuple key was missed")
            ok = False
        probe.unlink()
        print("self-test: ok" if ok else "self-test: FAILED")
        return 0 if ok else 1

    files = sorted(args.dir.glob("*.py"))
    if not files:
        print(f"check-gate-tables: no gate scripts under {args.dir} — "
              "refusing to pass vacuously", file=sys.stderr)
        return 1
    bad = 0
    for f in files:
        for line, msg in duplicates(f):
            try:
                shown = f.relative_to(REPO)
            except ValueError:
                shown = f  # --dir may point outside the repo (a probe)
            print(f"  {shown}:{line}: {msg}", file=sys.stderr)
            bad += 1
    print(f"check-gate-tables: {len(files)} gate scripts, {bad} duplicate keys")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
