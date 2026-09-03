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
    for path in sorted(list(CI.glob("*.sh")) + list(CI.glob("*.py"))):
        if path.name == pathlib.Path(__file__).name:
            continue
        text = path.read_text(errors="replace")
        verdicts = [
            l
            for l in text.split("\n")
            if VERDICT.search(l) and not l.strip().startswith(("#", "//"))
        ]
        if not verdicts:
            continue
        checked += 1
        if path.name in allow:
            continue
        if not any(QUANTITY.search(l) for l in verdicts):
            offenders.append((path.name, verdicts[0].strip()[:70]))

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
        f"{len(allow)} allowlisted, 0 without a quantity"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
