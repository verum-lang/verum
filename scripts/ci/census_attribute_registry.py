#!/usr/bin/env python3
"""Census: attributes the corpus USES against attributes the registry KNOWS.

`verum_types::attr::standard` declares each attribute with the targets it
may be applied to, and the compile path does not consult it. Its own
phase says so: "the attribute registry has always declared exactly that
… while nothing in the compile path called `AttributeRegistry::validate`
— the declaration was documentation, not enforcement."

Consequence, measured 2026-08-30: `@totally_made_up` on a function
compiles clean, and so does `@cap(net)` (T0943, T0929). But the registry
cannot simply be switched on, because it is the SMALLER of the two sets:

    registry                      82 attributes
    corpus (core/, vcs/, tests)  295 distinct
    used but unregistered        242, over 11 494 occurrences

The unregistered ones are not typos — `@extern` (2 114), `@cfg` (1 679),
`@intrinsic` (795) are load-bearing. Enforcement has to follow
registration, in that order, which is what this census measures.

Targets are inferred from USE — the declaration that follows the
attribute — rather than guessed, so the output can seed registry entries.

    scripts/ci/census_attribute_registry.py [--unknown-only] [--limit N]
"""

from __future__ import annotations

import argparse
import collections
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
REGISTRY = REPO / "crates" / "verum_types" / "src" / "attr" / "standard.rs"
CORPUS = ("core", "vcs/specs", "core-tests")

DECL_KINDS = [
    (re.compile(r"^\s*(public\s+)?fn\s"), "Function"),
    (re.compile(r"^\s*(public\s+)?type\s"), "Type"),
    (re.compile(r"^\s*(public\s+)?module\s"), "Module"),
    (re.compile(r"^\s*(public\s+)?const\s"), "Const"),
    (re.compile(r"^\s*(public\s+)?static\s"), "Static"),
    (re.compile(r"^\s*implement\s"), "Impl"),
    (re.compile(r"^\s*(public\s+)?context\s"), "Context"),
    (re.compile(r"^\s*(public\s+)?extern\s"), "Extern"),
]


def registered() -> set[str]:
    if not REGISTRY.is_file():
        print(f"registry not found at {REGISTRY}", file=sys.stderr)
        return set()
    return set(
        re.findall(r'AttributeMetadata::new\("([a-z_0-9]+)"', REGISTRY.read_text())
    )


def survey() -> tuple[collections.Counter, dict[str, collections.Counter]]:
    counts: collections.Counter = collections.Counter()
    targets: dict[str, collections.Counter] = collections.defaultdict(
        collections.Counter
    )
    for root in CORPUS:
        base = REPO / root
        if not base.is_dir():
            continue
        for path in base.rglob("*.vr"):
            try:
                lines = path.read_text(errors="replace").splitlines()
            except OSError:
                continue
            for i, line in enumerate(lines):
                m = re.match(r"^\s*@([a-z_][a-z_0-9]*)", line)
                if not m:
                    continue
                name = m.group(1)
                counts[name] += 1
                # Skip further attributes, comments and blank lines to
                # reach the declaration the attribute is attached to.
                for nxt in lines[i + 1 : i + 8]:
                    if re.match(r"^\s*(@|//|$)", nxt):
                        continue
                    for rx, kind in DECL_KINDS:
                        if rx.match(nxt):
                            targets[name][kind] += 1
                            break
                    else:
                        targets[name]["Other"] += 1
                    break
    return counts, targets


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--unknown-only", action="store_true")
    ap.add_argument("--limit", type=int, default=30)
    args = ap.parse_args()

    known = registered()
    counts, targets = survey()

    # Positive control: an empty survey would make every verdict below
    # meaningless while looking like a clean result.
    if len(counts) < 20:
        print(
            f"REFUSING TO REPORT: only {len(counts)} attributes found in the "
            "corpus — the scan is broken, not the tree.",
            file=sys.stderr,
        )
        return 2

    unknown = {n: c for n, c in counts.items() if n not in known}
    print(f"registry knows           : {len(known)}")
    print(f"corpus uses (distinct)   : {len(counts)}")
    print(
        f"used but unregistered    : {len(unknown)} "
        f"({sum(unknown.values())} occurrences)\n"
    )

    rows = unknown if args.unknown_only else counts
    print(f"{'attribute':<26}{'uses':>7}  inferred target(s)")
    for name, n in sorted(rows.items(), key=lambda kv: -kv[1])[: args.limit]:
        top = ", ".join(f"{k}:{v}" for k, v in targets[name].most_common(2))
        mark = " " if name in known else "*"
        print(f"{mark}@{name:<24}{n:>7}  {top}")
    if not args.unknown_only:
        print("\n* = not in the registry")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
