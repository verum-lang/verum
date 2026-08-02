#!/usr/bin/env python3
"""NOMINAL-DEFID-1 S0 (T0690): measured census of every NAME-KEYED
identity surface in the toolchain — the baseline the DefId migration
retires stage by stage, and the ratchet that keeps new ones out.

A "name-keyed surface" is a table or probe where a STRING decides which
function/type a reference binds to: bare-name registries with
first-wins/last-wins races, `(name, arity)` composite keys, ranked
suffix probes (`ends_with(".leaf")`), per-phase re-derivations of the
same mapping. Each is a measured defect factory (T0458's 99 duplicate
typenames, the ParseError twin-capture, T0448's arity flip, the whole
T0277 id→name carry machinery).

Usage:
    python3 scripts/ci/census_name_keyed_surfaces.py            # report
    python3 scripts/ci/census_name_keyed_surfaces.py --check    # ratchet

--check compares category totals against the RATCHET table below and
fails when a category GREW (a new name-keyed surface appeared without
updating the migration plan). Shrinking is the goal; equal is fine.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]

# category -> (description, [(crate-relative glob, regex)], stage that retires it)
SURFACES: dict[str, tuple[str, list[tuple[str, str]], str]] = {
    "fn-registry-writes": (
        "string-keyed function registrations (ctx.functions inserts)",
        [("crates/verum_vbc/src", r"register_function(_authoritative)?\(")],
        "S3",
    ),
    "fn-registry-lookups": (
        "string-keyed function lookups",
        [("crates/verum_vbc/src", r"lookup_function(_scoped|_in_scope|_with_arity|_qualified)?\(")],
        "S3",
    ),
    "suffix-probes": (
        "ranked suffix probes — `ends_with(\".leaf\")` style resolution",
        [
            ("crates/verum_vbc/src", r"ends_with\(&?(format!\(\"\.\{|\"\.)"),
            ("crates/verum_compiler/src", r"ends_with\(&?(format!\(\"\.\{|\"\.)"),
            ("crates/verum_types/src", r"ends_with\(&?(format!\(\"\.\{|\"\.)"),
        ],
        "S5",
    ),
    "arity-composite-keys": (
        "`name#arity` composite identity keys",
        [("crates/verum_vbc/src", r"format!\(\"\{\}#\{\}\"")],
        "S3",
    ),
    "type-name-keys": (
        "type identity through name maps (type_name_to_id / type_defs / type_aliases writes)",
        [
            ("crates/verum_vbc/src", r"type_name_to_id\s*\.\s*insert"),
            ("crates/verum_vbc/src", r"type_aliases\s*\.\s*insert"),
            ("crates/verum_types/src", r"type_defs\s*\.\s*insert|define_type\("),
        ],
        "S2",
    ),
    "loader-name-indexes": (
        "archive-loader name indexes (qualified_to_module / leaf fanout / by-name archive maps)",
        [("crates/verum_compiler/src", r"(qualified_to_module|leaf_to_qualified|archive_func_by_name|archive_id_to_name|external_id_to_name)")],
        "S4",
    ),
    "runtime-byname-resolution": (
        "runtime by-name resolution (band names, find_function_by_name, dispatch name probes)",
        [("crates/verum_vbc/src/interpreter", r"(find_function_by_name|external_function_names|resolve_function_by_name)")],
        "S5",
    ),
    "id-name-carries": (
        "id→name carry side-channels (exist only because ids are unstable across phases)",
        [
            ("crates/verum_vbc/src", r"resolved_name_by_id"),
            ("crates/verum_compiler/src", r"resolved_name_by_id"),
        ],
        "S4",
    ),
}

# Ratchet baselines: measured 2026-08-02 (git 1a21fdbfd), pinned EXACTLY. A category
# growing past its baseline fails --check. Update DOWNWARD as stages land.
RATCHET: dict[str, int] = {
    "fn-registry-writes": 108,
    "fn-registry-lookups": 223,
    "suffix-probes": 48,
    "arity-composite-keys": 11,
    "type-name-keys": 237,
    "loader-name-indexes": 20,
    "runtime-byname-resolution": 24,
    "id-name-carries": 8,
}


def count(root: str, pattern: str) -> list[str]:
    out = subprocess.run(
        ["grep", "-rn", "-E", pattern, str(REPO / root), "--include=*.rs"],
        capture_output=True,
        text=True,
    ).stdout
    hits = []
    for line in out.splitlines():
        # Skip tests and comment-only lines.
        path = line.split(":", 1)[0]
        if "/tests/" in path or path.endswith("_test.rs"):
            continue
        body = line.split(":", 2)[-1].lstrip()
        if body.startswith("//"):
            continue
        hits.append(line)
    return hits


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--list", metavar="CATEGORY", default=None)
    args = ap.parse_args()

    totals: dict[str, int] = {}
    listing: dict[str, list[str]] = {}
    for cat, (_desc, probes, _stage) in SURFACES.items():
        hits: list[str] = []
        for root, pat in probes:
            hits.extend(count(root, pat))
        totals[cat] = len(hits)
        listing[cat] = hits

    if args.list:
        for h in listing.get(args.list, []):
            print(h)
        return 0

    width = max(len(c) for c in SURFACES)
    print(f"{'category':<{width}}  count  ratchet  retires  description")
    failed = []
    for cat, (desc, _p, stage) in SURFACES.items():
        n = totals[cat]
        r = RATCHET[cat]
        mark = ""
        if args.check and n > r:
            mark = "  << GREW"
            failed.append((cat, n, r))
        print(f"{cat:<{width}}  {n:>5}  {r:>7}  {stage:>7}  {desc}{mark}")
    if args.check and failed:
        print(
            f"\ncensus-name-keyed: {len(failed)} categor(ies) grew past the "
            f"ratchet — a NEW name-keyed identity surface appeared. Either "
            f"retire it (preferred) or update the migration plan in "
            f"docs/architecture/nominal-identity.md AND the ratchet here, "
            f"in the same commit, with the justification."
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
