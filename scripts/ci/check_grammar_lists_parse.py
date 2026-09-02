#!/usr/bin/env python3
"""Gate: a production the grammar defines as a comma LIST parses with TWO items.

`grammar/verum.ebnf` defines 53 productions of the shape

    xs = x , { ',' , x } ;

and a parser can implement any of them as "read one x" without anything
noticing: one item is the common case, so the corpus keeps compiling and
the second item is simply refused when someone finally writes it.

Measured that way 2026-08-30: a FUNCTION contract read one expression at
three separate sites, so

    pure fn rank(a: Int, b: Int) -> Int
        requires a >= 0, b >= 0

was `E032: expected `{` to start function body` — blaming the body for a
comma — while the same clause on a THEOREM parsed. The grammar defines
the list once, for both (T0988).  FIXED and re-measured 2026-09-02:
both rows parse, and the two `T0988` markers are cleared — a gate that
keeps naming a closed defect teaches the next reader to expect the red.

WHAT THIS COVERS: 11 of the 53 productions, named below. Not a survey —
the sample is the clauses a program can exercise in a few lines. Adding
a row is the cheapest way to extend it.

A RED HERE IS NOT AUTOMATICALLY A DEFECT. Two of the three reds this
gate first produced were the probe's own fault: `where n > 0, n < 10` is
not grammatical (`value_where_clause` takes ONE expression; the list
lives in the braced `inline_refinement`, i.e. `where { n > 0, n < 10 }`).
Check a new red against the production's TERMINALS before filing it.

    scripts/ci/check_grammar_lists_parse.py [--verum PATH] [--keep]
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]

# (production, source using TWO items, expected-to-parse)
# `False` means a known defect, and the task that owns it.
CASES: list[tuple[str, str, bool, str]] = [
    ("derive_attribute", '@derive(Debug, Clone)\ntype P is { x: Int };\nfn main() { print("x"); }\n', True, ""),
    ("mount_list", 'mount core.text.{Text as T1, Text as T2};\nfn main() { print("x"); }\n', True, ""),
    ("extended_context_list",
     'context A { fn a(&self) -> Int; }\ncontext B { fn b(&self) -> Int; }\n'
     'fn f() -> Int using [A, B] { A.a() + B.b() }\nfn main() { print("x"); }\n', True, ""),
    ("refinement_predicates", 'type Small is n: Int where { n > 0, n < 10 };\nfn main() { print("x"); }\n', True, ""),
    ("refinement_predicates (named)",
     'type Small is n: Int where { lower: n > 0, upper: n < 10 };\nfn main() { print("x"); }\n', True, ""),
    ("where_predicates",
     'fn f<T, U>(a: T, b: U) -> Int where type T: Ord, type U: Ord { 1 }\nfn main() { print("x"); }\n', True, ""),
    ("generic_params", 'fn f<A, B>(a: A, b: B) -> Int { 1 }\nfn main() { print(f"{f(1, 2)}"); }\n', True, ""),
    ("expression_list", 'fn f(a: Int, b: Int) -> Int { a + b }\nfn main() { print(f"{f(1, 2)}"); }\n', True, ""),
    ("decreases",
     'fn f(m: Int, n: Int) -> Int decreases m, n { if m <= 0 { 0 } else { f(m - 1, n) } }\n'
     'fn main() { print(f"{f(2, 3)}"); }\n', True, ""),
    ("theorem requires_clause",
     'theorem t(a: Int, b: Int) requires a >= 0, b >= 0 ensures a + b >= 0 { proof by smt }\n'
     'fn main() { print("x"); }\n', True, ""),
    ("function requires_clause",
     'pure fn f(a: Int, b: Int) -> Int requires a >= 0, b >= 0 { a + b }\n'
     'fn main() { print(f"{f(1, 2)}"); }\n', True, ""),
    ("function ensures_clause",
     'pure fn f(a: Int, b: Int) -> Int ensures result >= 0, result >= a { a + b }\n'
     'fn main() { print(f"{f(1, 2)}"); }\n', True, ""),
]


def find_verum(explicit: str | None) -> Path | None:
    if explicit:
        p = Path(explicit)
        return p if p.is_file() else None
    for rel in ("target/release/verum", "target/debug/verum"):
        p = REPO / rel
        if p.is_file():
            return p
    return None


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--verum")
    ap.add_argument("--keep", action="store_true")
    args = ap.parse_args()

    verum = find_verum(args.verum)
    if verum is None:
        print(
            "no verum binary at target/{release,debug}/verum — pass one with "
            "--verum PATH. NOT CHECKED.",
            file=sys.stderr,
        )
        return 0

    root = Path(tempfile.mkdtemp(prefix="verum-lists-"))
    failures: list[str] = []
    unexpected_pass: list[str] = []
    ran = 0
    try:
        for name, source, should_parse, owner in CASES:
            src = root / (name.replace(" ", "_").replace("(", "").replace(")", "") + ".vr")
            src.write_text(source)
            proc = subprocess.run(
                [str(verum), "check", str(src)],
                capture_output=True, text=True, timeout=300,
            )
            ran += 1
            parsed = proc.returncode == 0
            if not parsed:
                first = next(
                    (ln for ln in (proc.stdout + proc.stderr).splitlines() if "error" in ln), ""
                )
                tag = f" [{owner}]" if owner else ""
                failures.append(f"  {name}{tag}\n      {first.strip()[:120]}")
            elif owner:
                # A row carrying a task id that now parses is NEWS: either
                # the task landed, or it never reproduced. Either way the
                # row must be re-measured rather than left as decoration.
                unexpected_pass.append(
                    f"  {name} — parses now, though {owner} is open; re-measure and clear the row"
                )
    finally:
        if not args.keep:
            shutil.rmtree(root, ignore_errors=True)

    if ran != len(CASES):
        print(f"REFUSING TO PASS: ran {ran} of {len(CASES)} cases", file=sys.stderr)
        return 2

    known = sum(1 for *_, owner in CASES if owner)
    print(
        f"grammar-list conformance: {ran} of 53 comma-list productions exercised "
        f"with two items ({known} with a known defect)"
    )
    if unexpected_pass:
        # A defect that stopped reproducing is news, not silence.
        print("\nnow parsing, though a task says otherwise:\n", file=sys.stderr)
        for u in unexpected_pass:
            print(u, file=sys.stderr)
    if not failures:
        print("every exercised list takes its second item")
        return 0
    print(f"\n{len(failures)} list(s) refuse a second item:\n", file=sys.stderr)
    for f in failures:
        print(f, file=sys.stderr)
    print(
        "\nCheck the production's TERMINALS in grammar/verum.ebnf before filing:\n"
        "two of this gate's first three reds were the probe's own bad syntax.",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
