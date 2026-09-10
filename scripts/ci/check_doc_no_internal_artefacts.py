#!/usr/bin/env python3
"""The public site must not carry the team's own bookkeeping.

WHY, AND HOW MANY THERE WERE
----------------------------
The site's own rules forbid internal feature-version labels (`FV-N`) and
tracker numbers, with the remedy stated: "Describe the change content,
not its ticket." Nothing read the site for them, and on 2026-09-10 it
carried **23 task IDs across 19 pages** — `Tracked as T1341.`,
`(T1268)`, `repaired under T1068.` — plus one link to
`.../blob/main/internal/specs/cli-framework.md`, a path that is not in
the repository at all (`git ls-files` returns nothing for it), so the
link was a 404 pointing at a private directory.

Every one of the 23 was a parenthetical. The prose around it already
described the defect in full, which is why removing them cost a reader
nothing — and why they accumulated: each looked harmless on its own.

EIGHT OF THE EIGHTEEN TASKS WERE ALREADY CLOSED, and that is the second
harm. A page saying "Tracked as T1341" tells a reader the problem is
open. Two of those pages were stale in substance as well, not only in
the citation, and both had to be re-measured rather than merely
de-numbered — which is the whole reason a ticket is a poor thing to
leave in a document: it moves, and the sentence around it does not.

WHAT THIS DOES NOT FLAG, measured before the rule was written:
  * `internal/` as a WORD — `guides/best-practices.md` draws a user's
    own project tree with an `internal/` directory, `stdlib/math.md`
    names the library's own `internal/` subdirectory, and
    `net/weft/spiffe.md` has a `billing.internal` hostname. Only a LINK
    whose target is under `internal/` is the repository's private tree.
  * `T` followed by one or two digits — `T1`, `T2` are type parameters.
    The pool's IDs are `T0123`, four digits, and three are admitted so
    an older `T999` cannot slip through.
"""

from __future__ import annotations

import os
import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parents[2]
_DOCS_ROOT = os.environ.get("VERUM_DOCS_DIR")
DOCS = pathlib.Path(_DOCS_ROOT) if _DOCS_ROOT else REPO.parent / "website" / "docs"

TASK_ID = re.compile(r"\bT[0-9]{3,4}\b")
FV = re.compile(r"\b(?:Pre-|post-)?FV-[0-9]+\b")
# A LINK into the private tree, not the word. `](…internal/…)` covers the
# markdown form and `href="…internal/…"` the JSX one.
#
# THE OPTIONAL `(?:…/)?` IS THE WHOLE RULE, and a first version got it
# wrong in the direction that matters. Writing `(?:^|/)internal/` anchors
# `^` to the start of the LINE, not to the start of the link target, so
# a bare relative `](internal/specs/x.md)` — the likeliest spelling of
# all — went unseen. Requiring a `/` immediately before the word is what
# keeps `my-internal/` and `billing.internal` out.
INTERNAL_LINK = re.compile(
    r"\]\(\s*(?:[^)]*?/)?internal/|href=[\"'](?:[^\"']*?/)?internal/")

# THE DEBT REGISTER'S ROW IDS, in the spelling the site actually used:
# "tracked as A78 in the tech-debt register", "A84 in the debt register",
# "Tracked as A79". A bare `\bA\d+\b` is NOT usable — the site has
# mermaid nodes `A1`/`A2`, a `Cortex-A53`, and hex bytes like `A2 11 16`.
# Measured before the rule was written: 16 lines match the bare pattern
# and only 3 are register IDs. So the line must ALSO speak of tracking.
DEBT_ID = re.compile(r"\bA[0-9]{2,3}\b(?=.*\b(?:debt|register|tracked)\b)"
                     r"|\b(?:debt|register|tracked)\b.*?\bA[0-9]{2,3}\b",
                     re.I)

RULES = (
    ("an internal task ID", TASK_ID,
     "describe what the ticket says; a number means nothing to a reader "
     "and goes stale when the task closes"),
    ("an internal feature-version label", FV,
     "describe the current state directly"),
    ("a link into the private internal/ tree", INTERNAL_LINK,
     "that path is not in the repository, so the link is a 404"),
    ("a debt-register row ID", DEBT_ID,
     "the row number says nothing a reader can act on; state the gap and "
     "the workaround, which the surrounding prose already did in every "
     "case found"),
)


def scan(root: pathlib.Path) -> list[tuple[str, int, str, str]]:
    hits: list[tuple[str, int, str, str]] = []
    for p in sorted(root.rglob("*")):
        if p.suffix not in (".md", ".mdx", ".ts", ".tsx", ".js", ".jsx"):
            continue
        if "node_modules" in p.parts:
            continue
        for n, line in enumerate(p.read_text(errors="replace").split("\n"), 1):
            for what, pat, _ in RULES:
                m = pat.search(line)
                if m:
                    hits.append((str(p), n, what, m.group(0)[:60]))
    return hits


def self_test() -> int:
    bad = 0
    # ANCHOR ONE: lines that were REALLY on the site and had to go.
    must_catch = [
        "array, not `-instrument-coverage`. Tracked as T1341.",
        "  did not exist — fixed 2026-09-09 (T1268);",
        "scalars have none) was repaired under T1068.",
        "- [`internal/specs/cli-framework.md`]"
        "(https://github.com/verum-lang/verum/blob/main/internal/specs/cli-framework.md)",
        "An earlier release shipped FV-9 axioms.",
        # THE LIKELIEST SPELLING, and the one a first version of the link
        # pattern could not see: a bare relative target.
        "[spec](internal/specs/x.md)",
        "[spec](../../internal/specs/x.md)",
        '<a href="internal/holon/x">spec</a>',
        # The debt register's rows, in the three spellings the site used.
        "against the feature; it is tracked as A78 in the tech-debt register.",
        "`self.get(i)` are the two spellings that work today; A84 in the debt",
        "Binding through the base type avoids it. Tracked as A79; `.len()` and",
    ]
    # ANCHOR TWO, A DIFFERENT SHAPE: lines that are on the site and must
    # STAY. A gate that only proves it can accuse has not been checked.
    must_pass = [
        "│   └── internal/          # non-public helpers",
        "`observational/`, `examples/`, `internal/`, `simple/`, `advanced/`,",
        'let resp = upstream.get("https://billing.internal/api/...").send().await?;',
        "type Pair<T1, T2> is { a: T1, b: T2 };",
        "the T5 tier and the T80 budget",
        # A `/` must sit immediately before the word, or the rule eats
        # hostnames and hyphenated path segments that are nobody's
        # private tree.
        "[a](https://ex.com/my-internal/x)",
        "[spec](https://example.com/external/x.md)",
        # `A\d+` is everywhere and almost never a register row. These
        # three are on the site right now and must stay.
        "    A1[[\"Adapter A\"]]",
        "| ARMv8 Cortex-A53 (budget mobile) | ~8,000/s |",
        "             C2 A2 11 16 7A BB 8C 5E 07 9E 09 E2 C8 A8 33 9C",
        "checking absorbs what Rust's lifetime analysis tracked statically.",
    ]
    for line in must_catch:
        if not any(pat.search(line) for _, pat, _ in RULES):
            print(f"self-test: MISSED a line that was really removed: {line[:70]}",
                  file=sys.stderr)
            bad += 1
    for line in must_pass:
        hit = next((w for w, pat, _ in RULES if pat.search(line)), None)
        if hit:
            print(f"self-test: FALSE accusation ({hit}) on a legitimate line: "
                  f"{line[:70]}", file=sys.stderr)
            bad += 1
    if bad:
        print(f"self-test: {bad} FAILED", file=sys.stderr)
        return 1
    print(f"[ok] self-test: {len(must_catch)} real removals caught, "
          f"{len(must_pass)} legitimate lines left alone")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    if not DOCS.is_dir():
        print(f"check-doc-no-internal-artefacts: no website at {DOCS} — "
              f"REFUSING to report OK. A gate whose INPUT is missing is a "
              f"failed checkout, not 'nothing to do'; set VERUM_DOCS_DIR.",
              file=sys.stderr)
        return 2

    roots = [DOCS]
    src = DOCS.parent / "src"
    if src.is_dir():
        roots.append(src)
    hits = [h for r in roots for h in scan(r)]
    files = len({h[0] for h in hits})
    print(f"check-doc-no-internal-artefacts: {len(hits)} internal artefact(s) "
          f"across {files} file(s) in {len(roots)} tree(s)")

    if hits:
        why = {w: r for w, _, r in RULES}
        for path, n, what, text in hits:
            rel = path.split("/website/", 1)[-1]
            print(f"    + {rel}:{n}  {what} — {text}")
            print(f"        {why[what]}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
