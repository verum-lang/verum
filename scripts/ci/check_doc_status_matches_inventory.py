#!/usr/bin/env python3
"""A stdlib page may not claim a conformance status GREENER than the tree.

WHY THIS EXISTS
---------------
The stdlib reference marks every module `**complete**`, `**stable**`,
`**partial**`, `**regression-only**`, `**undocumented**` or
`**unverified**`, and `core-tests/INVENTORY.md` carries the same
vocabulary per module, keyed on the same folder the page links to.
Nothing compared them.  Measured 2026-09-10:

    152 rows carry both a link and a token
    116 agree, 26 GREENER THAN THE TREE, 10 more conservative

    partial  -> unverified        13 rows
    complete -> partial            6 rows
    complete -> unverified         2 rows
    partial  -> regression-only    2 rows
    stable   -> partial            2 rows  (base/maybe, collections/deque)
    partial  -> stable             1 row

READ THE FIRST NUMBERS THIS FILE CARRIED AS A WARNING ABOUT ITSELF.
They were 135 rows / 25 disagreements / "2 cite a module the inventory
does not have", and the last of those was FALSE.  This gate's RANK
table lacked `complete`, which 40 inventory rows lead with, so those
rows parsed to no status at all and the gate announced that somebody
else's file was missing them.  A parser that cannot classify its input
does not go quiet — it blames the other side.  Hence `ANY_BOLD` and the
rc=2 refusal below: an unrecognised status word is now a stop, not a
skip.

ONE-DIRECTIONAL, AND THAT IS THE POINT
--------------------------------------
The rule is `doc_rank <= inventory_rank`, not equality.  A page more
conservative than the inventory is fine; a page greener than it is not.

Equality would be the wrong rule for a reason that is written down in
this repository: the inventory is STALE-GREEN — a row can claim `stable`
because nobody has re-run it, and `check-inventory-live` exists precisely
because that happened (`diagnostics` and `signal` both claimed a green
status the suite had never been run against).  Demanding equality would
push the SITE up to meet a number that may itself be unmeasured, which is
the opposite of what this gate is for.

So the failing direction is only ever "the page tells the reader more
than the tree recorded".

RANKS
    unverified / undocumented   0   nothing was measured
    regression-only             1   public-API tests do not pass
    partial                     2   a subset is conformance-tested
    stable                      3   every public method is
    complete                    4   stable, plus property laws, cross-stdlib
                                    integration and a routed audit

`unverified` and `undocumented` share rank 0 deliberately: they differ in
WHY nothing was measured, not in how much is claimed.

`complete` sits ABOVE `stable` on the inventory legend's own wording —
"rows marked stable graduate to complete when those land".  The SITE
disagreed: `docs/stdlib/async.md` defined complete as a "synonym for
stable".  Two legends for one vocabulary, and the site's was the wrong
one; corrected there rather than accommodated here.

WHAT IT DOES NOT CHECK
    Whether the inventory's own token is true — that is
    `make check-inventory-live`.  This gate only compares two written
    claims about the same module and refuses the greener one.

    Rows with no `core-tests/` link.  The link is the module key; a row
    that carries a token and no link is invisible here and is reported
    separately as a count, so the denominator stays honest.
"""

from __future__ import annotations

import os
import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
INVENTORY = REPO / "core-tests" / "INVENTORY.md"

# The site is a SEPARATE checkout beside this one; a tracked file may not
# name the working copy's private path.  Gate: `make check-internal-refs`.
_DOCS_ROOT = os.environ.get("VERUM_DOCS_DIR")
DOCS = pathlib.Path(_DOCS_ROOT) if _DOCS_ROOT else REPO.parent / "website" / "docs"

RANK = {
    "unverified": 0,
    "undocumented": 0,
    "regression-only": 1,
    "partial": 2,
    "stable": 3,
    "complete": 4,
}
TOKEN = re.compile(r"\*\*(" + "|".join(re.escape(t) for t in RANK) + r")\*\*")
# Any bold lower-case word, so a row whose status word this gate does not
# KNOW is distinguishable from a row that has no status at all.  The first
# version of this file lacked `complete`, and the two rows carrying it were
# reported as "the inventory does not have this module" — a false statement
# about somebody else's file, produced by a parser that could not classify
# its own input.  An unknown token is now rc=2, not a silent skip.
ANY_BOLD = re.compile(r"\*\*([a-z][a-z -]{2,24})\*\*")
INV_ROW = re.compile(r"\|\s*`([a-z0-9_/]+)`\s*\|")
# A row may cite the FOLDER or a file inside it — `.../core-tests/text/char)`
# and `.../core-tests/text/char/audit.md)` are the same claim.  The first
# version demanded a closing paren immediately after the folder and so
# missed every audit.md link, which put a whole page's worth of rows in
# the "carries a status nothing can check" bucket while they were in fact
# linked.  Measured wrong before measured right.
DOC_LINK = re.compile(r"core-tests/((?:[a-z0-9_]+/)*[a-z0-9_]+)(?:/[a-z0-9_.]+\.md)?\)")


def inventory_status(text: str) -> tuple[dict[str, str], dict[str, str]]:
    """module path -> its token; plus module path -> an UNKNOWN token.

    The row's status is its FIRST bold word: the prose that follows often
    quotes a superseded one ("prior claim was **complete** — drift caught
    by check-inventory-live"), and reading the last would report history
    as the current state.
    """
    known: dict[str, str] = {}
    unknown: dict[str, str] = {}
    for line in text.split("\n"):
        m = INV_ROW.match(line)
        if not m:
            continue
        bolds = ANY_BOLD.findall(line)
        if not bolds:
            continue
        first = bolds[0]
        if first in RANK:
            known[m.group(1)] = first
        else:
            unknown[m.group(1)] = first
    return known, unknown


def doc_rows(text: str) -> tuple[list[tuple[int, str, str]], int]:
    """(line, module, token) per checkable row, plus the UNCHECKABLE count.

    A row that carries a status token but no `core-tests/` link has no
    module key, so this gate cannot compare it with anything. Those are
    counted and printed rather than dropped: `docs/stdlib/text.md` alone
    carries ten `**complete**` claims in that shape, and a denominator
    that quietly excludes them would make the gate's coverage look
    larger than it is.
    """
    out = []
    unlinked = 0
    for i, line in enumerate(text.split("\n"), 1):
        if not line.startswith("|"):
            continue
        link = DOC_LINK.search(line)
        tok = TOKEN.search(line)
        if tok and not link:
            # the legend table itself leads with the token and is not a claim
            if not line.startswith("| **"):
                unlinked += 1
            continue
        if link and tok:
            out.append((i, link.group(1), tok.group(1)))
    return out, unlinked


def self_test() -> int:
    bad = 0

    inv, unknown = inventory_status(
        "| `a/one`   | 1 | 2 | 3 | 4 | 0. **stable** under `--interp`. |\n"
        "| `a/two`   | 1 | 2 | 3 | 4 | 0. **unverified** — never run. |\n"
        "| `a/hist`  | 1 | 2 | 3 | 4 | **partial** — prior claim was **complete**. |\n"
        "| `a/what`  | 1 | 2 | 3 | 4 | 0. **mostly fine** — a word this gate lacks. |\n"
        "| not a row, no backticks, **stable** |\n"
    )
    if inv != {"a/one": "stable", "a/two": "unverified", "a/hist": "partial"}:
        print(f"self-test: inventory parse wrong: {inv}", file=sys.stderr)
        bad += 1
    if unknown != {"a/what": "mostly fine"}:
        print(f"self-test: an unknown status word was not isolated: {unknown}",
              file=sys.stderr)
        bad += 1

    linked_via_file, _ = doc_rows(
        "| `x.vr` | **partial** | [audit](https://x/core-tests/a/one/audit.md) |\n"
    )
    if linked_via_file != [(1, "a/one", "partial")]:
        print(f"self-test: a link through the folder's audit.md was not "
              f"resolved: {linked_via_file}", file=sys.stderr)
        bad += 1

    rows, unlinked = doc_rows(
        "| `one.vr` | **partial** | [core-tests/a/one](https://x/core-tests/a/one) — ok |\n"
        "| `two.vr` | **partial** | [core-tests/a/two](https://x/core-tests/a/two) — ok |\n"
        "| `no.vr`  | **stable**  | no link on this row |\n"
        "| **stable** | a LEGEND row, not a claim about a module |\n"
        "not a table line at all, **stable**, core-tests/a/one)\n"
    )
    if rows != [(1, "a/one", "partial"), (2, "a/two", "partial")]:
        print(f"self-test: doc row parse wrong: {rows}", file=sys.stderr)
        bad += 1
    if unlinked != 1:
        print(f"self-test: unlinked count wrong (legend row must not count): "
              f"{unlinked}", file=sys.stderr)
        bad += 1

    # POLARITY.  Same two rows, same inventory: one is conservative
    # (partial under stable — allowed), one is greener (partial over
    # unverified — refused).  A rule of EQUALITY would flag both, which
    # is what this pair is here to prevent anyone reintroducing.
    over = [(m, d) for _, m, d in rows if RANK[d] > RANK[inv[m]]]
    if [m for m, _ in over] != ["a/two"]:
        print(f"self-test: polarity wrong, expected only a/two: {over}", file=sys.stderr)
        bad += 1

    under = [(m, d) for _, m, d in rows if RANK[d] < RANK[inv[m]]]
    if [m for m, _ in under] != ["a/one"]:
        print(f"self-test: the conservative row was not recognised: {under}",
              file=sys.stderr)
        bad += 1

    missing = [m for _, m, _ in doc_rows(
        "| `x.vr` | **stable** | [core-tests/a/nope](https://x/core-tests/a/nope) |\n"
    )[0] if m not in inv and m not in unknown]
    if missing != ["a/nope"]:
        print(f"self-test: a row citing an absent module was not caught: {missing}",
              file=sys.stderr)
        bad += 1

    if bad:
        print(f"self-test: {bad} FAILED", file=sys.stderr)
        return 1
    print(f"[ok] self-test: {len(RANK)} ranks, 2 parsers, "
          f"3 polarity cases (1 over, 1 under, 1 absent), "
          f"1 unknown-word case, 1 superseded-token row, "
          f"1 unlinked claim vs 1 legend row, 1 link through audit.md")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()

    if not INVENTORY.is_file():
        print(f"check-doc-status-matches-inventory: no inventory at {INVENTORY}",
              file=sys.stderr)
        return 2
    if not DOCS.is_dir():
        print(f"[skip] {DOCS} not present (the site is a separate checkout).")
        return 0

    inv, unknown = inventory_status(INVENTORY.read_text(errors="replace"))
    if unknown:
        print(f"check-doc-status-matches-inventory: {len(unknown)} inventory "
              f"row(s) lead with a status word this gate does not rank:",
              file=sys.stderr)
        for mod, word in sorted(unknown.items())[:10]:
            print(f"    {mod}: **{word}**", file=sys.stderr)
        print("  Add it to RANK with the position the legend gives it, or "
              "fix the row. Skipping it silently would blame the PAGE for "
              "an inventory this gate could not read.", file=sys.stderr)
        return 2
    if len(inv) < 100:
        print(f"check-doc-status-matches-inventory: the inventory parsed to "
              f"only {len(inv)} module(s) — the table shape changed and this "
              f"gate would go quiet rather than green", file=sys.stderr)
        return 2

    pages = sorted((DOCS / "stdlib").glob("*.md")) if (DOCS / "stdlib").is_dir() else []
    over: list[str] = []
    absent: list[str] = []
    under = agree = total = unlinked = 0
    for page in pages:
        rows, n = doc_rows(page.read_text(errors="replace"))
        unlinked += n
        for i, mod, tok in rows:
            total += 1
            if mod not in inv:
                # The folder exists (the link resolves) but the inventory
                # records no status for it.  Nothing was measured, so
                # nothing above rank 0 is supported — the page may say
                # `unverified` / `undocumented` and no more.
                if RANK[tok] > 0:
                    absent.append(
                        f"{page.name}:{i}  {mod} — page says {tok}, "
                        f"inventory records nothing")
                else:
                    agree += 1
                continue
            if RANK[tok] > RANK[inv[mod]]:
                over.append(f"{page.name}:{i}  {mod}: page {tok} > tree {inv[mod]}")
            elif RANK[tok] < RANK[inv[mod]]:
                under += 1
            else:
                agree += 1

    print(f"check-doc-status-matches-inventory: {total} checkable row(s) "
          f"across {len(pages)} page(s) against {len(inv)} inventory "
          f"module(s) — {agree} equal, {under} more conservative than the "
          f"tree, {len(over)} greener, {len(absent)} citing no inventory row; "
          f"{unlinked} further row(s) carry a status with no conformance "
          f"link and cannot be checked by anything")

    if over:
        print("  GREENER THAN THE TREE — the page tells a reader more than")
        print("  core-tests/INVENTORY.md records for that module:")
        for s in over:
            print(f"    + {s}")
    if absent:
        print("  NO INVENTORY ROW — the page links a conformance folder the")
        print("  inventory records no status for, and claims more than")
        print("  `unverified`; nothing measured supports it:")
        for s in absent:
            print(f"    ? {s}")
    if over or absent:
        print("  Lower the page's token, or add the module to the inventory")
        print("  with a status somebody measured.")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
