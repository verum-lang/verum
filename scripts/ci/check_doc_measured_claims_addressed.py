#!/usr/bin/env python3
"""A box that says MEASURED must say how to measure it again.

WHY, AND IT COST THREE CORRECTIONS IN ONE DAY
---------------------------------------------
Three admonitions were rewritten on 2026-09-10 because their BASIS had
moved while their verdict still held:

    "compiles and returns an empty trace"   refuted by a compiler run
    "the module does not type-check"        refuted: the bake emits two
                                            stub lines and neither is
                                            from that file
    "verum check reports E400 on
     @const_slot_for"                       the call was DELETED that
                                            morning; three mentions
                                            remain, all in a doc comment

All three rested on "we ran the compiler over a file in the tree" — an
evidence shape with no address. The file is edited and the box learns
last. The boxes that survived the same day rested on something a reader
can execute unchanged:

    strings runtime.vbca | grep -c frame_address     ->  0
    grep -rc '"@embed"' crates/ --include='*.rs'     ->  0
    a program printed IN the box, with its output beneath it

    THE EVIDENCE MUST CARRY ITS OWN INPUT.

A code fence does that: the program travels with the claim, so the claim
cannot be orphaned by an edit somewhere else. A command does it too, as
long as it is written out rather than described.

WHAT COUNTS AS AN ADDRESS
    a fenced block, or an inline / leading `verum`, `grep`, `strings`,
    `cargo`, `make`, `python3`. Deliberately generous: this gate is not
    judging whether the evidence is GOOD, only refusing a measured claim
    with nothing at all to run.

THE ROSTER IS NOT A COUNT. Thirty-five boxes predate the rule; a swap
of two would leave a count unmoved, so they are listed by (page, title)
and both a NEW one and a VANISHED one fail. Removing a row is how the
number goes down, and that requires giving the box its address.

A TITLE IS NOT UNIQUE ON A PAGE. `reference/meta-functions.md` carries
two boxes both titled "Not yet callable" — one measuring the sigil, one
pointing at it. Rows are compared as SETS, so under a title-only key
those two are one row: address either and the set does not move, the
gate stays green, and the surviving unaddressed claim is unwatched.
Measured on a two-box fixture, 2 boxes gave 1 row. A repeated title
therefore takes a body digest, and the digest rather than an occurrence
index because a box inserted above renumbers the rest — one NEW and one
GONE reported for an edit that addressed nothing. The self-test probes
the key of ONE box across three neighbourhoods; comparing sets cannot
see the renumbering, because the inserted box fills the key its
neighbour vacated and the old set stays a subset of the new.

Some are legitimately un-addressable — `architecture/module-system.md`'s
"What this rule does not cover" makes a claim about scope, not about
behaviour, and there is nothing to point at. Those stay on the roster
with the rest rather than in a second exemption list: one roster, and
its shrinking is the measure of the campaign.
"""

from __future__ import annotations

import collections
import hashlib
import json
import os
import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
_DOCS_ROOT = os.environ.get("VERUM_DOCS_DIR")
DOCS = pathlib.Path(_DOCS_ROOT) if _DOCS_ROOT else REPO.parent / "website" / "docs"
ROSTER = pathlib.Path(__file__).resolve().parent / "doc_measured_claims_unaddressed.json"

BOX = re.compile(r":::(caution|warning|danger|note|info|tip)([^\n]*)\n(.*?)\n:::", re.S)
MEASURED = re.compile(r"\bmeasured\b|\bre-measured\b", re.I)
ADDRESS = re.compile(
    r"```|`(?:verum|grep|strings|cargo|make|python3)\b|"
    r"^\s*(?:verum|grep|strings|cargo|make)\s",
    re.M,
)
FLOOR = 40


def digest(body: str) -> str:
    return hashlib.blake2s(body.encode()).hexdigest()[:8]


def key_of(title: str, body: str) -> str:
    """A stable name for the box. Falls back to a body digest when the
    admonition carries no title — one on the site does."""
    return (title.strip() or "h:" + digest(body))[:70]


def page_rows(rel: str, text: str) -> tuple[int, int, list[list[str]]]:
    """The measured boxes of ONE page, keyed uniquely within it."""
    boxes = [(m.group(2), m.group(3)) for m in BOX.finditer(text)
             if MEASURED.search(m.group(3))]
    dup = collections.Counter(key_of(t, b) for t, b in boxes)

    measured = addressed = 0
    naked: list[list[str]] = []
    for title, body in boxes:
        measured += 1
        base = key_of(title, body)
        # A page may carry the SAME title twice — `meta-functions.md` has
        # two "Not yet callable" boxes, the second pointing at the first
        # for its evidence. `have` and `want` are SETS, so one title on
        # one page is ONE row however many boxes wear it: address one of
        # a pair and the set does not move, the gate stays green, and the
        # other unaddressed claim is invisible. Measured on a two-box
        # fixture: 2 boxes -> 1 row.
        #
        # The suffix is the BODY, not the position. An occurrence index
        # renumbers when a box is inserted above, which reports one NEW
        # and one GONE where nothing was addressed — a false accusation,
        # and those cost more than a miss. A digest moves only when the
        # claim's own text moves, which is when it should be re-read.
        key = base if dup[base] == 1 else f"{base} @{digest(body)}"
        if ADDRESS.search(body):
            addressed += 1
        else:
            naked.append([rel, key])
    return measured, addressed, naked


def scan(root: pathlib.Path) -> tuple[int, int, list[list[str]]]:
    measured = addressed = 0
    naked: list[list[str]] = []
    for p in sorted(root.rglob("*.md")):
        pm, pa, pn = page_rows(str(p.relative_to(root)),
                               p.read_text(errors="replace"))
        measured += pm
        addressed += pa
        naked.extend(pn)
    return measured, addressed, naked


def self_test() -> int:
    bad = 0
    cases = [
        ("fenced block counts",
         ":::caution T\nMeasured 2026-09-10.\n```\nverum check x\n```\n:::\n", True),
        ("inline command counts",
         ":::caution T\nMeasured: `grep -c foo bar` gives 0.\n:::\n", True),
        ("prose alone does not",
         ":::caution T\nMeasured 2026-09-10: it does not work.\n:::\n", False),
        ("an unmeasured box is not judged",
         ":::note T\nThis section is a design.\n:::\n", None),
    ]
    for label, src, want in cases:
        m = BOX.search(src)
        if not m:
            print(f"self-test: {label}: the box did not parse", file=sys.stderr)
            bad += 1
            continue
        is_measured = bool(MEASURED.search(m.group(3)))
        if want is None:
            if is_measured:
                print(f"self-test: {label}: judged a box making no claim",
                      file=sys.stderr)
                bad += 1
            continue
        if not is_measured:
            print(f"self-test: {label}: the claim was not seen", file=sys.stderr)
            bad += 1
            continue
        if bool(ADDRESS.search(m.group(3))) != want:
            print(f"self-test: {label}: address detection wrong", file=sys.stderr)
            bad += 1

    if key_of("", "abc") == key_of("", "abd"):
        print("self-test: the untitled-box digest does not distinguish bodies",
              file=sys.stderr)
        bad += 1

    # THE ANCHOR. Two boxes, one title, one page — the shape that
    # `reference/meta-functions.md` actually carries. Keyed on the title
    # alone they are ONE row in a set, and addressing either leaves the
    # roster unmoved while an unaddressed claim goes unwatched.
    a = ":::caution Not yet callable\nMeasured: the first fails.\n:::\n"
    b = ":::caution Not yet callable\nMeasured: the second fails too.\n:::\n"
    a_fixed = (":::caution Not yet callable\nMeasured: the first fails.\n"
               "```\nverum check x\n```\n:::\n")
    third = ":::caution Not yet callable\nMeasured: a third, inserted.\n:::\n"

    pair = page_rows("p.md", a + b)[2]
    if len({tuple(r) for r in pair}) != 2:
        print(f"self-test: two same-titled boxes collapsed into "
              f"{len({tuple(r) for r in pair})} row(s): {pair}", file=sys.stderr)
        bad += 1

    # THE KEY OF A BOX MUST NOT DEPEND ON ITS NEIGHBOURS. Comparing SETS
    # cannot see this: renumber the rows and a third box fills the key
    # the second vacated, so the old set is still a subset of the new one
    # and a subset test reports no change. Each variant below is
    # therefore probed for the row belonging to `b` ITSELF, by position.
    #
    # This is what rules out an occurrence index. Under one, `b` is #2
    # beside `a` and #3 once a box is inserted above — one NEW and one
    # GONE reported for an edit that addressed nothing, and a false
    # accusation costs more than a miss.
    for label, text, idx in (("a box inserted above", third + a + b, 2),
                             ("the neighbour addressed", a_fixed + b, 0)):
        rows = page_rows("p.md", text)[2]
        if len(rows) <= idx or tuple(rows[idx]) != tuple(pair[1]):
            print(f"self-test: {label} changed the key of an untouched box: "
                  f"{rows[idx] if len(rows) > idx else None} was {pair[1]}",
                  file=sys.stderr)
            bad += 1

    if not ROSTER.is_file():
        print(f"self-test: {ROSTER.name} is missing — the roster IS the "
              f"baseline, and without it this gate cannot fail",
              file=sys.stderr)
        bad += 1

    if bad:
        print(f"self-test: {bad} FAILED", file=sys.stderr)
        return 1
    rows = json.loads(ROSTER.read_text()) if ROSTER.is_file() else []
    print(f"[ok] self-test: {len(cases)} address cases, 1 digest case, "
          f"3 duplicate-title cases, "
          f"{len(rows)} row(s) on the roster")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    if not DOCS.is_dir():
        print(f"check-doc-measured-claims: no website at {DOCS} — REFUSING to "
              f"report OK. A gate whose INPUT is missing is a failed "
              f"checkout, not 'nothing to do'; set VERUM_DOCS_DIR.",
              file=sys.stderr)
        return 2
    if not ROSTER.is_file():
        print(f"check-doc-measured-claims: {ROSTER} absent; the roster is the "
              f"baseline and this gate is inert without it", file=sys.stderr)
        return 2

    measured, addressed, naked = scan(DOCS)
    want = {tuple(r) for r in json.loads(ROSTER.read_text())}
    have = {tuple(r) for r in naked}

    print(f"check-doc-measured-claims: {measured} box(es) make a measured "
          f"claim, {addressed} carry something to run, {len(have)} do not "
          f"({len(want)} on the roster)")

    if measured < FLOOR:
        print(f"\nonly {measured} measured claim(s) were found, below the "
              f"floor of {FLOOR}. The admonition syntax or the wording "
              f"changed, so this gate measured almost nothing — refusing "
              f"rather than passing. Measured 2026-09-10 it was 85.",
              file=sys.stderr)
        return 2

    new = sorted(have - want)
    gone = sorted(want - have)
    if new:
        print("  NEW — a measured claim with nothing a reader can run. Three "
              "boxes needed rewriting in one day for exactly this:")
        for page, title in new:
            print(f"    + {page}  :::{title}")
    if gone:
        print("  GONE — addressed, moved or deleted; drop the row so the "
              "roster keeps meaning something:")
        for page, title in gone:
            print(f"    - {page}  :::{title}")
    return 1 if (new or gone) else 0


if __name__ == "__main__":
    sys.exit(main())
