#!/usr/bin/env python3
"""Gate: enumerate free-function (name, arity) collisions across core/.

A collision is two `public fn` declarations that share a name AND an
arity but live in different modules. Bare-name resolution has to CHOOSE
between them, and the choice is not visible at the call site — that is
the root of the bare-name-collision class (T0220 and kin), not a
cosmetic duplication.

The gate is a RATCHET: it fails when the count rises above the frozen
baseline, and it fails when the count drops without the baseline being
lowered. A silently improving number is how a gate stops measuring —
lower the baseline in the same commit that earns it.

Usage:
    check_barename_collisions.py            # enumerate, human-readable
    check_barename_collisions.py --check    # ratchet, exit 1 on drift
    check_barename_collisions.py --scope sqlite   # only the sqlite/native
                                                  # boundary (T0538)
"""

from __future__ import annotations

import argparse
import collections
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
CORE = ROOT / "core"
SQLITE_NATIVE = "database/sqlite/native"

# Frozen counts, measured 2026-08-11. Lower them in the commit that earns
# it; never raise them without a recorded reason.
BASELINE_ALL = 614
BASELINE_SQLITE = 84
# Same populations under the (name, arity, first-param type) key — the
# REUSE question. Duplicated WORK, not merely a shared verb.
BASELINE_ALL_TYPED = 297
BASELINE_SQLITE_TYPED = 15

# `public fn name(args)` at column 0 — the free-function surface. Methods
# live inside `implement` blocks and are indented, so column-0 anchoring
# is what separates the two without parsing.
DECL = re.compile(r"^public fn (\w+)\s*\(([^)]*)\)")

# The two questions this script answers are NOT the same, and conflating
# them overstates the reuse problem threefold:
#
#   (name, arity)         — a RESOLUTION question. Bare-name resolution
#                           has to choose, so this is the collision count.
#   (name, arity, T0)     — a REUSE question. Same name over a DIFFERENT
#                           first-parameter type is a generic verb
#                           (`mode_name(&BeginMode)` vs
#                           `mode_name(&SecureMode)`), not duplicated work.
#
# Measured 2026-08-11: 614 by the first key, 297 by the second.


def arity(params: str) -> int:
    return len([p for p in params.split(",") if p.strip()])


def collect(typed: bool = False) -> dict[tuple, set[str]]:
    """(name, arity[, first-param type]) -> modules declaring it."""
    found: dict[tuple, set[str]] = collections.defaultdict(set)
    for path in CORE.rglob("*.vr"):
        rel = path.relative_to(CORE).as_posix()
        try:
            text = path.read_text(encoding="utf-8", errors="ignore")
        except OSError:
            continue
        for line in text.splitlines():
            m = DECL.match(line)
            if not m:
                continue
            params = [a.strip() for a in m.group(2).split(",") if a.strip()]
            key: tuple
            if typed:
                first = params[0].split(":")[-1].strip() if params else "()"
                key = (m.group(1), len(params), first)
            else:
                key = (m.group(1), len(params))
            found[key].add(rel)
    return found


def collisions(found, scope: str) -> dict[tuple[str, int], set[str]]:
    out = {}
    for key, modules in found.items():
        if len(modules) < 2:
            continue
        if scope == "sqlite":
            # Only the boundary this task is about: declared BOTH inside
            # sqlite/native and outside it.
            inside = any(m.startswith(SQLITE_NATIVE) for m in modules)
            outside = any(not m.startswith(SQLITE_NATIVE) for m in modules)
            if not (inside and outside):
                continue
        out[key] = modules
    return out


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true", help="ratchet mode")
    ap.add_argument("--scope", choices=("all", "sqlite"), default="all")
    ap.add_argument(
        "--typed",
        action="store_true",
        help="key on (name, arity, first-param type): duplicated WORK, not a shared verb",
    )
    args = ap.parse_args()

    found = collect(typed=args.typed)
    coll = collisions(found, args.scope)
    if args.typed:
        baseline = BASELINE_SQLITE_TYPED if args.scope == "sqlite" else BASELINE_ALL_TYPED
    else:
        baseline = BASELINE_SQLITE if args.scope == "sqlite" else BASELINE_ALL
    count = len(coll)

    if not args.check:
        for key, modules in sorted(coll.items()):
            print("/".join(str(k) for k in key))
            for m in sorted(modules):
                print(f"    {m}")
        print(f"\n{count} colliding (name, arity) pairs [{args.scope}]")
        return 0

    if count > baseline:
        print(
            f"REGRESSION: {count} (name, arity) collisions [{args.scope}], "
            f"baseline {baseline}.",
            file=sys.stderr,
        )
        for key, modules in sorted(coll.items())[:10]:
            label = "/".join(str(k) for k in key)
            print(f"  {label}: {', '.join(sorted(modules))}", file=sys.stderr)
        return 1

    if count < baseline:
        print(
            f"BASELINE STALE: {count} collisions [{args.scope}], baseline "
            f"{baseline}. Lower it in this commit — a gate whose baseline "
            f"drifts above reality stops measuring.",
            file=sys.stderr,
        )
        return 1

    print(f"OK: {count} collisions [{args.scope}], at baseline.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
