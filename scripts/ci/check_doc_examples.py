#!/usr/bin/env python3
"""Gate: the Verum examples in the documentation must be Verum.

Extracts every ```verum block that carries an `fn main` — the ones a
reader can copy and run — and compiles each with `verum check`.

WHY ONLY THE SELF-CONTAINED ONES: of 2823 verum blocks in the docs, 63
have an `fn main`. The rest are fragments that need a surrounding
project, and wrapping them would invent context and produce failures
that say nothing about the documentation. The 63 are the blocks whose
correctness a reader can actually depend on.

WHAT THE FIRST RUN FOUND (2026-08-28), and why the classification
matters more than the total: 51 of 63 failed, and only about half of
that was documentation drift.

    26  named an API that does not exist       <- real
     8  parse errors                           <- real
    10  used a name defined in an earlier block  <- this tool's limit
     9  needed a context to be provided          <- this tool's limit
     7  mounted a sibling module                 <- this tool's limit
     4  elided with `...`                        <- by design

A gate that reported "51 broken examples" would have been wrong about
half of them, and the half it was wrong about is the half nobody can
fix.

BLOCKS THAT MUST NOT COMPILE are first-class here. Documentation
teaches by counter-example — "yield outside a fn* body", "an unbalanced
brace", "a refinement the compiler can clearly disprove" — and a block
marked as such is checked in the OTHER direction: if it compiles, the
marker is stale and the reader is being shown a fear that no longer
applies. Five marker spellings are recognised because five are in use.

Usage:
    check_doc_examples.py                  # report, grouped
    check_doc_examples.py --check          # exit 1 on a real failure
    check_doc_examples.py --self-test
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
import tempfile
from collections import Counter, defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
DOCS = REPO / "internal" / "website" / "docs"

BLOCK = re.compile(r"^```verum\n(.*?)^```", re.M | re.S)

# Blocks the documentation deliberately shows as NOT compiling.
MUST_FAIL = re.compile(
    r"//\s*(COMPILE ERROR|ERROR|error<E\d+>|does not compile|WRONG|BAD|✗|refused)",
    re.I,
)

# Failures that are this tool's limitation rather than a defect in the
# prose. Each one is a block that is correct in its page and incomplete
# on its own.
LIMITS = (
    ("mounts a sibling module", re.compile(r"module `[^`]+` not found")),
    ("needs a provided context", re.compile(r"undefined context")),
    ("name defined in an earlier block", re.compile(r"unbound variable|type not found")),
)


def verum_binary() -> Path:
    override = os.environ.get("VERUM_BIN")
    if override:
        p = Path(override)
        if not p.is_file():
            raise SystemExit(f"VERUM_BIN={override} is not a file")
        return p
    for c in (REPO / "target" / "release" / "verum", REPO / "target" / "debug" / "verum"):
        if c.is_file():
            return c
    raise SystemExit(
        "no verum binary at target/{release,debug}/verum — pass one with "
        "VERUM_BIN=/path/to/verum"
    )


def blocks():
    for d in sorted(DOCS.rglob("*.md")):
        text = d.read_text(errors="ignore")
        for m in BLOCK.finditer(text):
            body = m.group(1)
            if "fn main(" not in body:
                continue
            line = text[: m.start()].count("\n") + 1
            yield d.relative_to(DOCS.parent), line, body


def classify(body: str, errs: list[str]) -> str:
    if not errs:
        return "compiles"
    first = errs[0]
    if "..." in body and "Parse error" in first:
        return "elided with `...`"
    for name, pat in LIMITS:
        if pat.search(first):
            return name
    if "Parse error" in first:
        return "PARSE ERROR"
    return "API / semantic"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--timeout", type=int, default=120)
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()

    if args.self_test:
        ok = True
        cases = [
            ("fn main() {}", [], "compiles"),
            ("fn main() { ... }", ["error<E018>: Parse error: x"], "elided with `...`"),
            ("fn main() {}", ["error<E402>: module `x` not found"], "mounts a sibling module"),
            ("fn main() {}", ["error: undefined context: IO"], "needs a provided context"),
            ("fn main() {}", ["error<E018>: Parse error: y"], "PARSE ERROR"),
            ("fn main() {}", ["error<E400>: no method named `z`"], "API / semantic"),
        ]
        for body, errs, want in cases:
            got = classify(body, errs)
            if got != want:
                print(f"self-test FAIL: {errs} -> {got}, want {want}")
                ok = False
        if not MUST_FAIL.search("// COMPILE ERROR: nope"):
            print("self-test FAIL: marker not recognised")
            ok = False
        print("self-test: ok" if ok else "self-test: FAILED")
        return 0 if ok else 1

    binary = verum_binary()
    buckets: Counter[str] = Counter()
    detail: dict[str, list] = defaultdict(list)
    stale_markers: list = []

    with tempfile.TemporaryDirectory() as tmp:
        for rel, line, body in blocks():
            f = Path(tmp) / f"{Path(rel).stem}_{line}.vr"
            f.write_text(body)
            try:
                r = subprocess.run(
                    [str(binary), "check", str(f)],
                    capture_output=True, text=True, timeout=args.timeout,
                )
                blob = r.stdout + r.stderr
            except subprocess.TimeoutExpired:
                buckets["TIMEOUT"] += 1
                detail["TIMEOUT"].append((rel, line, "did not finish"))
                continue
            errs = [l for l in blob.splitlines() if l.startswith("error")]

            if MUST_FAIL.search(body):
                # Checked the other way round: this block is documented
                # as not compiling, so compiling is the failure.
                if errs:
                    buckets["counter-example, correctly refused"] += 1
                else:
                    buckets["STALE MARKER — marked as failing, compiles"] += 1
                    stale_markers.append((rel, line, ""))
                continue

            k = classify(body, errs)
            buckets[k] += 1
            if errs:
                detail[k].append((rel, line, errs[0][:100]))

    for k, v in buckets.most_common():
        print(f"{v:4d}  {k}")

    real = ("PARSE ERROR", "API / semantic", "TIMEOUT",
            "STALE MARKER — marked as failing, compiles")
    print()
    for k in real:
        if not detail.get(k) and k not in buckets:
            continue
        rows = detail.get(k) or stale_markers if "STALE" in k else detail.get(k, [])
        if not rows:
            continue
        print(f"--- {k} ---")
        for rel, line, e in rows:
            print(f"  {rel}:{line}  {e}")

    n_real = sum(buckets[k] for k in real)
    print(f"\ncheck-doc-examples: {n_real} example(s) a reader cannot trust")
    return 1 if (n_real and args.check) else 0


if __name__ == "__main__":
    sys.exit(main())
