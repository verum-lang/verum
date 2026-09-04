#!/usr/bin/env python3
"""Every `| A<n> |` row in the tech-debt register must be ONE line ending in `|`.

WHY THIS EXISTS.  A markdown table row cannot contain a newline.  Writing a
register entry with real paragraph breaks — the natural thing to do when the
entry is long — silently ends the table: everything after the first break
renders as loose paragraphs, and the rows below it start a new table with no
header.

Measured 2026-09-04: three rows were broken this way in a single session
(A59 spanning 277 lines, A61 78, A55 seven) and none of it was visible in
any diff, because each individual edit was a correct-looking paragraph.  It
was found by measuring a row's length, not by reading the file.

The register is 266KB of one-line rows.  The check is one pass and the
failure it prevents is invisible to review.
"""
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
REGISTER = REPO / "docs" / "architecture" / "tech-debt-register.md"
ROW = re.compile(r"^\| [A-Z]+\d+ \|")


def broken_rows(lines: list[str]) -> list[tuple[str, int]]:
    """Rows that do not terminate on their own line."""
    out = []
    for i, line in enumerate(lines):
        if ROW.match(line) and not line.rstrip().endswith("|"):
            label = line.split("|")[1].strip()
            out.append((label, i + 1))
    return out


SELF_TEST = [
    # (lines, expected number of broken rows)
    (["| A1 | text |", "| A2 | more |"], 0),
    (["| A1 | text", "", "continued |"], 1),
    (["| A1 | text |", "| A2 | broken", "tail |"], 1),
    (["| B7 | other section |"], 0),
    (["| A1 | trailing space |   "], 0),
]


def self_test() -> int:
    bad = 0
    for lines, want in SELF_TEST:
        got = len(broken_rows(lines))
        if got != want:
            bad += 1
            print(f"FAIL {lines!r} -> {got}, expected {want}", file=sys.stderr)
    if bad:
        print(f"self-test: {bad} of {len(SELF_TEST)} case(s) FAILED", file=sys.stderr)
        return 1
    print(f"[ok] self-test: {len(SELF_TEST)} case(s) hold")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    lines = REGISTER.read_text().splitlines()
    bad = broken_rows(lines)
    total = sum(1 for l in lines if ROW.match(l))
    if bad:
        print(f"[fail] {len(bad)} register row(s) span more than one line:")
        for label, ln in bad:
            print(f"    {label}  at line {ln}")
        print(
            "\nA markdown table row cannot contain a newline: everything after\n"
            "the break renders as loose paragraphs and ends the table.  Join the\n"
            "row onto one line — paragraph breaks inside a cell read fine as ` — `."
        )
        return 1
    print(f"[ok] register rows: {total} row(s), each one line")
    return 0


if __name__ == "__main__":
    sys.exit(main())
