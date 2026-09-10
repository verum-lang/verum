#!/usr/bin/env python3
"""GATE-VERDICT-QUANTITY-1 (T1073): a gate that says OK must say WHAT IT
MEASURED.

`[ok]` on its own is indistinguishable from an `[ok]` printed before the
check ran. That is not hypothetical — measured 2026-09-03:

    check_summary_line_is_one_spelling.sh printed
      [ok] summary-line: one producer ...
    while every one of its `mapfile` calls had failed (bash 4 builtin,
    absent from macOS's bash 3.2). A green verdict from a script that
    never executed its own check.

This is the worst shape in the inertness taxonomy this session and
verum-2b built: not silence, and not a wrong answer, but a CONTROL THAT
SPEAKS IN FAVOUR because the thing it measures with is broken. The
discriminator is whether the control could, in principle, have returned
"worse" — and a hardcoded string never can.

A number in the verdict is the cheapest evidence that the code path ran:
it comes from the scan, so it cannot be printed by a script that failed
before scanning.

The rule is deliberately weak — ANY interpolation counts. This gate is
not trying to judge whether the quantity is the RIGHT one; it refuses
only the verdict that carries none at all.

The quantity is looked for in the whole verdict STATEMENT, not on the
line the verdict marker happens to sit on. Measured 2026-09-10, this
gate painted a correct one:

    check_doc_meta_functions.py:269
      print(f"[ok] self-test: 3 detector cases + 3 roster loads "
            f"({len(accepted)} accepted names)")

Both lines are one implicitly-concatenated f-string; the interpolation
is simply on the second. A line-at-a-time reader is narrower than the
grammar it judges, and a FALSE POSITIVE here is the expensive kind: it
tells an author to damage a verdict that already carries its number.

The statement is bounded by parenthesis depth, capped at
MAX_CONTINUATION lines. The cap exists because this reader does not
know string literals from code: an unmatched `(` inside a message
would otherwise run the window down the file and find somebody else's
interpolation. Five lines is longer than every verdict in the tree.

Known exemptions live in `gate_verdict_quantity_allowlist.txt`, one
filename per line with a reason after `#`, for gates whose subject
genuinely has no cardinality.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
CI = ROOT / "scripts" / "ci"
ALLOWLIST = CI / "gate_verdict_quantity_allowlist.txt"

VERDICT = re.compile(r"(\[ok\]|:\s*OK\b|GATE OK)")
QUANTITY = re.compile(r"\$\{?[A-Za-z_(]|\{[^{}\n]+\}|%[sd]")
MAX_CONTINUATION = 5


def verdict_statements(text: str) -> list[list[str]]:
    """Every verdict line, with the rest of ITS statement attached.

    Returns a list of windows; window[0] is the line carrying the
    verdict marker, and is what an offender report quotes.
    """
    lines = text.split("\n")
    out = []
    for i, line in enumerate(lines):
        if not VERDICT.search(line) or line.strip().startswith(("#", "//")):
            continue
        window = [line]
        depth = line.count("(") - line.count(")")
        j = i
        while depth > 0 and len(window) <= MAX_CONTINUATION and j + 1 < len(lines):
            j += 1
            if not lines[j].strip():
                break
            window.append(lines[j])
            depth += lines[j].count("(") - lines[j].count(")")
        out.append(window)
    return out


def load_allowlist() -> set[str]:
    if not ALLOWLIST.exists():
        return set()
    out = set()
    for line in ALLOWLIST.read_text().split("\n"):
        name = line.split("#", 1)[0].strip()
        if name:
            out.add(name)
    return out


def main() -> int:
    allow = load_allowlist()
    offenders = []
    checked = 0
    continued = 0
    for path in sorted(list(CI.glob("*.sh")) + list(CI.glob("*.py"))):
        if path.name == pathlib.Path(__file__).name:
            continue
        text = path.read_text(errors="replace")
        verdicts = verdict_statements(text)
        if not verdicts:
            continue
        checked += 1
        if path.name in allow:
            continue
        carried = [w for w in verdicts if any(QUANTITY.search(l) for l in w)]
        continued += sum(
            1
            for w in carried
            if len(w) > 1 and not QUANTITY.search(w[0])
        )
        if not carried:
            offenders.append((path.name, verdicts[0][0].strip()[:70]))

    if offenders:
        print(
            f"GATE FAIL: gate-verdict-quantity: {len(offenders)} of {checked} "
            f"gate(s) announce OK without a measured quantity"
        )
        for name, line in offenders:
            print(f"    {name}")
            print(f"      {line}")
        print("  Print what you counted, so the verdict cannot be reached")
        print("  without running the scan. Genuinely uncountable subjects go in")
        print(f"  {ALLOWLIST.name} with a reason.")
        return 1

    print(
        f"[ok] gate-verdict-quantity: {checked} gate(s) with a verdict, "
        f"{len(allow)} allowlisted, {continued} carrying the quantity on a "
        f"continuation line, 0 without a quantity"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
