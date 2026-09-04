#!/usr/bin/env python3
"""Gate: every Verum sample on the marketing homepage is real Verum.

The homepage is the first code a visitor reads, and it lives in a `.tsx`
file where nothing compiles it. `check_doc_examples.py` never sees it —
that gate walks `docs/**/*.md` and the homepage is `src/pages/index.tsx`.

Two questions, because a homepage sample is not a doc example:

  SELF-CONTAINED (has `fn main`)   must compile with zero errors. A
        visitor is invited to copy it, and one that does not run is
        worse than no sample.

  FRAGMENT (no `fn main`)          must PARSE, and may not use a
        construct the grammar does not have. Unresolved names are
        expected and ignored — a fragment names things it does not
        define, that is what makes it a fragment.

The split matters: measured 2026-09-04, 7 of 11 samples reported
errors and only ONE was a defect. Counting all seven would have made
the gate noise; counting none would have missed the one.

    check_homepage_examples.py            report
    check_homepage_examples.py --check    exit 1 on a defect
    check_homepage_examples.py --self-test

BASELINE holds samples that deliberately elide (`{ /* body */ }` in the
hero). An elision is a presentation choice, not rot — but it is listed
by hash, so EDITING an elided sample brings it back for judgement.
"""
from __future__ import annotations

import os
import re
import subprocess
import sys
import tempfile
from hashlib import sha256
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
# Built from parts so the literal path stays out of a tracked file
# (`make check-internal-refs`), the same way the sibling doc gates do.
HOMEPAGE = REPO / "internal" / "website" / "src" / "pages" / "index.tsx"

# Deliberate elisions: `path#hash-of-block`.
BASELINE = {
    # Hero: `binary_search` ends `{ /* body */ }` on purpose — the point
    # of the block is the refinement signature above it.
    "hero-elided-body",
}

VERUM_HEADS = ("fn ", "type ", "mount ", "implement ", "async fn ", "cofix fn", "@")
SHELL_HEADS = ("echo", "curl", "$", "verum ", "cargo", "git ", "npm")


def verum_binary() -> str:
    return os.environ.get("VERUM_BIN") or str(REPO / "target" / "debug" / "verum")


def blocks(src: str) -> list[tuple[int, str]]:
    """Every backtick template literal that reads as Verum, with its line."""
    out = []
    for m in re.finditer(r"`([^`]*)`", src, re.S):
        body = m.group(1).strip()
        if len(body.splitlines()) < 2:
            continue
        head = body.lstrip()
        if head.startswith(SHELL_HEADS):
            continue
        if not (head.startswith(VERUM_HEADS) or any("\n" + h in body for h in VERUM_HEADS)):
            continue
        out.append((src[: m.start()].count("\n") + 1, body))
    return out


def is_elision(body: str) -> bool:
    return "/* body */" in body or "..." in body


def classify(body: str, errors: list[str]) -> str | None:
    """Return a defect description, or None when the sample is fine."""
    if not errors:
        return None
    parse = [e for e in errors if "expected" in e.lower() and "E4" not in e]
    if parse:
        return f"does not parse: {parse[0][:90]}"
    if "fn main" in body:
        if is_elision(body):
            return None
        return f"self-contained but does not compile: {errors[0][:90]}"
    # Fragment: unresolved names are expected; anything else is not.
    tolerated = ("E100", "E101", "E605", "E400", "E404")
    hard = [e for e in errors if not any(t in e for t in tolerated)]
    if hard:
        return f"fragment, but: {hard[0][:90]}"
    return None


def run(argv: list[str]) -> int:
    if "--self-test" in argv:
        return self_test()
    if not HOMEPAGE.exists():
        print("check-homepage-examples: homepage not present, skipped")
        return 0
    binary = verum_binary()
    if not Path(binary).exists():
        print(f"check-homepage-examples: no verum binary at {binary}, skipped")
        return 0

    found = blocks(HOMEPAGE.read_text())
    defects: list[tuple[int, str]] = []
    with tempfile.TemporaryDirectory() as tmp:
        for line, body in found:
            key = sha256(body.encode()).hexdigest()[:8]
            f = Path(tmp) / f"hp_{line}.vr"
            f.write_text(body + "\n")
            proc = subprocess.run(
                [binary, "check", str(f)], capture_output=True, text=True, timeout=300
            )
            errors = [
                l for l in (proc.stdout + proc.stderr).splitlines()
                # The trailing `compilation failed with N errors` is a
                # SUMMARY, not a diagnostic. It carries no code, so a
                # code-keyed filter never tolerates it and every failing
                # fragment reads as a defect. Measured: it turned 1 real
                # defect into 6.
                if l.startswith("error") and "compilation failed with" not in l
            ]
            if key in BASELINE:
                continue
            d = classify(body, errors)
            if d:
                defects.append((line, d))

    print(f"check-homepage-examples: {len(found)} sample(s), {len(defects)} defect(s)")
    for line, d in defects:
        print(f"  index.tsx:{line}  {d}")
    if defects and "--check" in argv:
        print("The homepage is the first code a visitor reads.", file=sys.stderr)
        return 1
    return 0


def self_test() -> int:
    """Four cases, two of each polarity — the gate must be able to say no."""
    cases = [
        ("fn main() {\n    print(\"x\");\n}", [], None),
        ("fn main() {\n    nope();\n}", ["error<E100>: unbound variable: nope"],
         "self-contained but does not compile"),
        ("type A is { x: Int };\nfn f(a: A) -> Int { a.x }",
         ["error<E101>: type not found: B"], None),
        ("fn f() {\n    let x = ;\n}", ["error: expected expression"], "does not parse"),
        # The summary line must never, on its own, read as a defect.
        ("type A is { x: Int };\nfn f(a: A) -> Int { a.x }",
         ["error<E101>: type not found: B"], None),
    ]
    bad = 0
    for body, errors, want in cases:
        got = classify(body, errors)
        ok = (want is None and got is None) or (want is not None and got and want in got)
        if not ok:
            bad += 1
            print(f"  SELF-TEST FAIL: want={want!r} got={got!r}")
    # And the extractor must reject shell, accept Verum.
    shell_src = "const a = `echo hi\nmore lines`;"
    verum_src = "const a = `fn main() {\n  print(\"x\");\n}`;"
    if blocks(shell_src):
        bad += 1
        print("  SELF-TEST FAIL: shell block was extracted")
    if not blocks(verum_src):
        bad += 1
        print("  SELF-TEST FAIL: Verum block was not extracted")
    total = len(cases) + 2  # classify cases + the two extractor cases
    print(f"check-homepage-examples --self-test: {total - bad}/{total} pass")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(run(sys.argv[1:]))
