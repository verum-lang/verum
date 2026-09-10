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
    a fenced block, an INDENTED block (this site sets `format: 'md'`,
    so four spaces really is a code block — see `has_indented_code`),
    or an inline / leading `verum`, `grep`, `strings`, `cargo`, `make`,
    `python3`. Deliberately generous: this gate is not judging whether
    the evidence is GOOD, only refusing a measured claim with nothing at
    all to run.

THE ROSTER IS NOT A COUNT. It opened at thirty-five; a swap of two
would leave a count unmoved, so rows are listed by (page, title) and
both a NEW one and a VANISHED one fail. Removing a row is how the number
goes down, and that requires giving the box its address.

THE ROSTER IS NOW EMPTY, 35 -> 0 in one day, and that is the strongest
state rather than a finished one: every measured claim on the site
carries something a reader can run, so the only thing this gate can
report is a NEW one. Verified against an empty roster — a naked box
added to any page still fails it.

WHAT THE CAMPAIGN ACTUALLY FOUND. Six boxes were simply naked and gained
a command. NINE were never naked at all and the gate was wrong about them
(see `has_indented_code`). The rest were rewritten one at a time, and
SEVEN of those rewrites turned up a claim that was WRONG rather than
merely unaddressed:

    a page denying a `Router` that exists under another meaning
    an h3 caution denying a writer and a cancellation core ships
    a token-API census denying two names declared elsewhere
    a line number that had moved eight lines
    a `Map.get_optional` the library does not declare
    five of six rows in a defect table, all already fixed
    a workaround still taught after the defect closed

Every one surfaced while writing the command down, which is the argument
for this gate in one sentence: you cannot write the address of a claim
you have not re-checked.

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
RUNNABLE = re.compile(
    r"```|`(?:verum|grep|strings|cargo|make|python3)\b|"
    r"^\s*(?:verum|grep|strings|cargo|make)\s",
    re.M,
)
MARKER = re.compile(r"^(\s*)(?:[-*+]|\d+[.)])(\s+)\S")
FLOOR = 40


def has_indented_code(body: str) -> bool:
    """A CommonMark indented code block, without a Markdown parser.

    THIS IS AN ADDRESS TOO, and leaving it out was a false accusation.
    The site sets `format: 'md'` in `docusaurus.config.ts`, so its 370
    pages compile as CommonMark rather than MDX — measured by running
    `@mdx-js/mdx` both ways over one page's block: under `md` it becomes
    `<pre><code>`, under `mdx` the compile FAILS, because `error<E018>:`
    in a paragraph is read as an unclosed JSX tag. So an indented block
    on this site really is code, and a box printing its program that way
    carries the strongest evidence there is — the input travels with the
    claim. Ten such boxes were being reported as having nothing to run.

    CI installs no Python packages, so this cannot call a Markdown
    library; it is a hand rule, validated against `markdown-it-py` in
    commonmark mode over all 203 admonitions on the site (13 positive,
    190 negative, 0 disagreements) and over nine adversarial fixtures.

    THE LIST CASE IS WHY THE FIXTURES EXIST. A first version asked only
    for four spaces after a blank line, which the corpus agreed with —
    and it read a list item's continuation paragraph as code. That error
    runs the WRONG WAY for a ratchet: a box with no evidence would count
    as addressed and leave the roster silently. Inside a list item whose
    content starts at column N, CommonMark wants N+4.
    """
    fence = False
    prev_blank = True
    content_col = 0
    for line in body.split("\n"):
        s = line.strip()
        if s.startswith("```") or s.startswith("~~~"):
            fence = not fence
            prev_blank = False
            continue
        if fence:
            continue
        if not s:
            prev_blank = True
            continue
        indent = len(line) - len(line.lstrip(" "))
        if indent < content_col and not prev_blank:
            content_col = 0
        m = MARKER.match(line)
        if m and indent <= content_col + 3:
            content_col = len(m.group(1)) + (len(m.group(0)) - len(m.group(1)) - 1)
            prev_blank = False
            continue
        if prev_blank and indent >= content_col + 4:
            return True
        if indent < content_col:
            content_col = 0
        prev_blank = False
    return False


def has_address(body: str) -> bool:
    return bool(RUNNABLE.search(body)) or has_indented_code(body)


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
        if has_address(body):
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
        # AN INDENTED BLOCK IS CODE ON THIS SITE (`format: 'md'`), so a
        # box printing its program that way carries its own input.
        ("an indented block counts",
         ":::caution T\nMeasured 2026-09-10:\n\n    verum check x.vr\n"
         "      -> error<E018>\n:::\n", True),
        # AND THE ONE THAT RUNS THE WRONG WAY. A list item's continuation
        # paragraph is also four spaces in, and reading it as code would
        # EXCUSE a naked claim — the roster would lose a row nobody
        # addressed. Checked against markdown-it-py when the rule was
        # written; this fixture is what keeps it checked.
        ("a list continuation is not code",
         ":::caution T\nMeasured 2026-09-10.\n\n* the first point\n\n"
         "    a continuation of that point, not a program\n:::\n", False),
        ("an indent inside a fence is not a second block",
         ":::caution T\nMeasured.\n\n```\n    indented inside\n```\n:::\n",
         True),
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
        if has_address(m.group(3)) != want:
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
