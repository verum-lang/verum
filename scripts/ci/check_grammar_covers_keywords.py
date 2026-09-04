#!/usr/bin/env python3
"""Gate: every keyword the lexer accepts must appear in the grammar.

`grammar/verum.ebnf` is declared the ONLY source of truth for Verum
syntax, and CLAUDE.md tells every contributor to check syntax against it
before writing a `.vr` file. That instruction is only sound if the file
actually contains the syntax the parser accepts.

IT DID NOT. Six keywords were live in the lexer, reached by the parser,
accepted by a release compiler, and absent from this file:

    layer        a named group of `provide` bindings, plus `A + B`
                 composition — an ITEM, not a statement
    link         an accepted synonym for `mount`
    inductive    `type Nat is inductive { | Zero | Succ(Nat) };`
    coinductive  `type Str<A> is coinductive { fn head(&self) -> A; };`
    inject       `inject Log` — the context INSTANCE as a value
    implies      a specification connective, with `<->`

Two of them were cited BY the parser against productions that did not
exist: `decl.rs` carries `Spec: grammar/verum.ebnf - inductive_def` and
`- coinductive_def`.

WHY THIS IS WORSE THAN A MISSING PARAGRAPH: a reader consulting the
authority concludes that valid syntax is invented. That is not
hypothetical — it happened during the session that wrote this gate. The
`layer` blocks in the context documentation were nearly deleted as
fiction because the grammar did not mention the keyword; a compile
probe, not the grammar, is what established they were real.

The direction matters. This checks LEXER -> GRAMMAR: syntax the
compiler accepts and the grammar omits. The reverse (grammar names a
keyword the lexer lacks) is a different defect and this gate does not
look for it — it would need a parse, not a grep.

Usage:
    check_grammar_covers_keywords.py            # report
    check_grammar_covers_keywords.py --check    # exit 1 if any missing
    check_grammar_covers_keywords.py --self-test
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
LEXER = REPO / "crates" / "verum_lexer" / "src" / "token.rs"
EBNF = REPO / "grammar" / "verum.ebnf"

# `#[token("word")]` — word-shaped only. Symbol tokens (`->`, `<->`) are
# spelled many ways in a grammar and matching them by string produces
# noise; keywords are unambiguous.
TOKEN = re.compile(r'#\[token\("([a-z_][a-z_0-9]*)"\)\]')

# A terminal in this grammar is single-quoted: 'mount', 'is', 'layer'.
TERMINAL = re.compile(r"'([a-z_][a-z_0-9]*)'")


def keywords() -> set[str]:
    return set(TOKEN.findall(LEXER.read_text()))


def terminals() -> set[str]:
    return set(TERMINAL.findall(EBNF.read_text()))


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--docs", type=Path, default=None,
                    help="also require every keyword to be NAMED on this page "
                         "(the keyword reference); 14 were missing when this was added, "
                         "`using` and `link` among them")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()

    if args.self_test:
        # The gate's own extraction, against text with a known answer —
        # including the case that motivated it (a keyword present in the
        # lexer and absent from the grammar) and its opposite, so a
        # regex that matched everything or nothing would be caught.
        ok = True
        got_kw = set(TOKEN.findall('#[token("layer")]\nLayer,\n#[token("mount")]'))
        if got_kw != {"layer", "mount"}:
            print(f"self-test FAIL: keywords -> {got_kw}")
            ok = False
        got_t = set(TERMINAL.findall("x = 'mount' , y ;"))
        if got_t != {"mount"}:
            print(f"self-test FAIL: terminals -> {got_t}")
            ok = False
        if got_kw - got_t != {"layer"}:
            print("self-test FAIL: the difference is not the missing keyword")
            ok = False
        # A symbol token must NOT be collected: it would report forever.
        if TOKEN.findall('#[token("<->")]'):
            print("self-test FAIL: symbol token collected as a keyword")
            ok = False
        print("self-test: ok" if ok else "self-test: FAILED")
        return 0 if ok else 1

    kws, terms = keywords(), terminals()
    missing = sorted(kws - terms)

    for m in missing:
        print(f"  {m}")
    print(
        f"\ncheck-grammar-covers-keywords: {len(missing)} of {len(kws)} lexer "
        f"keywords are absent from grammar/verum.ebnf"
    )

    # Same question, second target. A keyword reference that omits a
    # keyword is the documentation form of the defect this gate exists
    # for: `using` — the word the whole context system turns on — was
    # absent from the page that calls itself the full list, along with
    # `link`, `layer`, `volatile` and the ten tactic names.
    #
    # A WORD-SCAN, not a code-fence scan: the page lists most keywords
    # inside plain fences and a few in prose, and a fence-only reader
    # reported 57 absent when the true answer was 14. The question here
    # is only "is the word on the page at all", which is the weakest
    # claim worth gating and the one with no false positives.
    doc_missing: list[str] = []
    if args.docs is not None:
        if not args.docs.is_file():
            print(f"--docs {args.docs}: not a file — NOT CHECKED")
        else:
            words = set(re.findall(r"\b([a-z_][a-z_0-9]*)\b", args.docs.read_text()))
            doc_missing = sorted(kws - words)
            for m in doc_missing:
                print(f"  {m}  — not named on {args.docs.name}")
            print(
                f"check-keywords-documented: {len(doc_missing)} of {len(kws)} "
                f"lexer keywords are not named on {args.docs.name}"
            )

    if args.check and (missing or doc_missing):
        if missing:
            print(
                "The grammar is the only source of truth for Verum syntax; a "
                "keyword it omits reads to a contributor as invalid syntax."
            )
        if doc_missing:
            print(
                "A keyword absent from the keyword reference is a word a "
                "reader cannot look up and will not use."
            )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
