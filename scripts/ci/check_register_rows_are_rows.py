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


# Rows whose cell count already differs from their family's mode. A
# RATCHET, not zero, and the distinction is load-bearing: five of the
# sixteen carry one stray `|`, five are a two-column shape that may be
# deliberate, one (A59) embeds whole tables and has 158, and one is
# another session's row from an hour ago. Fixing them is separate work
# on rows this session did not write. What the ratchet is FOR is the
# case that produced it — A64 growing from 6 pipes to 8 under my own
# edits, rendering the priority column as prose, while every other
# check stayed green.
WIDTH_BASELINE = 16


def wrong_width(lines: list[str]) -> list[tuple[str, int, int, int]]:
    """Rows whose cell count differs from the rest of their own family.

    A row is one line (the check above) and still wrong if it carries a
    stray `|`: markdown reads it as another cell and every field after it
    shifts. Measured 2026-09-04 — A64 went from 6 pipes to 8 across three
    revisions because two edits put `None(Unit) | Some(&&_)` and
    `Nothing | Just(T)` inside inline code. Rendered, the priority column
    showed prose and the status column showed the priority. The line-ness
    check saw nothing: the row was still one line.

    The comparison is against the row's OWN PREFIX family (all `A…`
    rows, all `E…` rows), not against a header. A header would be the
    better authority and is not available here: this register interleaves
    nine tables, the A-rows are not contiguous, and the nearest separator
    above a given row belongs to whichever table was inserted last.

    Families with fewer than five members are skipped — a mode needs
    something to be modal about.
    """
    from collections import Counter, defaultdict

    widths: dict[str, list[tuple[str, int, int]]] = defaultdict(list)
    for i, line in enumerate(lines, 1):
        if not ROW.match(line):
            continue
        label = line.split("|")[1].strip()
        family = re.match(r"([A-Z]+)", label).group(1)
        widths[family].append((label, i, line.count("|")))

    out: list[tuple[str, int, int, int]] = []
    for family, rows in widths.items():
        if len(rows) < 5:
            continue
        modal = Counter(w for _, _, w in rows).most_common(1)[0][0]
        for label, ln, w in rows:
            if w != modal:
                out.append((label, ln, w, modal))
    return out


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
    widths = wrong_width(lines)
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
    if len(widths) > WIDTH_BASELINE:
        print(f"[fail] {len(widths)} register row(s) have the wrong cell count, "
              f"baseline {WIDTH_BASELINE}:")
        for label, ln, w, want in widths:
            print(f"    {label}  at line {ln}: {w} pipes, its family's other rows have {want}")
        print(
            "\nA stray `|` inside a cell — including inside inline code, where it\n"
            "reads as a sum type — opens another cell and shifts every field after\n"
            "it.  The row is still ONE LINE, so the check above cannot see it.\n"
            "Write it as `or`, or escape it `\\|`."
        )
        return 1
    if widths:
        print(f"[note] {len(widths)} row(s) render with shifted columns, "
              f"at the baseline of {WIDTH_BASELINE} — not growing")
    print(f"[ok] register rows: {total} row(s), each one line")
    return 0


if __name__ == "__main__":
    sys.exit(main())
