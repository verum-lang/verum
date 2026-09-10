#!/usr/bin/env python3
"""An unclosed fence silently swallows the rest of the page.

WHY, AND WHAT IT LOOKED LIKE
----------------------------
Markdown does not fail on an unbalanced fence, and neither does the site
build. The renderer simply keeps the code block open, so every heading,
paragraph and example below the stray marker is displayed as raw source.
Four pages were in that state on 2026-09-10, and all four came from the
same edit: a code block was removed and one of its two markers was left
behind.

    stdlib/cog.md            a stray OPENER before prose — the
                             explanation and the whole `CogArchive`
                             declaration rendered as one code block
    term/reference/api-event.md  a stray CLOSER after a table, which
                             swallowed `## MouseEvent` and the type
                             under it
    language/refinement-types.md a stray CLOSER after a paragraph
    cookbook/smt-debug.md    a stray OPENER after a sentence ending in
                             `:` — this one corrupted the ENTIRE tail of
                             the page, re-pairing nine later fences and
                             turning `### Diagnostic flags` into code

The last is the reason this is a gate rather than a one-off sweep. One
stray marker does not damage one block; it re-pairs every fence after
it, and the damage grows with the length of the page.

FENCE LENGTH IS PART OF THE RULE, not a detail. `tooling/auto-paper.md`
opens a ````markdown block that CONTAINS a ```verum pair. Counting
backtick lines and asking whether the total is even calls that file
balanced — but only by accident, because the inner pair happens to be
even. An odd number of inner markers would be reported as a defect in a
correct file, and a real defect inside such a block would be hidden. So
this walks the file the way CommonMark does: a fence closes only on a
run at least as long as the one that opened it, and everything between
is content.

ADMONITIONS TOO, by the same walk. `:::caution` … `:::` is Docusaurus,
not CommonMark, and an unclosed one absorbs the rest of the page in the
same way. Markers inside a code fence are content, which is why this
cannot be done with two independent counts.
"""

from __future__ import annotations

import os
import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
_DOCS_ROOT = os.environ.get("VERUM_DOCS_DIR")
DOCS = pathlib.Path(_DOCS_ROOT) if _DOCS_ROOT else REPO.parent / "website" / "docs"

# CommonMark allows a fence to be indented up to three spaces; four makes
# it an indented code block instead.
FENCE = re.compile(r"^ {0,3}(`{3,}|~{3,})(.*)$")
ADM_OPEN = re.compile(r"^:::[a-z]")
ADM_CLOSE = re.compile(r"^:::\s*$")


def defects(text: str) -> list[tuple[str, int, str]]:
    """[(what, line, detail)] — every mispaired or unclosed region.

    TWO DEFECTS, NOT ONE, and the second is why a walk alone is not
    enough. `stdlib/cog.md` had a stray ```verum before a paragraph and
    was perfectly BALANCED by CommonMark's rules: the stray opener
    simply made the prose and the next opener into content, and the
    later bare ``` closed it. Every block closed; the page rendered as
    nonsense. Only the shape gives it away — an opener carrying an INFO
    STRING, at the same fence length, inside an open fence of that
    length. That is someone starting a block inside a block, which no
    page means to do.

    Nesting at a DIFFERENT length is deliberate and stays silent:
    `tooling/auto-paper.md` documents markdown by putting a ```verum
    pair inside a ````markdown block.
    """
    fence: tuple[str, int, int] | None = None   # (char, length, line)
    adm: int | None = None
    out: list[tuple[str, int, str]] = []
    for n, line in enumerate(text.split("\n"), 1):
        m = FENCE.match(line)
        if m:
            run, rest = m.group(1), m.group(2)
            if fence is None:
                fence = (run[0], len(run), n)
                continue
            # A closer matches the opener's character, is at least as
            # long, and carries no info string.
            if run[0] == fence[0] and len(run) >= fence[1] and not rest.strip():
                fence = None
            elif run[0] == fence[0] and len(run) == fence[1] and rest.strip():
                out.append(("a fence opened INSIDE a fence", n,
                            f"{run}{rest.strip()[:20]} at line {n}, inside the "
                            f"block opened at line {fence[2]}"))
            continue
        if fence is not None:
            continue                      # inside a fence: everything is content
        if ADM_OPEN.match(line):
            if adm is None:
                adm = n
        elif ADM_CLOSE.match(line):
            adm = None
    if fence is not None:
        out.append(("a code fence", fence[2], fence[0] * fence[1]))
    if adm is not None:
        out.append(("an admonition", adm, ":::"))
    return out


def unclosed(text: str) -> list[tuple[str, int, str]]:
    return defects(text)


def self_test() -> int:
    bad = 0
    cases = [
        # (label, text, expect_defect)
        ("a balanced pair", "a\n\n```\nx\n```\n\nb\n", False),
        ("a stray OPENER before prose (cog.md)",
         "```verum\nProse that is not code.\n\n```verum\ntype A is Int;\n```\n", True),
        ("a stray CLOSER after a table (api-event.md)",
         "| a | b |\n|---|---|\n| 1 | 2 |\n```\n\n## Next\n", True),
        ("a stray CLOSER after prose (refinement-types.md)",
         "Some prose about a claim.\n```\n\n## Limitations\n", True),
        ("a stray OPENER after a colon (smt-debug.md)",
         "You would read the same shape:\n```\n\nOften the next thing.\n", True),
        # THE ACCIDENT THIS RULE EXISTS FOR: a longer fence containing a
        # shorter pair. A count of marker lines calls the first balanced
        # only because the inner pair is even, and calls the second
        # broken although it is fine.
        ("a ````block containing a ```pair",
         "````markdown\ntext\n```verum\nx\n```\nmore\n````\n", False),
        ("a ````block containing ONE ```line",
         "````markdown\nSee ``` for fences.\n````\n", False),
        ("an unclosed admonition", ":::caution T\nbody\n\n## Next\n", True),
        ("an admonition marker INSIDE a fence is content",
         "```\n:::caution not real\n```\n", False),
        ("a closed admonition", ":::note T\nbody\n:::\n", False),
    ]
    for label, text, want in cases:
        got = bool(unclosed(text))
        if got != want:
            print(f"self-test: {label}: expected defect={want}, got {got}",
                  file=sys.stderr)
            bad += 1
    if bad:
        print(f"self-test: {bad} FAILED", file=sys.stderr)
        return 1
    print(f"[ok] self-test: {len(cases)} cases — four real breakages, "
          f"the nested-fence accident, and the admonition pair")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    if not DOCS.is_dir():
        print(f"check-doc-fences-close: no website at {DOCS} — REFUSING to "
              f"report OK. A gate whose INPUT is missing is a failed "
              f"checkout, not 'nothing to do'; set VERUM_DOCS_DIR.",
              file=sys.stderr)
        return 2

    pages = sorted(DOCS.rglob("*.md"))
    hits = []
    for p in pages:
        for what, line, marker in unclosed(p.read_text(errors="replace")):
            hits.append((str(p.relative_to(DOCS)), line, what, marker))

    print(f"check-doc-fences-close: {len(pages)} page(s), {len(hits)} "
          f"mispaired or unclosed block(s)")
    for page, line, what, detail in hits:
        if what.startswith("a fence opened"):
            print(f"    + {page}:{line}  {what} — {detail}. The page still "
                  f"BALANCES; it just pairs wrong, so the text between the "
                  f"two openers renders as code")
        else:
            print(f"    + {page}:{line}  {what} (`{detail}`) is never closed "
                  f"— everything below it renders as raw source")
    return 1 if hits else 0


if __name__ == "__main__":
    sys.exit(main())
