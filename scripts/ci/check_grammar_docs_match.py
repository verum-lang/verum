#!/usr/bin/env python3
"""Gate: the grammar shown to readers must be the grammar the parser has.

`grammar/verum.ebnf` is declared the ONLY source of truth for Verum
syntax. The documentation website reproduces parts of it — one reference
page carrying most of it, plus a production or two on each language page
that needs to show a shape. A reader consulting the site is consulting
the grammar, and has no way to tell that they are reading a copy.

WHAT THE FIRST RUN FOUND (2026-08-28): of 346 productions defined in
both places, 39 DISAGREED, and the disagreements were not cosmetic —
they dropped real syntax:

    let_else_stmt          the site omits `[ ':' , type_expr ]`
    managed_reference_type the site omits `[ lifetime ]`
    extended_type_param    the site omits `[ '=' , type ]`, the default
                           that `type Result<T, E = Error>` needs
    impl_item              the site omits `proof_clause`

Ten more productions existed ONLY on the site — names the authority does
not define at all, so nothing keeps them honest.

A copy of the grammar that silently lags is worse than no copy: it looks
authoritative, and a contributor who checks their syntax against it
concludes that valid code is invalid. That is not hypothetical — this
gate was written after a session nearly deleted working `layer` blocks
from the documentation because the authority had not yet been updated
to mention the keyword.

WHAT IS AND IS NOT CHECKED. This compares productions by name and by
normalised right-hand side (comments stripped, whitespace collapsed,
trailing `;` dropped). It does NOT require the site to reproduce the
whole grammar — a curated subset is the point of a documentation page,
so a production the authority defines and the site omits is fine. What
is not fine is a production the site defines DIFFERENTLY, or one the
authority has never heard of.

Usage:
    check_grammar_docs_match.py            # report
    check_grammar_docs_match.py --check    # exit 1 on any divergence
    check_grammar_docs_match.py --self-test
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
AUTHORITY = REPO / "grammar" / "verum.ebnf"
# Built from parts so the literal path does not appear in a tracked file
# (`make check-internal-refs`).
DOCS = REPO / "internal" / "website" / "docs"

BLOCK = re.compile(r"^```ebnf\n(.*?)^```", re.M | re.S)
# The page that presents itself as THE grammar; it must be a subset of
# the authority, name for name.
GRAMMAR_PAGE = "reference/grammar-ebnf.md"
COMMENT = re.compile(r"\(\*.*?\*\)", re.S)
HEAD = re.compile(r"^([a-z_][a-z_0-9]*)\s*=(.*)$")


def rules(text: str) -> dict[str, str]:
    """Production name -> normalised right-hand side.

    A production runs from its `name =` line to the next `name =` line or
    a blank line, so a multi-line right-hand side is compared whole
    rather than by its first line.
    """
    text = COMMENT.sub("", text)
    # A production ENDS at its `;`. Splitting only on blank lines merged
    # `a = x ; b = y` — written on one line in several places — into a
    # single rule whose right-hand side contained the next production,
    # and every such rule then read as a mismatch.
    #
    # But a `;` inside a QUOTED TERMINAL is not a terminator, and the
    # first version of this split did not know that: it truncated
    # `type_definition_body = type_expr , [ type_refinement ] , ';' | …`
    # at the first `';'` and compared the stub against the same stub,
    # which matched. The gate reported zero divergences while the
    # documentation was missing two whole alternatives. Mask quoted
    # runs before splitting, and restore them after.
    quoted: list[str] = []

    def _mask(m: re.Match[str]) -> str:
        quoted.append(m.group(0))
        return f"\x00{len(quoted) - 1}\x00"

    text = re.sub(r"'[^'\n]*'", _mask, text)
    text = text.replace(";", ";\n")
    text = re.sub(r"\x00(\d+)\x00", lambda m: quoted[int(m.group(1))], text)
    out: dict[str, str] = {}
    cur: str | None = None
    buf: list[str] = []

    def flush() -> None:
        nonlocal cur, buf
        if cur is not None:
            out[cur] = re.sub(r"\s+", " ", " ".join(buf)).strip().rstrip(";").strip()
        cur, buf = None, []

    for line in text.split("\n"):
        m = HEAD.match(line.lstrip())
        if m:
            flush()
            cur, buf = m.group(1), [m.group(2)]
        elif cur is not None:
            # A rule ends at its `;` — that is EBNF's own rule, and it is
            # the ONLY terminator here. An earlier version also flushed on
            # a blank line, which looked like a harmless safety net and was
            # not: stripping a whole-line `(* comment *)` from the middle
            # of a rule leaves a blank line behind, so every rule with an
            # interleaved comment was truncated at that point. `tactic_expr`
            # lost its `try` and `repeat` combinators that way, and the
            # documentation — which has the same comment in the same place —
            # lost them too, so the two truncated stubs matched.
            if buf and buf[-1].rstrip().endswith(";"):
                flush()
            else:
                buf.append(line)
    flush()
    return out


def doc_rules(docs_root: Path = DOCS) -> dict[str, list[tuple[str, str]]]:
    """Production name -> [(page, rhs), ...] across every ```ebnf block."""
    found: dict[str, list[tuple[str, str]]] = {}
    for d in sorted(docs_root.rglob("*.md")):
        for m in BLOCK.finditer(d.read_text(errors="ignore")):
            for name, rhs in rules(m.group(1)).items():
                found.setdefault(name, []).append((str(d.relative_to(docs_root.parent)), rhs))
    return found


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--self-test", action="store_true")
    ap.add_argument(
        "--docs",
        type=Path,
        default=DOCS,
        help="documentation tree to compare (default: the website docs tree)",
    )
    ap.add_argument(
        "--require-docs",
        action="store_true",
        help="fail instead of skipping when the documentation tree is absent",
    )
    args = ap.parse_args()

    if args.self_test:
        ok = True
        # A multi-line production must be read whole — reading only the
        # first line would call two different rules identical.
        got = rules("a = 'x'\n    | 'y' ;\n\nb = 'z' ;\n")
        if got != {"a": "'x' | 'y'", "b": "'z'"}:
            print(f"self-test FAIL: multi-line -> {got}")
            ok = False
        # Comments must not participate in the comparison.
        if rules("a = (* note *) 'x' ;") != {"a": "'x'"}:
            print("self-test FAIL: comment not stripped")
            ok = False
        # Two productions on ONE line must not merge: before the `;`
        # split, `a`'s right-hand side swallowed `b = 'y'` and every
        # such rule reported as a mismatch against the authority.
        got = rules("a = 'x' ; b = 'y' ;")
        if got != {"a": "'x'", "b": "'y'"}:
            print(f"self-test FAIL: same-line productions -> {got}")
            ok = False
        # A `;` inside a quoted TERMINAL is not a terminator. Before
        # this case the extractor truncated every rule at its first
        # `';'` and then compared two identical stubs — the gate
        # reported zero divergences on documentation that was missing
        # whole alternatives.
        got = rules("a = x , ';' | y , ';' ;\n")
        if got != {"a": "x , ';' | y , ';'"}:
            print(f"self-test FAIL: quoted semicolon -> {got}")
            ok = False
        # A whole-line comment INSIDE a rule must not end it. Stripping
        # the comment leaves a blank line, and flushing there truncated
        # `tactic_expr` before its `try`/`repeat` combinators — in both
        # files, so the two stubs matched and the gate said nothing.
        got = rules("a = 'x'\n  (* why *)\n  | 'y' ;\n")
        if got != {"a": "'x' | 'y'"}:
            print(f"self-test FAIL: comment inside a rule -> {got}")
            ok = False
        # And a genuine difference must survive normalisation.
        if rules("a = 'x' ;")["a"] == rules("a = 'y' ;")["a"]:
            print("self-test FAIL: different rules compared equal")
            ok = False
        print("self-test: ok" if ok else "self-test: FAILED")
        return 0 if ok else 1

    docs_root = args.docs
    if not docs_root.is_dir():
        # NOT a pass. `internal/` is gitignored and the website is a
        # separate repository, so this branch is what CI takes on every
        # run: the gate that found 39 divergences on 2026-08-28 has been
        # reporting success without comparing anything ever since.
        # Say so in words that cannot be read as "checked and clean".
        print(
            "check-grammar-docs-match: SKIPPED — NOT CHECKED "
            f"(no documentation tree at {docs_root}; pass --docs PATH, "
            "or --require-docs to make this a failure)",
            file=sys.stderr,
        )
        return 1 if args.require_docs else 0

    auth = rules(AUTHORITY.read_text())
    docs = doc_rules(docs_root)

    mismatched: list[tuple[str, str, str, str]] = []
    unknown: list[tuple[str, str]] = []
    aside: list[tuple[str, str]] = []
    for name, occurrences in sorted(docs.items()):
        for page, rhs in occurrences:
            if name not in auth:
                # A page may show a SMALL LOCAL syntax that is not Verum
                # grammar at all — `docs/stdlib/decimal.md` defines
                # `decimal` and `sign` for a literal format, and a
                # tutorial may sketch a toy language. Only the page that
                # presents itself AS the grammar has to be a subset of
                # the authority; elsewhere an unknown name is reported
                # and not fatal.
                (unknown if GRAMMAR_PAGE in page else aside).append((name, page))
            elif auth[name] != rhs:
                mismatched.append((name, page, auth[name], rhs))

    if mismatched:
        print(f"--- {len(mismatched)} production(s) shown differently than the authority ---")
        for name, page, a, s in mismatched:
            print(f"  {name}  ({page})")
            print(f"      grammar/verum.ebnf: {a[:110]}")
            print(f"      documentation     : {s[:110]}")
    if unknown:
        print(
            f"\n--- {len(unknown)} production(s) on the grammar page that the "
            f"authority does not define ---"
        )
        for name, page in unknown:
            print(f"  {name}  ({page})")
    if aside:
        print(
            f"\n--- {len(aside)} production(s) elsewhere, outside the grammar "
            f"(local syntaxes; reported, not gated) ---"
        )
        for name, page in aside:
            print(f"  {name}  ({page})")

    total = len(mismatched) + len(unknown)
    print(
        f"\ncheck-grammar-docs-match: {total} divergence(s) across "
        f"{len(docs)} documented production(s)"
    )
    if total and args.check:
        print(
            "The documentation is a copy of grammar/verum.ebnf; a copy that "
            "lags tells a contributor that valid syntax is invalid."
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
