#!/usr/bin/env python3
"""Ratchet on `@arch_module` attestation across `core/`.

Every module in the standard library is supposed to declare what it is —
its foundation, its stratum, its lifecycle — through an `@arch_module(...)`
attribute.  A module without one is a module nobody has placed.

WHY A RATCHET RATHER THAN A GATE.  The backlog is real and old: T0712
recorded 160 of 2480 modules missing.  Re-measured 2026-08-15 it is 239 of
2561 — so of the roughly eighty files added in between, roughly
seventy-nine arrived unattested.  A gate demanding zero would be red on
the day it landed and would stay red, which is the same as no gate.  What
must stop first is the GROWTH; the backlog can then be drained a
directory at a time (`term` 68, `sys` 44, `math` 33 lead it).

WHY BY LIST RATHER THAN BY COUNT.  A count-only ratchet is satisfied by
deleting a file, and tells a reader nothing about which module slipped
through.  The list names them, so the diff of the list IS the review: a
line removed is a module attested, a line added is a decision to ship one
without attestation, taken in the open.

There is already a narrower pin — `pin_math_cogs_have_arch_module` in
`crates/verum_kernel/tests/k_arch_v_alignment.rs` — which covers
`core/math/*.vr` only.  This does the same job for the whole tree, and
does not replace it: that one demands zero for math and should keep doing
so as math's 33 are drained.

Usage:
    check_arch_attestation.py                  # gate; exit 1 on a new one
    check_arch_attestation.py --list           # print every missing module
    check_arch_attestation.py --write-baseline # re-record the known set
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
CORE = REPO / "core"
KNOWN = REPO / "scripts" / "ci" / "arch_attestation_known.txt"

ATTESTED = re.compile(r"@arch_module\s*\(")


def missing_modules() -> list[str]:
    """core/ modules carrying no `@arch_module(...)`, repo-relative, sorted."""
    out: list[str] = []
    for path in sorted(CORE.rglob("*.vr")):
        if not ATTESTED.search(path.read_text(errors="replace")):
            out.append(str(path.relative_to(REPO)))
    return out


def read_known() -> set[str]:
    if not KNOWN.exists():
        return set()
    return {
        line.strip()
        for line in KNOWN.read_text().splitlines()
        if line.strip() and not line.startswith("#")
    }


def main() -> int:
    missing = missing_modules()
    total = sum(1 for _ in CORE.rglob("*.vr"))

    if "--list" in sys.argv:
        for m in missing:
            print(m)
        print(f"\n{len(missing)} of {total} core/ modules carry no @arch_module")
        return 0

    if "--write-baseline" in sys.argv:
        header = KNOWN.read_text().split("\n")
        preamble = [ln for ln in header if ln.startswith("#")] if KNOWN.exists() else []
        KNOWN.write_text("\n".join(preamble + missing) + "\n")
        print(f"[ok] baseline rewritten: {len(missing)} module(s)")
        return 0

    known = read_known()
    now = set(missing)
    new = sorted(now - known)
    fixed = sorted(known - now)

    if fixed:
        print(f"[ok] {len(fixed)} module(s) newly attested — drop them from the list:")
        for f in fixed[:20]:
            print(f"    {f}")
        if len(fixed) > 20:
            print(f"    … and {len(fixed) - 20} more")
        print("    scripts/ci/check_arch_attestation.py --write-baseline")

    if new:
        print(
            f"\n[fail] {len(new)} module(s) added to core/ without "
            f"@arch_module attestation:"
        )
        for n in new:
            print(f"    {n}")
        print(
            "\nAn unattested module is one whose foundation, stratum and "
            "lifecycle nobody stated.\nAdd the attribute, or — if shipping it "
            "unattested is deliberate — say so in the\ncommit and re-record "
            "the baseline."
        )
        return 1

    print(
        f"[ok] attestation ratchet holds: {len(missing)} of {total} module(s) "
        f"missing, none new"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
