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
# ZERO, and it took getting there to see why a ratchet was the wrong
# shape. The baseline was 16 with 15 rows failing, so exactly one new
# violation fitted in the headroom — and one did: A78, written the day
# the ratchet was set, carried five stray `|` and the gate reported
# "at the baseline, not growing". A ratchet with slack admits its next
# instance silently.
#
# All sixteen are now fixed (2026-09-05), so the slack is gone:
#   - the header `| # | Item | Pri | Anchor | Acceptance |` is the
#     authority for five columns; six rows had two and got `—` for the
#     three the prose never stated, four had a sixth cell that markdown
#     drops, and thirty pipes inside inline code are now escaped `\|`;
#   - A59 was one 80KB line that had swallowed a section heading, ten
#     `A-*` rows and the whole nine-row B table — 21 rows in all had
#     stopped being rows. They are rows again.
#
# What the check is FOR is the case that produced it: A64 growing from
# 6 pipes to 8 under an edit of mine, rendering the priority column as
# prose while every other check stayed green.
WIDTH_BASELINE = 0


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
        # Count SEPARATORS, not pipe characters: markdown reads `\|`
        # as a literal pipe inside a cell, so an author who escapes a
        # stray pipe has already fixed the row. Measured 2026-09-05 on
        # A78 — five correctly escaped pipes still read as five extra
        # columns here, so the gate asked for a fix it would not accept.
        widths[family].append((label, i, line.replace("\\|", "").count("|")))

    out: list[tuple[str, int, int, int]] = []
    for family, rows in widths.items():
        if len(rows) < 5:
            continue
        modal = Counter(w for _, _, w in rows).most_common(1)[0][0]
        for label, ln, w in rows:
            if w != modal:
                out.append((label, ln, w, modal))
    return out


BURIED = re.compile(r"\|\s*\|?\s*([A-Z][A-Za-z0-9-]*)\s*\|\s*(?:\*\*|~~)")


def buried_rows(lines: list[str]) -> list[tuple[str, int, str]]:
    """Rows that have swallowed another row's label mid-line.

    A row swallowed this way is invisible to every reader that walks
    rows, and its own row still looks well-formed. Measured 2026-09-05:
    21 rows had stopped being rows — `A5` inside `A3` and `B6` inside
    `B1d` for months, and ten `A-*` rows plus the whole nine-row B table
    inside a single 80KB `A59` line, where a repair that rejoined
    multi-line rows with ` — ` had run past the end of the row it was
    repairing. That repair's control was a word count: 38157 words
    before, 38157 after. It could not see the loss, because no word was
    lost — only the structure was. Counting ROWS is the control that
    sees it, so this check counts rows.
    """
    out = []
    for i, line in enumerate(lines, 1):
        if not ROW.match(line):
            continue
        own = line.split("|")[1].strip()
        for m in BURIED.finditer(line):
            if m.start() > 2:
                out.append((own, i, m.group(1)))
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

# The width check needs its own poles: `broken_rows` above cannot see a
# stray `|`, which is the whole reason `wrong_width` exists. Measured
# 2026-09-04 — inverting a pole in SELF_TEST left this function
# untouched and the gate green, so for ten minutes it had no control at
# all. Five members minimum, because a mode needs something to be modal
# about.
WIDTH_SELF_TEST = [
    # (lines, expected number of wrong-width rows)
    ([f"| A{i} | a | b | c | d |" for i in range(1, 7)], 0),
    ([f"| A{i} | a | b | c | d |" for i in range(1, 6)]
     + ["| A9 | a | b | c | d | e |"], 1),
    ([f"| A{i} | a | b |" for i in range(1, 6)]
     + ["| A9 | a |"], 1),
    (["| A1 | a | b |", "| A2 | a | b |"], 0),   # family too small to judge
    # An ESCAPED pipe is a literal, not a separator — the row is correct.
    ([f"| A{i} | a | b | c | d |" for i in range(1, 6)]
     + [r"| A9 | a \| z | b | c | d |"], 0),
    # …and an UNESCAPED stray pipe is still caught. Both poles are
    # required: without the second, the escape rule could be widened to
    # "ignore pipes" and the check would pass everything.
    ([f"| A{i} | a | b | c | d |" for i in range(1, 6)]
     + ["| A9 | a | z | b | c | d |"], 1),
]


BURIED_SELF_TEST = [
    # A well-formed pair of rows buries nothing.
    (["| A1 | **finding** | P1 | a | open |",
      "| A2 | **finding** | P1 | a | open |"], 0),
    # A row that swallowed the next one is caught even though it is
    # still one line and still ends with `|` — the two checks above
    # both pass on it.
    (["| A1 | **finding** | P1 | a | open || A2 | **swallowed** | P1 | a | open |"], 1),
    # A row that merely CITES another row's label is not a burial: the
    # label must be in row position, `| Label | **`. Without this pole
    # the check would fire on every cross-reference in the register.
    (["| A1 | **finding**, see A2 and `| A2 |` above | P1 | a | open |"], 0),
]


def self_test() -> int:
    bad = 0
    for lines, want in BURIED_SELF_TEST:
        got = len(buried_rows(lines))
        if got != want:
            bad += 1
            print(f"FAIL buried {lines!r} -> {got}, expected {want}", file=sys.stderr)
    for lines, want in WIDTH_SELF_TEST:
        got = len(wrong_width(lines))
        if got != want:
            bad += 1
            print(f"FAIL width {lines!r} -> {got}, expected {want}", file=sys.stderr)
    for lines, want in SELF_TEST:
        got = len(broken_rows(lines))
        if got != want:
            bad += 1
            print(f"FAIL {lines!r} -> {got}, expected {want}", file=sys.stderr)
    if bad:
        print(f"self-test: {bad} of {len(SELF_TEST)} case(s) FAILED", file=sys.stderr)
        return 1
    print(f"[ok] self-test: {len(SELF_TEST) + len(WIDTH_SELF_TEST)} case(s) hold")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    lines = REGISTER.read_text().splitlines()
    bad = broken_rows(lines)
    total = sum(1 for l in lines if ROW.match(l))
    widths = wrong_width(lines)
    buried = buried_rows(lines)
    if buried:
        print(f"[fail] {len(buried)} register row(s) are buried inside another row:")
        for owner, ln, victim in buried:
            print(f"    {victim}  swallowed by {owner}  at line {ln}")
        print(
            "\nA row that swallowed the next one is still one line and still ends\n"
            "with `|`, so neither check below sees it — but the swallowed row is\n"
            "gone from every reader that walks rows.  Put it back on its own line."
        )
        return 1
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
