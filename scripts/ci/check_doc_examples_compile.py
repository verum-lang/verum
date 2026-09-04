#!/usr/bin/env python3
"""Gate: the website's self-contained examples still compile.

WHY THIS EXISTS.  The documentation is 371 pages and 2857 `verum` code
blocks, and a page goes stale silently: the language moves, the example
does not, and nothing says so until a reader copies it.  Measured
2026-09-04 on the first run: of 55 blocks that declare their own
`fn main`, 21 compiled and 34 did not.

WHAT IS CHECKED.  Only blocks that declare `fn main` — a block that does
not is a FRAGMENT by convention, and compiling one would report the
documentation's own style as a defect.  Even among the self-contained
ones, a block may lean on a helper the page defined earlier; those
surface as E100 / E101 / E402 (a name, a type or a module not found)
and are counted separately from failures INTERNAL to the example.

RATCHET, NOT GATE.  The known-failing set is recorded by page and line.
A new failure fails the run; a failure that disappears asks for the
baseline to be lowered in the same commit that earned it — a silently
improving number is how a gate stops measuring.

Usage:
    check_doc_examples_compile.py <verum-binary>            # ratchet
    check_doc_examples_compile.py <verum-binary> --list     # enumerate
    check_doc_examples_compile.py <verum-binary> --write-baseline
"""
from __future__ import annotations
import hashlib
import pathlib
import re
import subprocess
import sys
import tempfile

REPO = pathlib.Path(__file__).resolve().parents[2]
DOCS = REPO / "internal" / "website" / "docs"
BASELINE = REPO / "scripts" / "ci" / "doc_examples_known_failures.txt"
# A name the PAGE supplies elsewhere, not a defect in the example itself.
ELSEWHERE = ("error<E100>", "error<E101>", "error<E402>")


def blocks() -> list[tuple[str, str]]:
    """Every ```verum block that declares its own `fn main`, as (site, source)."""
    out = []
    for p in sorted(DOCS.rglob("*.md")):
        lines = p.read_text(errors="ignore").split("\n")
        buf, start = None, 0
        for i, line in enumerate(lines, 1):
            s = line.strip()
            if s.startswith("```verum"):
                buf, start = [], i
                continue
            if buf is not None and s.startswith("```"):
                body = "\n".join(buf)
                if re.search(r"^\s*(public\s+)?fn main\s*\(", body, re.M):
                    # Keyed by page + a hash of the BLOCK, not by line: an
                    # edit anywhere above shifts every line number below it,
                    # and a baseline that moves for unrelated reasons reports
                    # noise as regression. Editing the block itself DOES
                    # change the key, which is right — it is a new example.
                    h = hashlib.blake2s(body.encode(), digest_size=4).hexdigest()
                    out.append((f"{p.relative_to(DOCS)}#{h}", body))
                buf = None
                continue
            if buf is not None:
                buf.append(line)
    return out


def compile_one(verum: str, src: str, tmp: pathlib.Path) -> str:
    f = tmp / "ex.vr"
    f.write_text(src + "\n")
    try:
        r = subprocess.run([verum, "check", str(f)], capture_output=True,
                           text=True, timeout=120)
    except subprocess.TimeoutExpired:
        return "TIMEOUT"
    if r.returncode == 0:
        return ""
    m = re.search(r"error(<[^>]*>)?: .{0,70}", r.stdout + r.stderr)
    return m.group(0) if m else f"exit {r.returncode}"


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 2
    verum = sys.argv[1]
    if not pathlib.Path(verum).is_file():
        print(f"check_doc_examples: no verum binary at {verum}", file=sys.stderr)
        return 2
    write = "--write-baseline" in sys.argv
    listing = "--list" in sys.argv

    failures: dict[str, str] = {}
    with tempfile.TemporaryDirectory() as d:
        tmp = pathlib.Path(d)
        found = blocks()
        for site, src in found:
            err = compile_one(verum, src, tmp)
            if err:
                failures[site] = err

    frag = {k: v for k, v in failures.items() if v.startswith(ELSEWHERE)}
    real = {k: v for k, v in failures.items() if not v.startswith(ELSEWHERE)}
    total = len(found)
    print(f"doc examples: {total} self-contained, {total - len(failures)} compile, "
          f"{len(failures)} do not ({len(frag)} lean on the page, {len(real)} fail on their own)")

    if listing:
        for site, err in sorted(real.items()):
            print(f"  {site}\n      {err}")

    if write:
        BASELINE.write_text("".join(f"{s}\n" for s in sorted(failures)))
        print(f"[write] baseline: {len(failures)} known-failing example(s)")
        return 0

    known = set()
    if BASELINE.is_file():
        known = {l.strip() for l in BASELINE.read_text().split("\n") if l.strip()}
    new = sorted(set(failures) - known)
    fixed = sorted(known - set(failures))
    if new:
        print(f"\n[fail] {len(new)} doc example(s) newly fail to compile:")
        for s in new:
            print(f"    {s}\n        {failures[s]}")
        return 1
    if fixed:
        print(f"\n[fail] {len(fixed)} known-failing example(s) now COMPILE — lower the baseline")
        for s in fixed:
            print(f"    {s}")
        return 1
    print(f"[ok] doc examples: {len(failures)} known-failing, none new")
    return 0


if __name__ == "__main__":
    sys.exit(main())
