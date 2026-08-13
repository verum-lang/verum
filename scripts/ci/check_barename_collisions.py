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
PRELUDE_SOURCE = CORE / "mod.vr"

# Frozen counts, measured 2026-08-11. Lower them in the commit that earns
# it; never raise them without a recorded reason.
BASELINE_ALL = 614
BASELINE_SQLITE = 84
# The PRELUDE scope — the subset a user meets without importing anything.
# This is not a stylistic count: for these names the ambiguity DECIDES which
# implementation runs, and 14 of the 20 prelude math functions currently
# resolve to SQLite's SQL builtins and answer `Relaxed` instead of a number
# (measured 2026-08-11 through `mount core.prelude.*`).
BASELINE_PRELUDE = 26
# Same populations under the (name, arity, first-param type) key — the
# REUSE question. Duplicated WORK, not merely a shared verb.
BASELINE_ALL_TYPED = 297
# `--kind types`: two `public type` declarations sharing a SIMPLE NAME in
# different modules. Frozen at the count measured when the scope was added.
#
# Not a tidiness metric. One of these pairs cost a whole public function:
# `core/term/render/diff.vr` mounts `core.term.style.Modifier` EXPLICITLY, and
# `Modifier.BOLD` still resolved against the SQLite date grammar's unrelated
# `Modifier` sum — so `write_modifiers` shipped as a panic stub until the
# SQLite type was renamed `DateModifier`. Every remaining pair is the same
# shape, waiting for a resolution order to shift under it.
BASELINE_TYPES = 104
BASELINE_SQLITE_TYPED = 15

# `public fn name(args)` at column 0 — the free-function surface. Methods
# live inside `implement` blocks and are indented, so column-0 anchoring
# is what separates the two without parsing.
DECL = re.compile(r"^public fn (\w+)\s*\(([^)]*)\)")
TYPE_DECL = re.compile(r"^public\s+type\s+(\w+)")

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
#
# THE TYPED KEY IS NOT PROOF OF DUPLICATION EITHER — checked on its own
# largest cluster, 2026-08-12. `is_known_only/1/Int64` appears in five
# sqlite modules and reads like five copies of one function. The bodies
# differ: each ANDs against a different flag mask
# (`f_nosavepoint()|f_invert()|f_ignorenoop()|f_fknoaction()` in
# changeset_apply_policy, `well_known_mask()` in open_v2_flags_api, and so
# on). Same verb, same carrier type, different DOMAIN — and the domain is
# distinguished by the MODULE, which no key over (name, arity, type) can
# see.
#
# So treat 297 as an upper bound on duplicated work, and READ THE BODIES
# before merging anything this script pairs up. Kin: register entry C9,
# where three math modules share 18 names and turned out to be three
# deliberate CONTRACTS (high-precision / zero-libc / correctly-rounded),
# not three copies.


def arity(params: str) -> int:
    return len([p for p in params.split(",") if p.strip()])


def prelude_named_exports() -> set[str]:
    """Names the prelude re-exports EXPLICITLY, from `core/mod.vr`.

    LIMITATION, stated rather than hidden: the prelude also carries ~13
    `public mount super.<mod>.*;` globs, whose members are NOT counted here —
    enumerating them means resolving each module's public surface, which this
    script deliberately does not do. So the prelude count is a LOWER BOUND on
    the ambiguous names a user can hit without importing anything.
    """
    try:
        src = PRELUDE_SOURCE.read_text(encoding="utf-8", errors="ignore")
    except OSError:
        return set()
    names: set[str] = set()
    # `public mount super.math.{sin, cos, ...};` — braced lists, possibly
    # spanning lines and carrying `//` comments between entries.
    for block in re.finditer(r"public mount super\.[\w.]+\.\{([^}]*)\}", src, re.S):
        body = re.sub(r"//[^\n]*", "", block.group(1))
        for name in re.split(r"[,\s]+", body):
            if re.fullmatch(r"[A-Za-z_]\w*", name):
                names.add(name)
    # `public mount super.io.print;` — single-name form.
    for one in re.finditer(r"public mount super\.[\w.]+\.(\w+);", src):
        names.add(one.group(1))
    return names


def collect_types() -> dict[tuple, set[str]]:
    """(type name,) -> modules declaring it as a `public type`."""
    found: dict[tuple, set[str]] = collections.defaultdict(set)
    for path in CORE.rglob("*.vr"):
        rel = path.relative_to(CORE).as_posix()
        try:
            text = path.read_text(encoding="utf-8", errors="ignore")
        except OSError:
            continue
        for line in text.splitlines():
            m = TYPE_DECL.match(line)
            if m:
                found[(m.group(1),)].add(rel)
    return found


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
    prelude = prelude_named_exports() if scope == "prelude" else set()
    for key, modules in found.items():
        if len(modules) < 2:
            continue
        if scope == "prelude":
            # The name is reachable with no import at all, and more than one
            # module answers to it at the same arity — so which body runs is
            # decided by resolution order, invisibly, at every call site.
            if key[0] not in prelude:
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
    ap.add_argument("--scope", choices=("all", "sqlite", "prelude"), default="all")
    ap.add_argument(
        "--kind",
        choices=("functions", "types"),
        default="functions",
        help="types: `public type` simple-name collisions across modules",
    )
    ap.add_argument(
        "--typed",
        action="store_true",
        help="key on (name, arity, first-param type): duplicated WORK, not a shared verb",
    )
    args = ap.parse_args()

    if args.kind == "types":
        found = collect_types()
        coll = {k: v for k, v in found.items() if len(v) > 1}
        for (name,), mods in sorted(coll.items()):
            print(f"{name:28s} {', '.join(sorted(mods))}")
        print(f"\n{len(coll)} colliding public type names [types]")
        if args.check and len(coll) != BASELINE_TYPES:
            direction = "rose above" if len(coll) > BASELINE_TYPES else "dropped below"
            print(
                f"RATCHET: public-type collisions {direction} the baseline "
                f"({len(coll)} vs {BASELINE_TYPES}). A rise adds a name whose "
                f"resolution is invisible at the use site; a drop must lower the "
                f"baseline in the same commit that earns it.",
                file=sys.stderr,
            )
            return 1
        return 0

    found = collect(typed=args.typed)
    coll = collisions(found, args.scope)
    if args.typed:
        if args.scope == "prelude":
            print(
                "--typed is not defined for the prelude scope: the question there is "
                "WHICH BODY RUNS for a bare name, and a differing first-parameter type "
                "does not make that unambiguous.",
                file=sys.stderr,
            )
            return 2
        baseline = BASELINE_SQLITE_TYPED if args.scope == "sqlite" else BASELINE_ALL_TYPED
    elif args.scope == "prelude":
        baseline = BASELINE_PRELUDE
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
