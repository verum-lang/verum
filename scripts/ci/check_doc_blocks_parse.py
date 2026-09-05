#!/usr/bin/env python3
"""Gate: a ```verum block in the docs must be Verum the parser accepts.

`check_doc_examples.py` compiles the SELF-CONTAINED blocks — the ones
with `fn main`. Measured 2026-09-04 on `docs/language`: 550 blocks, 9
self-contained. **One per cent.** The other 541 are where syntax drift
lives, because nothing ever reads them.

This gate asks the weaker question of all of them: does it PARSE?

Three kinds of block are not defects and are excluded by construction,
because a gate that reports them is a gate nobody runs. Measured on a
stride sample of 62: 30 failed to parse as-is, and only SIX were real.

  ELISION   contains `...` — a deliberate hole. Cannot parse and is
            not meant to.
  TABLE     half or more of its lines carry `->`, `≡`, `⇒` or an
            aligned trailing comment. It is a rendering of a mapping,
            not code.
  FRAGMENT  parses once wrapped in `fn main() { … }` — an expression
            or statement shown without its frame.

What remains is a block that parses neither way and is not a table:
syntax the compiler does not have. `pattern Even(n: Int) = is_even(n)`
(no return type), `type File.Read is …` (a dotted declaration name),
`where forall i in 0..3. …` on a type.

    check_doc_blocks_parse.py            report
    check_doc_blocks_parse.py --check    exit 1 above the baseline
    check_doc_blocks_parse.py --self-test

BASELINE is a ratchet, not zero, and the difference is deliberate: the
survivors are real defects in the compiler as often as in the docs
(A65), so demanding zero would ask a doc author to fix a parser. The
ratchet stops the population growing while those are worked down.
"""
from __future__ import annotations

import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
DOCS = REPO / "internal" / "website" / "docs"
# The fence may carry an INFO STRING — ```verum title="…" — and it is
# still a Verum block. Measured 2026-09-05: adding a title to one
# block took a file from 9 blocks to 8, so the block left the census
# instead of being counted. The defect total would have fallen for
# the wrong reason. `verumx` is a different language and must not
# match, which is why the space is required.
BLOCK = re.compile(r"^```verum(?:[ \t][^\n]*)?\n(.*?)^```", re.M | re.S)
TABLE_GLYPH = re.compile(r"->|≡|⇒|→")
# NOT SET. The full census has not been run — a stride sample of 46 of
# `docs/language`'s 550 blocks classified 26 ok / 8 fragment / 6 elision
# / 6 DEFECT in 12 seconds, which extrapolates but does not measure.
# `--check` refuses to run until this is a number somebody counted:
# a ratchet on an estimate legitimises whatever the estimate was wrong
# about, in whichever direction it was wrong.
# Measured 2026-09-05 over the whole estate, 68 minutes: 2845 blocks —
# 1483 ok, 406 table, 383 elision, 345 fragment, 13 mixed,
# 2 counter-example, 213 DEFECT. A ratchet, not zero: the 213 are
# real and include parser defects (A65, A69, A70, A71) whose fix is
# not a documentation edit.
BASELINE = 213


def verum_binary() -> str:
    return os.environ.get("VERUM_BIN") or str(REPO / "target" / "debug" / "verum")


# A block that announces its own failure is a counter-example: the page
# is SHOWING what the compiler refuses. Measured 2026-09-04: 39 such
# blocks in 25 files. Reporting them as defects inverts their purpose —
# the day one of them starts compiling, the PAGE is wrong, not the
# block, and that is a different check.
COUNTER_EXAMPLE = re.compile(
    r"//[^\n]*\b(COMPILE ERROR|does not compile|not yet supported|"
    r"is refused|is rejected|will not compile)\b", re.I)


def is_counter_example(body: str) -> bool:
    return COUNTER_EXAMPLE.search(body) is not None


def is_elision(body: str) -> bool:
    return "..." in body or "/* body */" in body or "…" in body


# A trailing comment sitting in the same column on most lines is a
# rendering of a mapping, the same as an arrow glyph. Measured on
# `ffi.md` block 4 — four pointer spellings each with an aligned `//`
# gloss, reported as a DEFECT because no compilation unit accepts a
# bare type expression.
ALIGNED_COMMENT = re.compile(r"^\S.*?\s{2,}//")

# A reference page lists an API as bare signatures — no `fn`, often
# several to a line separated by `/` or column alignment. Measured
# 2026-09-04: 385 such lines across 50 files, 94 in `intrinsics.md`
# alone. It is the house style for the stdlib reference, not 385
# typos, and no compilation unit accepts it.
BARE_SIGNATURE = re.compile(
    r"^\s*[a-z_][A-Za-z0-9_]*(<[^>]*>)?\([^)]*\)\s*(->.*)?$")
# The same house style with a receiver: `c.is_ascii()  c.is_digit()`,
# several method calls to a line and no statement terminator. Measured
# on `text.md` block 17 — nine such lines, no `;` anywhere.
METHOD_RUN = re.compile(
    r"^\s*[a-z_][A-Za-z0-9_]*\.[a-z_][A-Za-z0-9_]*\([^)]*\)"
    r"(\s{2,}[a-z_][A-Za-z0-9_]*\.[a-z_][A-Za-z0-9_]*\([^)]*\))+\s*$")
SIGNATURE_RUN = re.compile(
    r"^\s*[a-z_][A-Za-z0-9_]*(<[^>]*>)?\s*(\([^)]*\))?"
    r"(\s*(/|\s{2,})\s*[a-z_][A-Za-z0-9_]*(<[^>]*>)?\s*(\([^)]*\))?.*)+$")


def is_table(body: str) -> bool:
    lines = [l for l in body.split("\n") if l.strip()]
    if not lines:
        return True
    marked = sum(1 for l in lines
                 if TABLE_GLYPH.search(l)
                 or ALIGNED_COMMENT.match(l)
                 or BARE_SIGNATURE.match(l)
                 or SIGNATURE_RUN.match(l)
                 or METHOD_RUN.match(l)
                 or l.strip().startswith("//"))
    return marked * 2 >= len(lines)


def parses(binary: str, text: str, tmp: str, tag: str) -> bool:
    f = Path(tmp) / f"{tag}.vr"
    f.write_text(text + "\n")
    try:
        p = subprocess.run([binary, "check", str(f)], capture_output=True,
                           text=True, timeout=120)
    except subprocess.TimeoutExpired:
        return True  # a timeout is not a parse defect; do not report it as one
    return "Parse error" not in (p.stdout + p.stderr)


def classify(binary: str, body: str, tmp: str, tag: str) -> str:
    if is_elision(body):
        return "elision"
    if parses(binary, body, tmp, tag):
        return "ok"
    # A counter-example is a block that ANNOUNCES its own failure and
    # then fails. Both halves are required: measured 2026-09-04, ten
    # blocks carry the marker and EIGHT of them parse — the marker is
    # usually about a diagnostic the example DISCUSSES, not about the
    # block itself, so the marker alone would have absorbed eight
    # working examples.
    if is_counter_example(body):
        return "counter-example"
    if is_table(body):
        return "table"
    if parses(binary, "fn zz_wrap() {\n" + body + "\n}", tmp, tag + "w"):
        return "fragment"
    # MIXED: top-level items AND a bare expression in one block, which
    # neither test can accept — the expression is illegal at top level,
    # and the items are illegal inside the wrapper. Measured on
    # `active-patterns.md` block 10: `pattern` declarations followed by
    # a bare `match`, a shape a reader understands and no single
    # compilation unit does. Splitting the last statement off and
    # wrapping only that answers the question the block poses.
    lines = body.split("\n")
    for cut in range(len(lines) - 1, 0, -1):
        head, tail = "\n".join(lines[:cut]), "\n".join(lines[cut:])
        if not tail.strip():
            continue
        if parses(binary, head, tmp, tag + "h") and parses(
            binary, head + "\nfn zz_wrap() {\n" + tail + "\n}", tmp, tag + "m"
        ):
            return "mixed"
    return "DEFECT"


def run(argv: list[str]) -> int:
    if "--self-test" in argv:
        return self_test()
    if not DOCS.exists():
        print("check-doc-blocks-parse: docs not present, skipped")
        return 0
    binary = verum_binary()
    if not Path(binary).exists():
        print(f"check-doc-blocks-parse: no verum binary at {binary}, skipped")
        return 0

    counts = {"ok": 0, "counter-example": 0, "elision": 0, "table": 0,
              "fragment": 0, "mixed": 0, "DEFECT": 0}
    defects: list[str] = []
    # An optional subtree, so a section can be measured in ten minutes
    # instead of the whole estate in an hour. The BASELINE only means
    # anything for a full run, and --check refuses a partial one.
    sub = [a for a in argv if not a.startswith("--")]
    root = DOCS / sub[0] if sub else DOCS
    if not root.exists():
        print(f"check-doc-blocks-parse: {root} does not exist", file=sys.stderr)
        return 1
    mds = [root] if root.is_file() else sorted(root.rglob("*.md"))
    todo = [(md, i, b) for md in mds
            for i, b in enumerate(BLOCK.findall(md.read_text()))]
    # A run that prints nothing for forty minutes cannot be told from a
    # run that has hung, and I killed one to find out which it was.
    print(f"check-doc-blocks-parse: {len(todo)} blocks, up to two checks each",
          flush=True)
    with tempfile.TemporaryDirectory() as tmp:
        for n, (md, i, body) in enumerate(todo, 1):
            tag = f"{md.stem}_{i}"
            k = classify(binary, body.strip(), tmp, tag)
            counts[k] += 1
            if k == "DEFECT":
                defects.append(f"{md.relative_to(DOCS)} block {i}")
            if n % 100 == 0:
                print(f"  {n}/{len(todo)}  {counts['DEFECT']} defect(s) so far",
                      flush=True)

    total = sum(counts.values())
    print(f"check-doc-blocks-parse: {total} block(s) — "
          + ", ".join(f"{v} {k}" for k, v in counts.items()))
    # A per-directory tally first: 213 lines of block references say
    # nothing about where the work is, and a truncated list says less.
    from collections import Counter
    by_dir = Counter(d.split("/")[0] if "/" in d else "(root)" for d in defects)
    for name, n in by_dir.most_common():
        print(f"  {n:>4}  {name}")
    # Print them ALL. A truncated list is a list you cannot work from,
    # and the count at the top already says how many there are: measured
    # 2026-09-05, the cap of 20 hid 174 of 194 references and I spent a
    # separate run re-deriving what this line already knew.
    limit = len(defects) if "--all" in sys.argv or len(defects) <= 200 else 200
    for d in defects[:limit]:
        print(f"      {d}")
    if len(defects) > limit:
        print(f"      … and {len(defects) - limit} more — pass --all")
    if "--check" in argv:
        if sub:
            print("--check needs the whole estate; a subtree cannot be "
                  "compared against a whole-estate baseline.", file=sys.stderr)
            return 1
        if BASELINE is None:
            print("--check needs a BASELINE somebody has counted; run without it "
                  "to get the figure, then set it.", file=sys.stderr)
            return 1
        if counts["DEFECT"] > BASELINE:
            print(f"{counts['DEFECT']} blocks the parser refuses, baseline {BASELINE}.",
                  file=sys.stderr)
            return 1
    return 0


def self_test() -> int:
    """Both polarities on every classifier that runs without a binary."""
    cases = [
        (is_elision, "fn f() { ... }", True),
        (is_elision, "fn f() { 1 }", False),
        (is_table, "a -> b\nc -> d", True),
        (is_table, "fn f() {\n    let x = 1;\n    x\n}", False),
        (is_table, "", True),
    ]
    bad = 0
    for fn, arg, want in cases:
        got = fn(arg)
        if got != want:
            bad += 1
            print(f"  SELF-TEST FAIL: {fn.__name__}({arg[:24]!r}) = {got}, want {want}")
    print(f"check-doc-blocks-parse --self-test: {len(cases) - bad}/{len(cases)} pass")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(run(sys.argv[1:]))
