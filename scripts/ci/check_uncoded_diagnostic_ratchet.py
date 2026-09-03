#!/usr/bin/env python3
"""UNCODED-DIAGNOSTIC-RATCHET-1 (T1073): fail on any NET INCREASE of
`TypeError::Other(` sites in PRODUCTION code.

A `TypeError::Other(text)` prints as

    error: <text>
      --> <no source location attached to this diagnostic>

with no `error<CODE>` bracket and no span.  Three consequences, each
measured 2026-09-03:

  1. The user gets no code to look up and no line to go to.  The public
     docs promised, for one of these, "a diagnostic pointing at the
     offending property"; what printed was the no-location line.  In
     that case `span` was already a parameter of the emitting function
     and simply dropped (fixed, E428).

  2. Every counting instrument keyed on the rendered form — `grep -cE
     '^error<'`, which is what ad-hoc sweeps and this session's probe
     scripts used — MISSES them entirely.  Counts taken that way are
     lower bounds, silently.  The `compilation failed with N error(s)`
     summary line does see them, which is why the two numbers disagree
     on exactly these files.

  3. `verum_error`'s `registry_covers_every_emitted_code` test cannot
     cover them: there is no code to cover.  The registry gate is
     therefore green while ~85% of this family is outside it.

Counting rules:
  * `TypeError::Other(` only.  `OtherWithCode` and `OtherWithCodeSpanned`
    are the fixed forms and are NOT counted.
  * tests and benches are excluded — a diagnostic constructed in a test
    is not user-facing.
  * COMMENTS are excluded, for the reason the panic-surface ratchet
    gives: a gate a comment can redden teaches people not to write the
    comment.

Baseline: scripts/ci/uncoded_diagnostic_baseline.txt (a single integer).
  count >  baseline → FAIL, naming the worst files;
  count == baseline → OK;
  count <  baseline → OK + a reminder to ratchet the baseline down.

The fix for any one site is mechanical: pick or add a registry code and
switch to `TypeError::OtherWithCodeSpanned { code, msg, span }`, passing
the span the site already has.  Where it genuinely has none,
`OtherWithCode` still gives the reader a code.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
TARGETS = [ROOT / "crates"]
BASELINE_FILE = pathlib.Path(__file__).resolve().parent / "uncoded_diagnostic_baseline.txt"

UNCODED_RE = re.compile(r"TypeError::Other\(")


def strip_comments(text: str) -> str:
    """Blank out // line comments and /* */ block comments.

    Same treatment as the panic-surface ratchet: a line that NAMES the
    pattern while explaining it must not count as an instance.
    """
    out = []
    i = 0
    n = len(text)
    in_block = False
    while i < n:
        if in_block:
            if text.startswith("*/", i):
                in_block = False
                i += 2
            else:
                out.append("\n" if text[i] == "\n" else " ")
                i += 1
            continue
        if text.startswith("/*", i):
            in_block = True
            i += 2
            continue
        if text.startswith("//", i):
            while i < n and text[i] != "\n":
                out.append(" ")
                i += 1
            continue
        out.append(text[i])
        i += 1
    return "".join(out)


def is_excluded(path: pathlib.Path) -> bool:
    parts = path.parts
    return "tests" in parts or "benches" in parts or "examples" in parts


def main() -> int:
    per_file: dict[str, int] = {}
    total = 0
    for target in TARGETS:
        for path in target.rglob("*.rs"):
            if is_excluded(path):
                continue
            try:
                text = path.read_text(encoding="utf-8", errors="replace")
            except OSError:
                continue
            hits = len(UNCODED_RE.findall(strip_comments(text)))
            if hits:
                rel = str(path.relative_to(ROOT))
                per_file[rel] = hits
                total += hits

    if not BASELINE_FILE.exists():
        print(f"uncoded-diagnostic-ratchet: no baseline file at {BASELINE_FILE}")
        print(f"  current count is {total}; write it there to arm the gate")
        return 1
    baseline = int(BASELINE_FILE.read_text().strip())

    if total > baseline:
        print(
            f"GATE FAIL: uncoded-diagnostic-ratchet: {total} `TypeError::Other(` "
            f"sites > baseline {baseline}"
        )
        print("  A diagnostic without a code prints no `error<CODE>` bracket and")
        print("  no source location, and every `^error<`-keyed counter misses it.")
        print("  Use `TypeError::OtherWithCodeSpanned { code, msg, span }`.")
        # A CENSUS, not a delta: these are the largest holders, not the
        # files that caused the increase. A peer misread exactly this
        # shape in a sibling ratchet — the first line after the verdict
        # was the alphabetically smallest name, and it read as the new
        # entry. Say which it is.
        print("  Largest holders (a CENSUS, not the increase — the new site")
        print("  may be in any file):")
        for rel, hits in sorted(per_file.items(), key=lambda kv: -kv[1])[:10]:
            print(f"    {hits:4d}  {rel}")
        return 1

    if total < baseline:
        print(f"uncoded-diagnostic-ratchet: OK ({total} < baseline {baseline})")
        print(f"  ratchet down: write {total} into {BASELINE_FILE.name}")
        return 0

    print(f"uncoded-diagnostic-ratchet: OK ({total} == baseline)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
