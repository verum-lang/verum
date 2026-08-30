#!/usr/bin/env python3
"""Compare the published grammar page against `grammar/verum.ebnf` (T0942).

`grammar/verum.ebnf` is the only source of truth for Verum syntax, and the
website ships a hand-maintained excerpt of it that calls itself "the
authoritative specification of Verum's concrete syntax".  Two authorities,
one hand-copied from the other, drift — and the drift is invisible because
neither file is executable.

Measured instance that motivated this check: the site carried
`bitwise_expr = shift_expr , { ( '&' | '|' | '^' ) , shift_expr }` — one flat
level — long after the compiler had settled on the C ladder, so the two
documents predicted DIFFERENT VALUES for `a | b & c`.

This script compares every production that appears in BOTH files and reports
the ones whose right-hand sides differ.  Productions the excerpt omits are
fine by construction; productions it renames or re-spells are not.

    scripts/ci/check_site_grammar_parity.py [--grammar PATH] [--page PATH]

Exit status 1 on any divergence.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
DEFAULT_GRAMMAR = REPO / "grammar" / "verum.ebnf"
DEFAULT_PAGE = REPO / "internal" / "website" / "docs" / "reference" / "grammar-ebnf.md"

COMMENT = re.compile(r"\(\*.*?\*\)", re.DOTALL)
PRODUCTION = re.compile(r"^([a-z_][a-z0-9_]*)\s*=\s*(.*?);", re.DOTALL | re.MULTILINE)


def normalise(rhs: str) -> str:
    """Collapse whitespace so alignment differences do not read as drift."""
    return " ".join(COMMENT.sub(" ", rhs).split())


def productions(text: str) -> dict[str, str]:
    text = COMMENT.sub(" ", text)
    out: dict[str, str] = {}
    for name, rhs in PRODUCTION.findall(text):
        # First definition wins: later repeats in prose are illustrations.
        out.setdefault(name, normalise(rhs))
    return out


def ebnf_blocks(markdown: str) -> str:
    return "\n".join(re.findall(r"```ebnf\n(.*?)```", markdown, re.DOTALL))


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--grammar", type=Path, default=DEFAULT_GRAMMAR)
    ap.add_argument("--page", type=Path, default=DEFAULT_PAGE)
    args = ap.parse_args()

    if not args.grammar.is_file():
        print(f"missing grammar: {args.grammar}", file=sys.stderr)
        return 2
    if not args.page.is_file():
        # The website is a separate repository and may not be checked out.
        print(f"site grammar page not present ({args.page}) — nothing to compare")
        return 0

    truth = productions(args.grammar.read_text(encoding="utf-8"))
    page = productions(ebnf_blocks(args.page.read_text(encoding="utf-8")))

    # Positive control: if the extractor stops matching, an empty overlap
    # would make this gate pass while comparing nothing.
    shared = sorted(set(truth) & set(page))
    if len(shared) < 20:
        print(
            f"REFUSING TO PASS: only {len(shared)} productions matched in both "
            f"files ({len(truth)} in the grammar, {len(page)} on the page). "
            "The extractor is broken, not the documents.",
            file=sys.stderr,
        )
        return 2

    drift = [(n, truth[n], page[n]) for n in shared if truth[n] != page[n]]

    print(f"compared {len(shared)} shared productions "
          f"({len(truth)} in grammar, {len(page)} on the page)")
    if not drift:
        print("site grammar page agrees with grammar/verum.ebnf")
        return 0

    print(f"\n{len(drift)} production(s) differ:\n", file=sys.stderr)
    for name, want, got in drift:
        print(f"  {name}", file=sys.stderr)
        print(f"    grammar/verum.ebnf : {want}", file=sys.stderr)
        print(f"    website page       : {got}", file=sys.stderr)
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
